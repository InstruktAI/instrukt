//! The binding set: every `spec` binding a project's tests declare.
//!
//! The forms are `use-case-coverage`'s "The bindings": a Rust line comment
//! `// spec: <spec_id> <UC-id>` attached to the function that follows it,
//! and a Python decorator `@pytest.mark.spec(<spec_id>, <UC-id>)` whose
//! arguments are string literals or module-level string constants.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

use super::SpecsError;
use crate::git;
use crate::ids::{SnippetId, UseCaseId, UseCaseRef};

/// Where a binding is declared: the test file (relative to the repository
/// root), the 1-based line of the binding and the function it is attached
/// to, when one follows.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TestLocation {
    pub file: PathBuf,
    pub line: usize,
    pub function: Option<String>,
}

impl fmt::Display for TestLocation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.function {
            Some(function) => write!(f, "{}::{function}", self.file.display()),
            None => write!(f, "{}:{}", self.file.display(), self.line),
        }
    }
}

/// What a binding cites.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Citation {
    /// A snippet identity and a well-formed use-case id.
    UseCase(UseCaseRef),
    /// Anything else, as written: names no use case.
    Malformed(String),
}

impl Citation {
    fn read(spec: &str, use_case: &str) -> Self {
        match (SnippetId::parse(spec), UseCaseId::parse(use_case)) {
            (Ok(spec), Ok(use_case)) => Self::UseCase(UseCaseRef { spec, use_case }),
            _ => Self::Malformed(format!("{spec} {use_case}").trim().to_owned()),
        }
    }

    /// The use case cited, when the citation is well formed.
    pub fn use_case(&self) -> Option<&UseCaseRef> {
        match self {
            Self::UseCase(reference) => Some(reference),
            Self::Malformed(_) => None,
        }
    }
}

impl fmt::Display for Citation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UseCase(reference) => reference.fmt(f),
            Self::Malformed(text) => f.write_str(text),
        }
    }
}

/// One test's declaration that it realizes one use case.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Binding {
    pub test: TestLocation,
    pub cites: Citation,
}

/// Every binding in the project's tests, read from its working tree: every
/// `.rs` file and every Python test file (`test_*.py`, `*_test.py`) git
/// does not ignore.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingSet {
    bindings: Vec<Binding>,
}

impl BindingSet {
    pub fn read(repo_root: &Path) -> Result<Self, SpecsError> {
        let listing = git::run_bytes(
            repo_root,
            [
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
            ],
        )?;
        let files: BTreeSet<String> = listing
            .split(|b| *b == 0)
            .filter(|name| !name.is_empty())
            .map(|name| String::from_utf8_lossy(name).into_owned())
            .collect();
        let mut bindings = Vec::new();
        for file in files {
            let language = match Language::of(&file) {
                Some(language) => language,
                None => continue,
            };
            let path = repo_root.join(&file);
            let text = match std::fs::read(&path) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                // Listed by the index but deleted from the working tree.
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => return Err(SpecsError::Read { path, source }),
            };
            let found = match language {
                Language::Rust => rust_bindings(&text),
                Language::Python => python_bindings(&text),
            };
            bindings.extend(found.into_iter().map(|(line, function, cites)| Binding {
                test: TestLocation {
                    file: PathBuf::from(&file),
                    line,
                    function,
                },
                cites,
            }));
        }
        Ok(Self { bindings })
    }

    /// Every binding, by file then line.
    pub fn bindings(&self) -> &[Binding] {
        &self.bindings
    }
}

#[derive(Debug, Clone, Copy)]
enum Language {
    Rust,
    Python,
}

impl Language {
    fn of(file: &str) -> Option<Self> {
        let name = file.rsplit('/').next().unwrap_or(file);
        if name.ends_with(".rs") {
            Some(Self::Rust)
        } else if name.ends_with(".py") && (name.starts_with("test_") || name.ends_with("_test.py"))
        {
            Some(Self::Python)
        } else {
            None
        }
    }
}

