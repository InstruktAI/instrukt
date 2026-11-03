//! instrukt's section of one hook file, as text: where it is, whether it is
//! current, and the edits that install or remove it. Pure; the files are
//! read and written by `_hooks`.

use super::_hook::Hook;

/// The line opening instrukt's section in a hook file.
pub const SECTION_START: &str = "# >>> instrukt >>>";
/// The line closing instrukt's section in a hook file.
pub const SECTION_END: &str = "# <<< instrukt <<<";

/// The interpreter line of a hook file instrukt creates.
const INTERPRETER_LINE: &str = "#!/bin/sh\n";

/// Interpreters that run instrukt's section as written.
const SHELLS: [&str; 7] = ["sh", "bash", "dash", "zsh", "ksh", "ash", "mksh"];

/// The call a section makes, and the line a hook manager is given to keep
/// instrukt's behaviour: the hook run, never `exec`ing a command, never
/// failing the hook.
pub fn hook_run_line(hook: Hook) -> String {
    format!("{} || true", hook_run_call(hook))
}

fn hook_run_call(hook: Hook) -> String {
    format!("instrukt hooks run {} \"$@\"", hook.name())
}

/// The whole section for `hook`, markers included, ending in a newline.
///
/// Git passes pre-push its ref list on stdin, which every section of the
/// hook may need. The pre-push section copies it to a temporary file, feeds
/// that file to the hook run, then restores the hook's stdin from the file
/// with a redirection-only `exec` and removes the file: the open descriptor
/// keeps the list readable for the sections after it, and nothing stays in
/// the temporary directory. No step can end the hook or change git's
/// result, under `set -e` or `set -u` alike: a temporary file that cannot
/// be made skips the section, a failed copy or removal is ignored, and the
/// `exec` runs only while its file is readable — a redirection error on
/// `exec` would end a POSIX shell whatever guards it.
pub(crate) fn section(hook: Hook) -> String {
    let body = match hook {
        Hook::PrePush => [
            "if instrukt_refs=\"$(mktemp \"${TMPDIR:-/tmp}/instrukt-pre-push.XXXXXX\" 2>/dev/null)\"; then"
                .to_owned(),
            "  cat >\"$instrukt_refs\" 2>/dev/null || true".to_owned(),
            format!("  {} <\"$instrukt_refs\" || true", hook_run_call(hook)),
            "  if [ -r \"$instrukt_refs\" ]; then exec 0<\"$instrukt_refs\"; fi".to_owned(),
            "  rm -f \"$instrukt_refs\" 2>/dev/null || true".to_owned(),
            "fi".to_owned(),
        ]
        .join("\n"),
        _ => hook_run_line(hook),
    };
    format!("{SECTION_START}\n{body}\n{SECTION_END}\n")
}

/// A hook file instrukt creates: an interpreter line and the section.
pub(crate) fn new_file(hook: Hook) -> String {
    format!("{INTERPRETER_LINE}{}", section(hook))
}

/// The section markers of a file do not delimit exactly one section.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SectionError {
    #[error("line {line} opens an instrukt section that is never closed")]
    Unterminated { line: usize },
    #[error("line {line} opens a second instrukt section")]
    Duplicate { line: usize },
    #[error("line {line} closes an instrukt section that was never opened")]
    StrayEnd { line: usize },
}

/// Where a section stands in a file: the byte range from the start of its
/// opening marker line through the end of its closing marker line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
}

/// Whether a file's section is the one instrukt would write now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionState {
    Current,
    Outdated,
    Absent,
}

/// What installing the section does to a file's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Installed {
    /// The section is current; the file stays as it is.
    Unchanged,
    /// The section's content was replaced.
    Replaced(String),
    /// The section was appended after every existing line.
    Appended(String),
    /// The section was inserted before the first line running `exec`
    /// (1-based), so it runs before the hook is replaced.
    BeforeExec { text: String, line: usize },
}

