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

impl Binding {
    /// Whether a test function follows the binding. A detached citation
    /// binds no test.
    pub fn is_attached(&self) -> bool {
        self.test.function.is_some()
    }

    /// The use case this binding establishes coverage for: a well-formed
    /// citation attached to a test function. `None` makes it an orphan.
    pub fn use_case(&self) -> Option<&UseCaseRef> {
        self.cites.use_case().filter(|_| self.is_attached())
    }
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
        let root = repo_root
            .canonicalize()
            .map_err(|source| SpecsError::Read {
                path: repo_root.to_path_buf(),
                source,
            })?;
        let mut bindings = Vec::new();
        for file in files {
            let language = match Language::of(&file) {
                Some(language) => language,
                None => continue,
            };
            let path = repo_root.join(&file);
            // Every symlink, of the file and of its directories, resolved:
            // only files inside the repository are its tests.
            let real = match path.canonicalize() {
                Ok(real) => real,
                // Listed by the index but deleted from the working tree, or
                // a dangling link.
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => return Err(SpecsError::Read { path, source }),
            };
            if !real.starts_with(&root) {
                tracing::warn!(
                    event = "specs_test_file_outside_repository",
                    file = %file,
                    target = %real.display(),
                    "skipping a test file that resolves outside the repository"
                );
                continue;
            }
            let text = match std::fs::read(&real) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
                Err(source) => return Err(SpecsError::Read { path: real, source }),
            };
            let found = match language {
                Language::Rust => rust_bindings(&text),
                Language::Python => python_bindings(&text),
            };
            bindings.extend(found.into_iter().map(|(line, function, cites)| Binding {
                test: TestLocation {
                    file: PathBuf::from(&file),
                    line,
                    function: function.map(|f| f.name),
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

/// The function a binding is attached to: its name and the 1-based line
/// its item starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Attached {
    name: String,
    line: usize,
}

/// A binding found in one file: its 1-based line, the function it is
/// attached to and what it cites.
type Found = (usize, Option<Attached>, Citation);

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
fn following_rust_fn(
    lines: &[&str],
    from: usize,
    code: &impl Fn(usize) -> bool,
) -> Option<Attached> {
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
        return rust_fn_name(trimmed).map(|name| Attached {
            name,
            line: index + 1,
        });
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

/// A Python token, as far as bindings need one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PyToken {
    /// An identifier, keyword or number.
    Name(String),
    /// A string literal: its value for a plain, raw or unicode literal;
    /// `None` for a bytes literal or an f-string, which are no string
    /// constants.
    Str(Option<String>),
    Op(char),
}

/// One logical line: the column of its first token and its tokens, each
/// with its 1-based line. Comments, line continuations and the line breaks
/// inside brackets and strings are already resolved.
struct LogicalLine {
    indent: usize,
    tokens: Vec<(usize, PyToken)>,
}

impl LogicalLine {
    fn kinds(&self) -> Vec<&PyToken> {
        self.tokens.iter().map(|(_, t)| t).collect()
    }
}

fn is_name_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_name_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Splits Python source into logical lines of tokens.
fn python_logical_lines(text: &str) -> Vec<LogicalLine> {
    let chars: Vec<char> = text.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let mut lines = Vec::new();
    let mut current = LogicalLine {
        indent: 0,
        tokens: Vec::new(),
    };
    let mut line = 1;
    let mut line_start = 0;
    let mut depth = 0usize;
    let mut i = 0;
    while let Some(c) = at(i) {
        let column = i - line_start;
        let token_line = line;
        let push = |current: &mut LogicalLine, token: PyToken| {
            if current.tokens.is_empty() {
                current.indent = column;
            }
            current.tokens.push((token_line, token));
        };
        match c {
            '\n' => {
                line += 1;
                i += 1;
                line_start = i;
                if depth == 0 && !current.tokens.is_empty() {
                    lines.push(std::mem::replace(
                        &mut current,
                        LogicalLine {
                            indent: 0,
                            tokens: Vec::new(),
                        },
                    ));
                }
            }
            '#' => {
                while at(i).is_some_and(|c| c != '\n') {
                    i += 1;
                }
            }
            '\\' if at(i + 1) == Some('\n') => {
                line += 1;
                i += 2;
                line_start = i;
            }
            '\'' | '"' => {
                let (value, end, newlines) = python_string(&chars, i, "");
                push(&mut current, PyToken::Str(value));
                i = end;
                line += newlines;
            }
            c if is_name_start(c) => {
                let mut end = i;
                while at(end).is_some_and(is_name_char) {
                    end += 1;
                }
                let word: String = chars[i..end].iter().collect();
                let prefix = word.to_ascii_lowercase();
                let is_prefix = matches!(
                    prefix.as_str(),
                    "r" | "u" | "b" | "f" | "br" | "rb" | "fr" | "rf"
                );
                if is_prefix && matches!(at(end), Some('\'' | '"')) {
                    let (value, after, newlines) = python_string(&chars, end, &prefix);
                    push(&mut current, PyToken::Str(value));
                    i = after;
                    line += newlines;
                } else {
                    push(&mut current, PyToken::Name(word));
                    i = end;
                }
            }
            c if c.is_ascii_digit() => {
                let mut end = i;
                while at(end).is_some_and(|c| is_name_char(c) || c == '.') {
                    end += 1;
                }
                push(&mut current, PyToken::Name(chars[i..end].iter().collect()));
                i = end;
            }
            c if c.is_whitespace() => i += 1,
            c => {
                match c {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth = depth.saturating_sub(1),
                    _ => {}
                }
                push(&mut current, PyToken::Op(c));
                i += 1;
            }
        }
    }
    if !current.tokens.is_empty() {
        lines.push(current);
    }
    lines
}

/// Reads the string literal whose opening quote is at `start`: its value
/// (for a plain, raw or unicode literal), the index after it and the line
/// breaks it spans. An unterminated one-line string ends at its line.
fn python_string(chars: &[char], start: usize, prefix: &str) -> (Option<String>, usize, usize) {
    let quote = chars[start];
    let triple = chars.get(start + 1) == Some(&quote) && chars.get(start + 2) == Some(&quote);
    let raw = prefix.contains('r');
    let constant = !prefix.contains('b') && !prefix.contains('f');
    let mut value = String::new();
    let mut newlines = 0;
    let mut i = start + if triple { 3 } else { 1 };
    while let Some(&c) = chars.get(i) {
        if c == quote
            && (!triple || (chars.get(i + 1) == Some(&quote) && chars.get(i + 2) == Some(&quote)))
        {
            let end = i + if triple { 3 } else { 1 };
            return (constant.then_some(value), end, newlines);
        }
        if c == '\n' {
            if !triple {
                return (None, i, newlines);
            }
            newlines += 1;
            value.push(c);
            i += 1;
            continue;
        }
        if c == '\\'
            && let Some(&escaped) = chars.get(i + 1)
        {
            if escaped == '\n' {
                newlines += 1;
            }
            if raw {
                value.push(c);
                value.push(escaped);
            } else {
                match escaped {
                    '\n' => {}
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    other => value.push(other),
                }
            }
            i += 2;
            continue;
        }
        value.push(c);
        i += 1;
    }
    (None, i, newlines)
}

fn python_bindings(text: &str) -> Vec<Found> {
    let lines = python_logical_lines(text);
    let constants = python_constants(&lines);
    let mut found = Vec::new();
    for (index, logical) in lines.iter().enumerate() {
        let Some(args) = spec_decorator_args(logical) else {
            continue;
        };
        let Some(values) = python_arguments(&args, &constants) else {
            continue;
        };
        let [spec, use_case] = values.as_slice() else {
            continue;
        };
        let line = logical.tokens.first().map_or(1, |(line, _)| *line);
        let function = following_python_def(lines.get(index + 1..).unwrap_or_default());
        found.push((line, function, Citation::read(spec, use_case)));
    }
    found
}

/// Module-level `NAME = "text"` (or `NAME: <annotation> = "text"`)
/// assignments.
fn python_constants(lines: &[LogicalLine]) -> HashMap<String, String> {
    let mut constants = HashMap::new();
    for logical in lines.iter().filter(|l| l.indent == 0) {
        let tokens = logical.kinds();
        let Some(assign) = tokens.iter().position(|t| **t == PyToken::Op('=')) else {
            continue;
        };
        let target = match tokens.as_slice() {
            [PyToken::Name(name), PyToken::Op('=' | ':'), ..] => name,
            _ => continue,
        };
        if let [PyToken::Str(Some(value))] = &tokens[assign + 1..] {
            constants.insert(target.clone(), value.clone());
        }
    }
    constants
}

/// The argument tokens of `@pytest.mark.spec(…)` when `logical` is that
/// decorator and nothing else.
fn spec_decorator_args(logical: &LogicalLine) -> Option<Vec<&PyToken>> {
    let tokens = logical.kinds();
    let name = |t: &PyToken, n: &str| matches!(t, PyToken::Name(w) if w == n);
    let [at, pytest, dot, mark, dot2, spec, open, inner @ .., close] = tokens.as_slice() else {
        return None;
    };
    let decorator = **at == PyToken::Op('@')
        && name(pytest, "pytest")
        && **dot == PyToken::Op('.')
        && name(mark, "mark")
        && **dot2 == PyToken::Op('.')
        && name(spec, "spec")
        && **open == PyToken::Op('(')
        && **close == PyToken::Op(')');
    decorator.then(|| inner.to_vec())
}

/// The values of a `spec(…)` call whose every argument is one string
/// literal or one module-level string constant; `None` otherwise.
fn python_arguments(args: &[&PyToken], constants: &HashMap<String, String>) -> Option<Vec<String>> {
    let mut parts: Vec<&[&PyToken]> = args.split(|t| **t == PyToken::Op(',')).collect();
    // A trailing comma leaves one empty part.
    if parts.len() > 1 && parts.last().is_some_and(|p| p.is_empty()) {
        parts.pop();
    }
    parts
        .into_iter()
        .map(|part| match part {
            [PyToken::Str(Some(value))] => Some(value.clone()),
            [PyToken::Name(name)] => constants.get(name).cloned(),
            _ => None,
        })
        .collect()
}

/// The name of the function the decorators end at.
fn following_python_def(after: &[LogicalLine]) -> Option<Attached> {
    let item = after
        .iter()
        .find(|l| l.tokens.first().map(|(_, t)| t) != Some(&PyToken::Op('@')))?;
    let name = match item.kinds().as_slice() {
        [PyToken::Name(def), PyToken::Name(name), ..] if def == "def" => name.clone(),
        [
            PyToken::Name(a),
            PyToken::Name(def),
            PyToken::Name(name),
            ..,
        ] if a == "async" && def == "def" => name.clone(),
        _ => return None,
    };
    let line = item.tokens.first().map(|(line, _)| *line)?;
    Some(Attached { name, line })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each binding's line, the line of the function it is attached to,
    /// and whether it cites a well-formed use case.
    fn shape(found: &[Found]) -> Vec<(usize, Option<usize>, bool)> {
        found
            .iter()
            .map(|(line, function, cites)| {
                (
                    *line,
                    function.as_ref().map(|f| f.line),
                    matches!(cites, Citation::UseCase(_)),
                )
            })
            .collect()
    }

    #[test]
    fn a_rust_binding_attaches_to_the_function_after_its_attributes() {
        let text = "// spec: acme/spec/feature/x/y UC-XY1\n#[test]\n#[cfg_attr(\n    unix,\n    ignore\n)]\n\
                    // more\npub async fn realizes_it() {}\n";
        assert_eq!(shape(&rust_bindings(text)), vec![(1, Some(8), true)]);
    }

    #[test]
    fn comment_shaped_lines_inside_strings_and_block_comments_are_not_bindings() {
        let text = "const A: &str = \"\n// spec: acme/spec/feature/x/y UC-XY1\n\";\n\
                    const B: &str = r#\"\n// spec: acme/spec/feature/x/y UC-XY2\n\"#;\n\
                    /* outer /* inner */\n// spec: acme/spec/feature/x/y UC-XY3\n*/\n\
                    fn f<'a>(c: char) -> bool { c == '\"' }\n// spec: acme/spec/feature/x/y UC-XY4\nfn g() {}\n";
        assert_eq!(shape(&rust_bindings(text)), vec![(11, Some(12), true)]);
    }

    #[test]
    fn doc_comments_are_not_bindings() {
        assert!(rust_bindings("/// spec: acme/spec/feature/x/y UC-XY1\nfn f() {}\n").is_empty());
        assert!(rust_bindings("//! spec: acme/spec/feature/x/y UC-XY1\n").is_empty());
    }

    #[test]
    fn a_binding_of_another_shape_is_malformed_and_one_with_no_function_is_detached() {
        let found = rust_bindings("// spec: docs/x.md UC-XY1\nstruct S;\n// spec: only-one\n");
        assert_eq!(shape(&found), vec![(1, None, false), (3, None, false)]);
    }

    #[test]
    fn a_python_binding_reads_literals_and_module_constants() {
        let text = "SPEC = \"acme/spec/feature/x/y\"\n\n@pytest.mark.spec(SPEC, 'UC-XY1')\n\
                    @pytest.mark.slow\nasync def test_it():\n    pass\n\n\
                    @pytest.mark.spec(\n    \"acme/spec/feature/x/y\",\n    \"UC-XY2\",\n)\ndef test_other():\n    pass\n";
        assert_eq!(
            shape(&python_bindings(text)),
            vec![(3, Some(5), true), (8, Some(12), true)]
        );
    }

    #[test]
    fn comments_inside_decorator_arguments_do_not_hide_a_binding() {
        let text = "@pytest.mark.spec(  # the use case\n    'acme/spec/feature/x/y',  # spec\n    \
                    'UC-XY1',  # ) not the end\n)\ndef test_it():\n    pass\n";
        assert_eq!(shape(&python_bindings(text)), vec![(1, Some(5), true)]);
    }

    #[test]
    fn triple_quotes_inside_a_comment_do_not_open_a_string() {
        let text = "# a comment with \"\"\" in it\n@pytest.mark.spec('acme/spec/feature/x/y', 'UC-XY1')\n\
                    def test_it():\n    pass\n";
        assert_eq!(shape(&python_bindings(text)), vec![(2, Some(3), true)]);
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

    #[test]
    fn a_python_binding_followed_by_no_function_is_detached() {
        let found = python_bindings(
            "@pytest.mark.spec('acme/spec/feature/x/y', 'UC-XY1')\nclass TestIt:\n    pass\n",
        );
        assert_eq!(shape(&found), vec![(1, None, true)]);
    }
}
