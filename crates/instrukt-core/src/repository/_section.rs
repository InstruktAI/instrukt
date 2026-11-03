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
/// the temporary directory.
pub(crate) fn section(hook: Hook) -> String {
    let body = match hook {
        Hook::PrePush => [
            "if instrukt_refs=\"$(mktemp \"${TMPDIR:-/tmp}/instrukt-pre-push.XXXXXX\")\"; then"
                .to_owned(),
            "  cat >\"$instrukt_refs\" || true".to_owned(),
            format!("  {} <\"$instrukt_refs\" || true", hook_run_call(hook)),
            "  exec 0<\"$instrukt_refs\"".to_owned(),
            "  rm -f \"$instrukt_refs\"".to_owned(),
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
        Some(span) if text[span.start..span.end] == section(hook) => SectionState::Current,
        Some(_) => SectionState::Outdated,
    })
}

/// Installs `hook`'s section into `text`, touching no byte outside it.
pub(crate) fn install(text: &str, hook: Hook) -> Result<Installed, SectionError> {
    let wanted = section(hook);
    if let Some(span) = locate(text)? {
        if text[span.start..span.end] == wanted {
            return Ok(Installed::Unchanged);
        }
        let mut out = String::with_capacity(text.len() + wanted.len());
        out.push_str(&text[..span.start]);
        out.push_str(&wanted);
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
    }
    out.push_str(&wanted);
    Ok(Installed::Appended(out))
}

/// `text` without its section; `None` when it holds none.
pub(crate) fn remove(text: &str) -> Result<Option<String>, SectionError> {
    Ok(locate(text)?.map(|span| {
        let mut out = String::with_capacity(text.len());
        out.push_str(&text[..span.start]);
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

/// Whether `line` is an `exec` that replaces the shell with a command; an
/// `exec` holding only redirections (`exec 1>&2`, `exec 0<"$file"`) keeps
/// the shell, so the lines after it still run.
fn execs_a_command(line: &str) -> bool {
    let Some(rest) = line.trim_start().strip_prefix("exec") else {
        return false;
    };
    if !(rest.is_empty() || rest.starts_with(char::is_whitespace)) {
        return false;
    }
    let mut words = rest.split_whitespace();
    while let Some(word) = words.next() {
        if word.starts_with('#') {
            return false;
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
    fn markers_that_do_not_delimit_one_section_are_refused() {
        let unterminated = format!("{SECTION_START}\nx\n");
        assert_eq!(
            locate(&unterminated),
            Err(SectionError::Unterminated { line: 1 })
        );
        let stray = format!("x\n{SECTION_END}\n");
        assert_eq!(locate(&stray), Err(SectionError::StrayEnd { line: 2 }));
        let twice = format!("{}{}", section(HOOK), section(HOOK));
        assert_eq!(locate(&twice), Err(SectionError::Duplicate { line: 4 }));
    }

    #[test]
    fn only_an_exec_running_a_command_counts_and_a_redirection_does_not() {
        for command in [
            "exec npx lint-staged",
            "  exec \"$@\"",
            "exec 2>&1 other-tool",
            "exec > log other-tool",
        ] {
            assert!(execs_a_command(command), "{command}");
        }
        for kept in [
            "exec 1>&2",
            "exec 0<\"$instrukt_refs\"",
            "exec > log 2>&1",
            "exec {fd}<file",
            "exec",
            "exec # nothing",
            "executable=1",
            "# exec npx",
        ] {
            assert!(!execs_a_command(kept), "{kept}");
        }
        for hook in Hook::ALL {
            assert_eq!(first_exec_line(&section(hook)), None, "{hook}");
        }
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
    fn only_shells_run_the_section() {
        for shell in [
            "#!/bin/sh",
            "#!/usr/bin/env bash",
            "#!/bin/zsh -e",
            "no line",
        ] {
            assert_eq!(foreign_interpreter(shell), None, "{shell}");
        }
        assert_eq!(
            foreign_interpreter("#!/usr/bin/env -S python3 -u\n").as_deref(),
            Some("python3")
        );
        assert_eq!(
            foreign_interpreter("#!/usr/bin/node\n").as_deref(),
            Some("node")
        );
    }

    proptest! {
        #[test]
        fn install_then_remove_restores_every_byte(
            own in prop::collection::vec("[ -~]{0,20}", 0..8),
        ) {
            let text: String = own.iter().map(|line| format!("{line}\n")).collect();
            prop_assume!(locate(&text).is_ok_and(|span| span.is_none()));
            let installed = installed_text(install(&text, HOOK).unwrap());
            prop_assert_eq!(state(&installed, HOOK).unwrap(), SectionState::Current);
            prop_assert_eq!(remove(&installed).unwrap(), Some(text));
        }
    }
}