/// The state of `hook`'s section in `text`.
pub(crate) fn state(text: &str, hook: Hook) -> Result<SectionState, SectionError> {
    Ok(match locate(text)? {
        None => SectionState::Absent,
        Some(span) if span.holds(text, &section(hook)) => SectionState::Current,
        Some(_) => SectionState::Outdated,
    })
}

impl Span {
    /// Whether the section ends the file without a final newline: the file
    /// had none before the section was appended, and the newline that ends
    /// its last line is the one just before the section.
    fn unterminated_at_end(self, text: &str) -> bool {
        self.end == text.len() && !text[self.start..self.end].ends_with('\n')
    }

    /// Whether the section is `wanted` (in its form without a final newline
    /// when it ends a file that has none).
    fn holds(self, text: &str, wanted: &str) -> bool {
        let found = &text[self.start..self.end];
        found == wanted
            || (self.unterminated_at_end(text) && wanted.strip_suffix('\n') == Some(found))
    }
}

/// Installs `hook`'s section into `text`, touching no byte outside it. A
/// file without a final newline keeps having none: the section is appended
/// after a newline ending the file's last line, without one of its own, so
/// removing it restores every byte.
pub(crate) fn install(text: &str, hook: Hook) -> Result<Installed, SectionError> {
    let wanted = section(hook);
    if let Some(span) = locate(text)? {
        if span.holds(text, &wanted) {
            return Ok(Installed::Unchanged);
        }
        let replacement = if span.unterminated_at_end(text) {
            wanted.trim_end_matches('\n')
        } else {
            wanted.as_str()
        };
        let mut out = String::with_capacity(text.len() + wanted.len());
        out.push_str(&text[..span.start]);
        out.push_str(replacement);
        out.push_str(&text[span.end..]);
        return Ok(Installed::Replaced(out));
    }
    if let Some((line, offset)) = first_exec_line(text) {
        let mut out = String::with_capacity(text.len() + wanted.len());
        out.push_str(&text[..offset]);
        out.push_str(&wanted);
        out.push_str(&text[offset..]);
        return Ok(Installed::BeforeExec { text: out, line });
    }
    let mut out = String::with_capacity(text.len() + wanted.len() + 1);
    out.push_str(text);
    if !text.is_empty() && !text.ends_with('\n') {
        out.push('\n');
        out.push_str(wanted.trim_end_matches('\n'));
    } else {
        out.push_str(&wanted);
    }
    Ok(Installed::Appended(out))
}

/// `text` without its section; `None` when it holds none. A section ending
/// a file without a final newline takes the newline before it along, so
/// the file ends as it did before the section was appended.
pub(crate) fn remove(text: &str) -> Result<Option<String>, SectionError> {
    Ok(locate(text)?.map(|span| {
        let mut start = span.start;
        if span.unterminated_at_end(text) && start > 0 && text[..start].ends_with('\n') {
            start -= 1;
        }
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..start]);
        out.push_str(&text[span.end..]);
        out
    }))
}

