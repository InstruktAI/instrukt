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

/// The one call a section holds, and the line a hook manager is given to
/// keep instrukt's behaviour: the hook run, never `exec`, never failing the
/// hook.
pub fn hook_run_line(hook: Hook) -> String {
    format!("instrukt hooks run {} \"$@\" || true", hook.name())
}

/// The whole section for `hook`, markers included, ending in a newline.
pub(crate) fn section(hook: Hook) -> String {
    format!("{SECTION_START}\n{}\n{SECTION_END}\n", hook_run_line(hook))
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
        .find(|(_, _, line)| {
            line.trim_start()
                .strip_prefix("exec")
                .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
        })
        .map(|(number, offset, _)| (number, offset))
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