/// A binding found in one file: its 1-based line, the function it is
/// attached to and what it cites.
type Found = (usize, Option<String>, Citation);

// ---- Rust -------------------------------------------------------------

/// Where a Rust lexer stands at the start of a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RustState {
    Code,
    Str,
    RawStr(usize),
    BlockComment(usize),
}

/// The lexer state at the start of each line of `text`, so comment-shaped
/// lines inside strings and block comments are never read as comments.
fn rust_line_states(text: &str) -> Vec<RustState> {
    let chars: Vec<char> = text.chars().collect();
    let mut states = vec![RustState::Code];
    let mut state = RustState::Code;
    let mut i = 0;
    let at = |i: usize| chars.get(i).copied();
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            states.push(state);
            i += 1;
            continue;
        }
        match state {
            RustState::Code => match c {
                '/' if at(i + 1) == Some('/') => {
                    while i < chars.len() && chars[i] != '\n' {
                        i += 1;
                    }
                    continue;
                }
                '/' if at(i + 1) == Some('*') => {
                    state = RustState::BlockComment(1);
                    i += 2;
                    continue;
                }
                '"' => state = RustState::Str,
                'r' if starts_raw_prefix(&chars, i) => {
                    let hashes = chars[i + 1..].iter().take_while(|c| **c == '#').count();
                    if at(i + 1 + hashes) == Some('"') {
                        state = RustState::RawStr(hashes);
                        i += hashes + 2;
                        continue;
                    }
                }
                '\'' => {
                    // A char literal ('x', '\n', '\u{..}') or a lifetime.
                    if at(i + 1) == Some('\\') {
                        let mut j = i + 2;
                        while j < chars.len() && chars[j] != '\'' && chars[j] != '\n' {
                            j += 1;
                        }
                        i = j + 1;
                        continue;
                    }
                    if at(i + 2) == Some('\'') {
                        i += 3;
                        continue;
                    }
                }
                _ => {}
            },
            RustState::Str => match c {
                '\\' => {
                    // Skip the escaped character, unless it is the line end.
                    if at(i + 1) != Some('\n') {
                        i += 1;
                    }
                }
                '"' => state = RustState::Code,
                _ => {}
            },
            RustState::RawStr(hashes) => {
                if c == '"' && (1..=hashes).all(|k| at(i + k) == Some('#')) {
                    state = RustState::Code;
                    i += hashes + 1;
                    continue;
                }
            }
            RustState::BlockComment(depth) => {
                if c == '/' && at(i + 1) == Some('*') {
                    state = RustState::BlockComment(depth + 1);
                    i += 2;
                    continue;
                }
                if c == '*' && at(i + 1) == Some('/') {
                    state = if depth <= 1 {
                        RustState::Code
                    } else {
                        RustState::BlockComment(depth - 1)
                    };
                    i += 2;
                    continue;
                }
            }
        }
        i += 1;
    }
    states
}

/// Whether the `r` at `i` starts a token (`r"…"`, or `br"…"`).
fn starts_raw_prefix(chars: &[char], i: usize) -> bool {
    let ident = |c: char| c.is_alphanumeric() || c == '_';
    match i.checked_sub(1).map(|p| (p, chars[p])) {
        None => true,
        Some((p, 'b')) => p.checked_sub(1).is_none_or(|q| !ident(chars[q])),
        Some((_, prev)) => !ident(prev),
    }
}

fn rust_bindings(text: &str) -> Vec<Found> {
    let lines: Vec<&str> = text.lines().collect();
    let states = rust_line_states(text);
    let code = |index: usize| states.get(index) == Some(&RustState::Code);
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !code(index) {
            continue;
        }
        let Some(rest) = line.trim_start().strip_prefix("//") else {
            continue;
        };
        if rest.starts_with('/') || rest.starts_with('!') {
            continue;
        }
        let Some(declared) = rest.trim_start().strip_prefix("spec:") else {
            continue;
        };
        let cites = citation(declared);
        let function = following_rust_fn(&lines, index + 1, &code);
        found.push((index + 1, function, cites));
    }
    found
}