/// The interpreter a file names on its first line when it is not a shell
/// that runs the section; `None` for a shell or a file without one (git
/// runs such a file with the shell).
pub(crate) fn foreign_interpreter(text: &str) -> Option<String> {
    let first = text.lines().next()?;
    let rest = first.strip_prefix("#!")?;
    let mut words = rest.split_whitespace();
    let mut program = basename(words.next()?);
    if program == "env" {
        program = basename(words.find(|word| !word.starts_with('-') && !word.contains('='))?);
    }
    if SHELLS.contains(&program) {
        None
    } else {
        Some(program.to_owned())
    }
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Each line with its 1-based number and byte offset; lines keep their
/// newline.
fn lines(text: &str) -> impl Iterator<Item = (usize, usize, &str)> {
    text.split_inclusive('\n')
        .scan(0usize, |offset, line| {
            let start = *offset;
            *offset += line.len();
            Some((start, line))
        })
        .enumerate()
        .map(|(index, (start, line))| (index + 1, start, line))
}

fn is_marker(line: &str, marker: &str) -> bool {
    line.trim() == marker
}

fn locate(text: &str) -> Result<Option<Span>, SectionError> {
    let mut found: Option<Span> = None;
    let mut open: Option<(usize, usize)> = None;
    for (number, offset, line) in lines(text) {
        if is_marker(line, SECTION_START) {
            if open.is_some() {
                return Err(SectionError::Unterminated {
                    line: open.map_or(number, |(line, _)| line),
                });
            }
            if found.is_some() {
                return Err(SectionError::Duplicate { line: number });
            }
            open = Some((number, offset));
        } else if is_marker(line, SECTION_END) {
            match open.take() {
                Some((_, start)) => {
                    found = Some(Span {
                        start,
                        end: offset + line.len(),
                    });
                }
                None => return Err(SectionError::StrayEnd { line: number }),
            }
        }
    }
    match open {
        Some((line, _)) => Err(SectionError::Unterminated { line }),
        None => Ok(found),
    }
}

/// The first line that runs `exec`, with its number and byte offset.
fn first_exec_line(text: &str) -> Option<(usize, usize)> {
    lines(text)
        .find(|(_, _, line)| execs_a_command(line))
        .map(|(number, offset, _)| (number, offset))
}

/// Whether `line` runs a command through `exec`, replacing the shell, in
/// any of its shell forms: at the start of the line or after `;`, `&&`,
/// `||`, `&`, `|` or a keyword (`then`, `do`, `{` …), after assignment
/// prefixes (`FOO=bar exec cmd`), after `exec`'s own options. An `exec`
/// holding only redirections (`exec 1>&2`, `exec 0<"$file"`) keeps the
/// shell, so the lines after it still run.
fn execs_a_command(line: &str) -> bool {
    shell_words(line)
        .split(|word| matches!(word, Word::Separator))
        .any(|command| {
            let words: Vec<&str> = command
                .iter()
                .filter_map(|word| match word {
                    Word::Text(text) => Some(text.as_str()),
                    Word::Separator => None,
                })
                .collect();
            let mut rest = words
                .iter()
                .copied()
                .skip_while(|word| is_assignment(word) || KEYWORDS.contains(word));
            rest.next() == Some("exec") && runs_command(rest)
        })
}

/// Words after which a command starts.
const KEYWORDS: [&str; 9] = [
    "then", "else", "elif", "do", "if", "while", "until", "{", "!",
];

/// Whether the words after `exec` name a command, past its options and
/// redirections.
fn runs_command<'a>(mut words: impl Iterator<Item = &'a str>) -> bool {
    let mut options = true;
    while let Some(word) = words.next() {
        if options && word == "--" {
            options = false;
            continue;
        }
        if options && word.starts_with('-') && word.len() > 1 {
            if word == "-a" {
                words.next();
            }
            continue;
        }
        match redirection(word) {
            Some(Target::Next) => {
                words.next();
            }
            Some(Target::Attached) => {}
            None => return true,
        }
    }
    false
}

/// `NAME=value`, a variable assignment prefixing a command.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        name.chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// One shell word, quotes removed, or a command separator.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Word {
    Text(String),
    Separator,
}

/// The words of one line as the shell splits them: quotes group, `#` at a
/// word's start begins a comment, and `;`, `&`, `|`, `(`, `)` separate
/// commands unless they continue a redirection (`2>&1`, `>|`).
fn shell_words(line: &str) -> Vec<Word> {
    let mut words = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = line.chars().peekable();
    let flush = |words: &mut Vec<Word>, current: &mut String, in_word: &mut bool| {
        if *in_word {
            words.push(Word::Text(std::mem::take(current)));
            *in_word = false;
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for quoted in chars.by_ref() {
                    if quoted == '\'' {
                        break;
                    }
                    current.push(quoted);
                }
            }
            '"' => {
                in_word = true;
                while let Some(quoted) = chars.next() {
                    match quoted {
                        '"' => break,
                        '\\' => current.extend(chars.next()),
                        other => current.push(other),
                    }
                }
            }
            '\\' => {
                in_word = true;
                current.extend(chars.next());
            }
            '#' if !in_word => break,
            '&' | '|' if current.ends_with(['<', '>']) => current.push(c),
            ';' | '&' | '|' | '(' | ')' => {
                flush(&mut words, &mut current, &mut in_word);
                words.push(Word::Separator);
            }
            c if c.is_whitespace() => flush(&mut words, &mut current, &mut in_word),
            other => {
                in_word = true;
                current.push(other);
            }
        }
    }
    flush(&mut words, &mut current, &mut in_word);
    words
}

