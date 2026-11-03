//! The docs reference filter `instrukt-docs`: on checkout (smudge) it
//! expands `@~/.instrukt` to the absolute home path, on commit (clean) it
//! restores the tilde form, so committed docs stay portable while agents
//! read absolute paths.

use std::os::unix::ffi::OsStrExt as _;
use std::path::Path;

use crate::home::{HOME_DIR_NAME, Home};

use super::_error::RepositoryError;
use super::_git;

/// The filter's name in git configuration and `.gitattributes`.
pub const FILTER_NAME: &str = "instrukt-docs";

/// The `.gitattributes` line routing docs through the filter.
pub const FILTER_PATTERN: &str = "docs/**/*.md filter=instrukt-docs";

/// The arguments, after `instrukt`, of the filter's two directions: the CLI
/// routes them to [`apply_docs_filter`].
pub const FILTER_COMMAND: [&str; 2] = ["repo", "docs-filter"];

/// Which way the filter runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterDirection {
    /// Checkout: `@~/.instrukt` becomes the absolute home path.
    Smudge,
    /// Commit: the absolute home path becomes `@~/.instrukt`.
    Clean,
}

impl FilterDirection {
    pub fn name(self) -> &'static str {
        match self {
            FilterDirection::Smudge => "smudge",
            FilterDirection::Clean => "clean",
        }
    }
}

/// `input` filtered in `direction` for `home`.
pub fn apply_docs_filter(input: &[u8], home: &Home, direction: FilterDirection) -> Vec<u8> {
    let portable = format!("@~/{HOME_DIR_NAME}").into_bytes();
    let mut absolute = b"@".to_vec();
    absolute.extend_from_slice(home.root().as_os_str().as_bytes());
    match direction {
        FilterDirection::Smudge => replace(input, &portable, &absolute),
        FilterDirection::Clean => replace(input, &absolute, &portable),
    }
}

/// The git configuration `(key, value)` pairs that define the filter.
pub(crate) fn definition() -> [(String, String); 3] {
    let command = |direction: FilterDirection| {
        let mut words = vec!["instrukt"];
        words.extend(FILTER_COMMAND);
        words.push(direction.name());
        words.join(" ")
    };
    [
        (
            format!("filter.{FILTER_NAME}.smudge"),
            command(FilterDirection::Smudge),
        ),
        (
            format!("filter.{FILTER_NAME}.clean"),
            command(FilterDirection::Clean),
        ),
        (format!("filter.{FILTER_NAME}.required"), "true".to_owned()),
    ]
}

/// How many of the filter's keys the repository's own configuration
/// already holds as instrukt defines them, out of how many.
pub(crate) fn defined_keys(root: &Path) -> Result<(usize, usize), RepositoryError> {
    let definition = definition();
    let mut matching = 0;
    for (key, value) in &definition {
        if _git::local_config(root, key)?.as_deref() == Some(value.as_str()) {
            matching += 1;
        }
    }
    Ok((matching, definition.len()))
}

/// Writes every key of the filter that differs into the repository's own
/// configuration.
pub(crate) fn define(root: &Path) -> Result<(), RepositoryError> {
    for (key, value) in definition() {
        if _git::local_config(root, &key)?.as_deref() != Some(value.as_str()) {
            crate::git::run(root, ["config", "--local", key.as_str(), value.as_str()])?;
        }
    }
    Ok(())
}

fn replace(input: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut rest = input;
    while let Some(at) = find(rest, from) {
        out.extend_from_slice(&rest[..at]);
        out.extend_from_slice(to);
        rest = &rest[at + from.len()..];
    }
    out.extend_from_slice(rest);
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use proptest::prelude::*;

    use super::*;

    fn home() -> Home {
        Home::at_user_home(PathBuf::from("/Users/dev"))
    }

    #[test]
    fn smudge_expands_every_home_reference_and_clean_restores_it() {
        let committed =
            b"Read @~/.instrukt/docs/a.md and @~/.instrukt/docs/b.md; not ~/.instrukt.\n";
        let checked_out = apply_docs_filter(committed, &home(), FilterDirection::Smudge);
        let mut expected = Vec::new();
        expected.extend_from_slice(b"Read @");
        expected.extend_from_slice(home().root().as_os_str().as_bytes());
        expected.extend_from_slice(b"/docs/a.md and @");
        expected.extend_from_slice(home().root().as_os_str().as_bytes());
        expected.extend_from_slice(b"/docs/b.md; not ~/.instrukt.\n");
        assert_eq!(checked_out, expected);
        assert_eq!(
            apply_docs_filter(&checked_out, &home(), FilterDirection::Clean),
            committed
        );
    }

    proptest! {
        #[test]
        fn clean_undoes_smudge(parts in prop::collection::vec(
            prop_oneof![Just("@~/.instrukt".to_owned()), "[a-z ./@~]{0,8}"], 0..10,
        )) {
            let text = parts.concat();
            prop_assume!(find(text.as_bytes(), b"@/Users/dev").is_none());
            let smudged = apply_docs_filter(text.as_bytes(), &home(), FilterDirection::Smudge);
            prop_assert_eq!(apply_docs_filter(&smudged, &home(), FilterDirection::Clean), text.into_bytes());
        }
    }
}