/// A binding's two words; any other shape names no use case.
fn citation(declared: &str) -> Citation {
    let words: Vec<&str> = declared.split_whitespace().collect();
    match words.as_slice() {
        [spec, use_case] => Citation::read(spec, use_case),
        _ => Citation::Malformed(declared.trim().to_owned()),
    }
}

/// The name of the function whose item follows line `from`, past blank
/// lines, comments and attributes.
fn following_rust_fn(lines: &[&str], from: usize, code: &impl Fn(usize) -> bool) -> Option<String> {
    let mut attribute_depth: i64 = 0;
    for (index, line) in lines.iter().enumerate().skip(from) {
        let trimmed = line.trim();
        if attribute_depth > 0 || trimmed.starts_with("#[") {
            attribute_depth += bracket_balance(trimmed);
            continue;
        }
        if trimmed.is_empty() || !code(index) || trimmed.starts_with("//") {
            continue;
        }
        return rust_fn_name(trimmed);
    }
    None
}

fn bracket_balance(text: &str) -> i64 {
    text.chars().fold(0, |depth, c| match c {
        '[' => depth + 1,
        ']' => depth - 1,
        _ => depth,
    })
}

/// `fn <name>` after optional qualifiers (`pub`, `pub(crate)`, `async`,
/// `const`, `unsafe`, `extern "C"`).
fn rust_fn_name(item: &str) -> Option<String> {
    let mut rest = item;
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("fn ") {
            let name: String = after
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            return (!name.is_empty()).then_some(name);
        }
        let word_len = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
        let word = &rest[..word_len];
        let qualifier = word == "async"
            || word == "const"
            || word == "unsafe"
            || word == "extern"
            || word.starts_with("pub")
            || word.starts_with('"');
        if !qualifier || word_len == 0 {
            return None;
        }
        rest = &rest[word_len..];
    }
}

// ---- Python -----------------------------------------------------------

fn python_bindings(text: &str) -> Vec<Found> {
    let lines: Vec<&str> = text.lines().collect();
    let in_code = python_code_lines(&lines);
    let constants = python_constants(&lines, &in_code);
    let mut found = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let trimmed = lines[index].trim_start();
        let start = index;
        index += 1;
        if !in_code[start] {
            continue;
        }
        let Some(after) = trimmed.strip_prefix("@pytest.mark.spec(") else {
            continue;
        };
        // The argument list may span lines; it ends at the closing paren.
        let mut args = after.to_owned();
        while !balanced_call(&args) && index < lines.len() {
            args.push('\n');
            args.push_str(lines[index]);
            index += 1;
        }
        let Some(values) = python_arguments(&args, &constants) else {
            continue;
        };
        let [spec, use_case] = values.as_slice() else {
            continue;
        };
        let function = following_python_def(&lines, index);
        found.push((start + 1, function, Citation::read(spec, use_case)));
    }
    found
}

/// Whether each line starts outside a triple-quoted string.
fn python_code_lines(lines: &[&str]) -> Vec<bool> {
    let mut open: Option<&str> = None;
    lines
        .iter()
        .map(|line| {
            let starts_in_code = open.is_none();
            let mut rest = *line;
            loop {
                let next = match open {
                    Some(quote) => rest.find(quote).map(|at| (at, quote)),
                    None => ["\"\"\"", "'''"]
                        .iter()
                        .filter_map(|q| rest.find(q).map(|at| (at, *q)))
                        .min_by_key(|(at, _)| *at),
                };
                let Some((at, quote)) = next else { break };
                open = match open {
                    Some(_) => None,
                    None => Some(quote),
                };
                rest = &rest[at + 3..];
            }
            starts_in_code
        })
        .collect()
}