/// Where a redirection word's target is.
enum Target {
    /// In the word itself (`2>&1`, `0<"$f"`).
    Attached,
    /// In the next word (`> file`).
    Next,
}

/// The target of `word` when it is a redirection; `None` when it is not
/// one. A descriptor number or `{name}` may precede the operator.
fn redirection(word: &str) -> Option<Target> {
    let operator = match word.strip_prefix('{') {
        Some(rest) => rest.split_once('}').map_or(word, |(_, after)| after),
        None => word.trim_start_matches(|c: char| c.is_ascii_digit()),
    };
    ["<<-", "<<", ">>", "<&", ">&", "<>", ">|", "<", ">"]
        .iter()
        .find_map(|op| operator.strip_prefix(op))
        .map(|target| {
            if target.is_empty() {
                Target::Next
            } else {
                Target::Attached
            }
        })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    const HOOK: Hook = Hook::PostCommit;

    fn installed_text(edit: Installed) -> String {
        match edit {
            Installed::Replaced(text)
            | Installed::Appended(text)
            | Installed::BeforeExec { text, .. } => text,
            Installed::Unchanged => String::new(),
        }
    }

    #[test]
    fn an_appended_section_is_current_and_reinstalling_changes_nothing() {
        let own = "#!/bin/sh\nmake lint\n";
        let text = installed_text(install(own, HOOK).unwrap());
        assert_eq!(state(&text, HOOK).unwrap(), SectionState::Current);
        assert_eq!(install(&text, HOOK).unwrap(), Installed::Unchanged);
        assert_eq!(remove(&text).unwrap().as_deref(), Some(own));
    }

    #[test]
    fn a_section_goes_before_the_first_line_running_exec() {
        let own = "#!/bin/sh\n# exec is mentioned here\nexecutable=1\nexec npx lint-staged\n";
        let Installed::BeforeExec { text, line } = install(own, HOOK).unwrap() else {
            panic!("expected an insertion before exec");
        };
        assert_eq!(line, 4);
        assert_eq!(remove(&text).unwrap().as_deref(), Some(own));
        let exec_at = text.find("exec npx").unwrap();
        let section_at = text.find(SECTION_START).unwrap();
        assert!(section_at < exec_at);
    }

    #[test]
    fn an_outdated_section_is_replaced_in_place() {
        let stale = format!("#!/bin/sh\na\n{SECTION_START}\nold call\n{SECTION_END}\nb\n");
        assert_eq!(state(&stale, HOOK).unwrap(), SectionState::Outdated);
        let text = installed_text(install(&stale, HOOK).unwrap());
        assert_eq!(state(&text, HOOK).unwrap(), SectionState::Current);
        assert_eq!(remove(&text).unwrap(), remove(&stale).unwrap());
    }

    #[test]
    fn an_opening_marker_never_closed_is_refused() {
        let unterminated = format!("{SECTION_START}\nx\n");
        assert_eq!(
            locate(&unterminated),
            Err(SectionError::Unterminated { line: 1 })
        );
    }

    #[test]
    fn a_closing_marker_never_opened_is_refused() {
        let stray = format!("x\n{SECTION_END}\n");
        assert_eq!(locate(&stray), Err(SectionError::StrayEnd { line: 2 }));
    }

    #[test]
    fn a_second_section_is_refused() {
        let twice = format!("{}{}", section(HOOK), section(HOOK));
        assert_eq!(locate(&twice), Err(SectionError::Duplicate { line: 4 }));
    }

    #[test]
    fn a_plain_exec_of_a_command_counts() {
        assert!(execs_a_command("exec npx lint-staged"));
    }

    #[test]
    fn an_exec_whose_redirections_precede_a_command_counts() {
        assert!(execs_a_command("exec 2>&1 other-tool"));
    }

    #[test]
    fn a_redirection_target_in_its_own_word_is_skipped_before_the_command() {
        assert!(execs_a_command("exec > log other-tool"));
    }

    #[test]
    fn an_assignment_prefixed_exec_counts() {
        assert!(execs_a_command("FOO=bar exec sh -c 'x'"));
    }

    #[test]
    fn a_quoted_assignment_value_stays_one_word() {
        assert!(execs_a_command("HOOK=\"a b\" exec \"$tool\""));
    }

    #[test]
    fn an_exec_after_a_command_separator_counts() {
        assert!(execs_a_command("[ -x tool ] && exec tool"));
    }

    #[test]
    fn an_exec_after_a_keyword_counts() {
        assert!(execs_a_command("if true; then exec tool; fi"));
    }

    #[test]
    fn an_exec_past_its_own_options_counts() {
        assert!(execs_a_command("exec -a name tool"));
    }

    #[test]
    fn a_redirection_only_exec_does_not_count() {
        assert!(!execs_a_command("exec 1>&2"));
    }

    #[test]
    fn redirections_with_separate_targets_alone_do_not_count() {
        assert!(!execs_a_command("exec > log 2>&1"));
    }

    #[test]
    fn a_bare_exec_does_not_count() {
        assert!(!execs_a_command("exec"));
    }

    #[test]
    fn an_exec_inside_quotes_does_not_count() {
        assert!(!execs_a_command("echo 'exec tool'"));
    }

    #[test]
    fn an_exec_in_a_comment_does_not_count() {
        assert!(!execs_a_command("echo done # exec tool"));
    }

    #[test]
    fn a_word_beginning_with_exec_is_not_exec() {
        assert!(!execs_a_command("executable=1"));
    }

    #[test]
    fn the_pre_push_sections_own_redirection_is_not_taken_for_an_exec() {
        assert_eq!(first_exec_line(&section(Hook::PrePush)), None);
    }

    #[test]
    fn a_pre_push_section_goes_before_a_command_exec_and_after_a_redirection() {
        let own = "#!/bin/sh\nexec 1>&2\nexec other-tool pre-push \"$@\"\n";
        let Installed::BeforeExec { text, line } = install(own, Hook::PrePush).unwrap() else {
            panic!("expected an insertion before the command exec");
        };
        assert_eq!(line, 3);
        assert_eq!(state(&text, Hook::PrePush).unwrap(), SectionState::Current);
        assert_eq!(install(&text, Hook::PrePush).unwrap(), Installed::Unchanged);
        assert_eq!(remove(&text).unwrap().as_deref(), Some(own));
    }

    #[test]
    fn a_shell_named_through_env_runs_the_section() {
        assert_eq!(foreign_interpreter("#!/usr/bin/env bash\n"), None);
    }

    #[test]
    fn a_file_without_an_interpreter_line_runs_with_the_shell() {
        assert_eq!(foreign_interpreter("echo hook\n"), None);
    }

    #[test]
    fn a_foreign_interpreter_named_through_env_with_options_is_detected() {
        assert!(foreign_interpreter("#!/usr/bin/env -S python3 -u\n").is_some());
    }

    #[test]
    fn a_foreign_interpreter_named_directly_is_detected() {
        assert!(foreign_interpreter("#!/usr/bin/node\n").is_some());
    }

    proptest! {
        #[test]
        fn install_then_remove_restores_every_byte(
            own in prop::collection::vec("[ -~]{0,20}", 0..8),
            final_newline in any::<bool>(),
        ) {
            let mut text: String = own.iter().map(|line| format!("{line}\n")).collect();
            if !final_newline {
                text.pop();
            }
            prop_assume!(locate(&text).is_ok_and(|span| span.is_none()));
            let installed = installed_text(install(&text, HOOK).unwrap());
            prop_assert_eq!(state(&installed, HOOK).unwrap(), SectionState::Current);
            prop_assert_eq!(remove(&installed).unwrap(), Some(text));
        }
    }
}