/// Module-level `NAME = "text"` (or `NAME: str = "text"`) assignments.
fn python_constants(lines: &[&str], in_code: &[bool]) -> HashMap<String, String> {
    let mut constants = HashMap::new();
    for (line, code) in lines.iter().zip(in_code) {
        if !code || line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some((target, value)) = line.split_once('=') else {
            continue;
        };
        let name = target.split(':').next().unwrap_or_default().trim();
        let is_name = name
            .chars()
            .next()
            .is_some_and(|c| c.is_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_alphanumeric() || c == '_');
        if !is_name || value.starts_with('=') {
            continue;
        }
        if let Some((text, rest)) = python_string(value.trim())
            && (rest.trim().is_empty() || rest.trim_start().starts_with('#'))
        {
            constants.insert(name.to_owned(), text);
        }
    }
    constants
}

/// Whether the text after `spec(` has reached its closing paren.
fn balanced_call(args: &str) -> bool {
    split_call(args).is_some()
}

/// The arguments before the call's closing paren, split at top-level
/// commas; `None` while the paren is not reached.
fn split_call(args: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut chars = args.chars();
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            current.push(c);
            if c == '\\' {
                if let Some(escaped) = chars.next() {
                    current.push(escaped);
                }
            } else if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                current.push(c);
            }
            '(' | '[' | '{' => {
                depth += 1;
                current.push(c);
            }
            ')' if depth == 0 => {
                parts.push(current);
                return Some(parts);
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                current.push(c);
            }
            ',' if depth == 0 => parts.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    None
}

/// The values of a `spec(...)` call whose every argument is a string
/// literal or a module-level string constant; `None` otherwise.
fn python_arguments(args: &str, constants: &HashMap<String, String>) -> Option<Vec<String>> {
    let mut parts = split_call(args)?;
    // A trailing comma leaves one empty part.
    if parts.last().is_some_and(|p| p.trim().is_empty()) {
        parts.pop();
    }
    parts
        .iter()
        .map(|part| {
            let part = part.trim();
            match python_string(part) {
                Some((text, rest)) if rest.trim().is_empty() => Some(text),
                Some(_) => None,
                None => constants.get(part).cloned(),
            }
        })
        .collect()
}

/// A one-line Python string literal at the start of `text` (optionally
/// `r`/`u`-prefixed): its value and the text after it.
fn python_string(text: &str) -> Option<(String, &str)> {
    let (raw, body) = match text.chars().next()? {
        'r' | 'R' => (true, &text[1..]),
        'u' | 'U' => (false, &text[1..]),
        _ => (false, text),
    };
    let quote = body.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let mut value = String::new();
    let mut chars = body[1..].char_indices();
    while let Some((at, c)) = chars.next() {
        if c == quote {
            return Some((value, &body[1 + at + 1..]));
        }
        if c == '\\' && !raw {
            let (_, escaped) = chars.next()?;
            value.push(match escaped {
                'n' => '\n',
                't' => '\t',
                other => other,
            });
        } else {
            value.push(c);
        }
    }
    None
}

/// The name of the function the decorators end at, from line `from`.
fn following_python_def(lines: &[&str], from: usize) -> Option<String> {
    for line in lines.iter().skip(from) {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('@') {
            continue;
        }
        let after = trimmed
            .strip_prefix("async def ")
            .or_else(|| trimmed.strip_prefix("def "))?;
        let name: String = after
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        return (!name.is_empty()).then_some(name);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn functions(found: &[Found]) -> Vec<Option<String>> {
        found.iter().map(|(_, f, _)| f.clone()).collect()
    }

    fn reference(spec: &str, use_case: &str) -> Citation {
        Citation::UseCase(UseCaseRef {
            spec: SnippetId::parse(spec).unwrap(),
            use_case: UseCaseId::parse(use_case).unwrap(),
        })
    }

    #[test]
    fn a_rust_binding_attaches_to_the_function_after_its_attributes() {
        let text = "// spec: acme/spec/feature/x/y UC-XY1\n#[test]\n#[cfg_attr(\n    unix,\n    ignore\n)]\n\
                    // more\npub async fn realizes_it() {}\n";
        let found = rust_bindings(text);
        assert_eq!(
            found,
            vec![(
                1,
                Some("realizes_it".to_owned()),
                reference("acme/spec/feature/x/y", "UC-XY1")
            )]
        );
    }

    #[test]
    fn comment_shaped_lines_inside_strings_and_block_comments_are_not_bindings() {
        let text = "const A: &str = \"\n// spec: acme/spec/feature/x/y UC-XY1\n\";\n\
                    const B: &str = r#\"\n// spec: acme/spec/feature/x/y UC-XY2\n\"#;\n\
                    /* outer /* inner */\n// spec: acme/spec/feature/x/y UC-XY3\n*/\n\
                    fn f<'a>(c: char) -> bool { c == '\"' }\n// spec: acme/spec/feature/x/y UC-XY4\nfn g() {}\n";
        let found = rust_bindings(text);
        assert_eq!(functions(&found), vec![Some("g".to_owned())]);
    }

    #[test]
    fn doc_comments_are_not_bindings() {
        assert!(rust_bindings("/// spec: acme/spec/feature/x/y UC-XY1\nfn f() {}\n").is_empty());
        assert!(rust_bindings("//! spec: acme/spec/feature/x/y UC-XY1\n").is_empty());
    }

    #[test]
    fn a_binding_of_another_shape_is_malformed_and_one_with_no_function_has_none() {
        let found = rust_bindings("// spec: docs/x.md UC-XY1\nstruct S;\n// spec: only-one\n");
        let malformed: Vec<bool> = found
            .iter()
            .map(|(_, _, c)| matches!(c, Citation::Malformed(_)))
            .collect();
        assert_eq!(malformed, vec![true, true]);
        assert_eq!(functions(&found), vec![None, None]);
    }

    #[test]
    fn a_python_binding_reads_literals_and_module_constants() {
        let text = "SPEC = \"acme/spec/feature/x/y\"\n\n@pytest.mark.spec(SPEC, 'UC-XY1')\n\
                    @pytest.mark.slow\nasync def test_it():\n    pass\n\n\
                    @pytest.mark.spec(\n    \"acme/spec/feature/x/y\",\n    \"UC-XY2\",\n)\ndef test_other():\n    pass\n";
        let found = python_bindings(text);
        assert_eq!(
            found,
            vec![
                (
                    3,
                    Some("test_it".to_owned()),
                    reference("acme/spec/feature/x/y", "UC-XY1")
                ),
                (
                    8,
                    Some("test_other".to_owned()),
                    reference("acme/spec/feature/x/y", "UC-XY2")
                ),
            ]
        );
    }

    #[test]
    fn a_python_decorator_with_another_argument_form_is_not_a_binding() {
        let text = "def spec_id():\n    return 'x'\n\n@pytest.mark.spec(spec_id(), \"UC-XY1\")\ndef test_a():\n    pass\n\
                    @pytest.mark.spec(f\"acme/{x}\", \"UC-XY1\")\ndef test_b():\n    pass\n\
                    @pytest.mark.spec(UNKNOWN, \"UC-XY1\")\ndef test_c():\n    pass\n";
        assert!(python_bindings(text).is_empty());
    }

    #[test]
    fn a_decorator_inside_a_docstring_is_not_a_binding() {
        let text = "\"\"\"Example:\n@pytest.mark.spec('acme/spec/feature/x/y', 'UC-XY1')\ndef test_x(): ...\n\"\"\"\n";
        assert!(python_bindings(text).is_empty());
    }
}
