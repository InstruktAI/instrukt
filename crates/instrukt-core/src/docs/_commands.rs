//! Shared-doctrine command-invocation notice check
//! (`project/instrukt/spec/feature/docs/snippet-validation`, Notices).
//!
//! Scans inline code spans and fenced code blocks of `general` and domain
//! snippets (current versions and non-empty companions) for invocations of
//! `instrukt <subcommand>` matched against the binary's Clap-derived
//! subcommand set on [`CoreWiring::cli_subcommands`]. Emits non-failing
//! [`NoticeCode::CommandInSharedDoctrine`] notices; `project` snippets,
//! `third-party` docs, and skipped `_`-prefixed directories are exempt.

use std::collections::BTreeSet;
use std::path::Path;

use super::_corpus::{Corpus, Stray};
use super::_finding::{Notice, NoticeCode};
use super::_identity::Scope;
use super::_text;
use crate::interfaces::CoreWiring;

const BINARY_NAME: &str = "instrukt";

/// Records a [`NoticeCode::CommandInSharedDoctrine`] notice for every
/// `instrukt <subcommand>` invocation inside inline code spans or fenced
/// code blocks of `general` and domain snippets (current versions and
/// non-empty companions).
pub(super) fn check(corpus: &Corpus, wiring: &CoreWiring, out: &mut Vec<Notice>) {
    if wiring.cli_subcommands.is_empty() {
        return;
    }
    for snippet in corpus.snippets() {
        if snippet.class().scope != Scope::Global {
            continue;
        }
        scan_body(
            snippet.current().path(),
            snippet.current().body(),
            &wiring.cli_subcommands,
            out,
        );
        for companion in snippet.companions() {
            if let Some(next) = companion.next() {
                scan_body(next.path(), next.body(), &wiring.cli_subcommands, out);
            }
        }
    }
    for stray in corpus.strays() {
        if let Stray::Companion {
            class,
            version: Some(version),
            ..
        } = stray
            && class.scope == Scope::Global
        {
            scan_body(version.path(), version.body(), &wiring.cli_subcommands, out);
        }
    }
}

fn scan_body(path: &Path, body: &str, subcommands: &BTreeSet<String>, out: &mut Vec<Notice>) {
    let lines = _text::lines(body);
    for segment in _text::code_segments(&lines) {
        for subcommand in scan_segment(&segment, subcommands) {
            out.push(Notice {
                path: path.to_path_buf(),
                code: NoticeCode::CommandInSharedDoctrine,
                value: Some(format!("{BINARY_NAME} {subcommand}")),
            });
        }
    }
}

/// Matched subcommand names for every `instrukt <subcommand>` invocation
/// inside one code segment (an inline code span or one line of a fenced
/// code block).
fn scan_segment<'a>(segment: &'a str, subcommands: &BTreeSet<String>) -> Vec<&'a str> {
    let mut matches = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = segment[cursor..].find(BINARY_NAME) {
        let start = cursor + rel;
        let after_name = start + BINARY_NAME.len();
        let valid_prefix = segment[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_ascii_alphanumeric() && !matches!(c, '_' | '-' | '.' | ':'));
        if !valid_prefix {
            cursor = after_name;
            continue;
        }
        let rest = &segment[after_name..];
        let ws_len = rest
            .bytes()
            .take_while(|b| *b == b' ' || *b == b'\t')
            .count();
        if ws_len == 0 {
            cursor = after_name;
            continue;
        }
        let after_ws = &rest[ws_len..];
        let token_len = after_ws
            .char_indices()
            .find(|(_, ch)| is_token_delimiter(*ch))
            .map_or(after_ws.len(), |(i, _)| i);
        let token = &after_ws[..token_len];
        if !token.is_empty() && subcommands.contains(token) {
            matches.push(token);
            cursor = after_name + ws_len + token_len;
        } else {
            cursor = after_name;
        }
    }
    matches
}

fn is_token_delimiter(ch: char) -> bool {
    ch.is_whitespace()
        || matches!(
            ch,
            ';' | '|' | '&' | ')' | ']' | '}' | '"' | '\'' | ',' | '`'
        )
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::docs::SourceFile;
    use crate::ids::ProjectSlug;

    fn subcommands(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    fn wiring(names: &[&str]) -> CoreWiring {
        CoreWiring {
            cli_subcommands: subcommands(names),
            ..CoreWiring::default()
        }
    }

    fn snippet_body(extra: &str) -> String {
        format!(
            "---\ndescription: Sample.\n---\n\n# Sample — Concept\n\n## What\n\n{extra}\n\n## Why\n\nBecause.\n"
        )
    }

    #[test]
    fn segment_matching_enforces_token_and_prefix_boundaries() {
        let cmds = subcommands(&["sync", "docs", "todo", "status", "dev-seed"]);

        assert_eq!(scan_segment("instrukt sync", &cmds), vec!["sync"]);
        assert_eq!(
            scan_segment("instrukt docs get --domain sdlc", &cmds),
            vec!["docs"]
        );
        assert_eq!(
            scan_segment("$ instrukt todo list && instrukt status", &cmds),
            vec!["todo", "status"]
        );
        assert_eq!(
            scan_segment(
                "$(instrukt status); /usr/local/bin/instrukt dev-seed",
                &cmds
            ),
            vec!["status", "dev-seed"]
        );
        assert_eq!(scan_segment("instrukt instrukt sync", &cmds), vec!["sync"]);

        for non_invocation in [
            "instrukt",
            "instrukt.yaml",
            "instrukt.db",
            "instrukt-core",
            "instrukt_ai_logging",
            "instrukt:todo",
            "~/.instrukt/docs",
            "instrukt <command> -h",
            "instrukt docs/index.yaml",
            "instrukt docs.md",
            "instrukt docs-repo",
            "instrukt --help",
            "my-instrukt sync",
            "foo_instrukt sync",
            "a.instrukt sync",
            "t3:instrukt sync",
            "Instrukt sync",
        ] {
            assert!(
                scan_segment(non_invocation, &cmds).is_empty(),
                "unexpected match for {non_invocation:?}"
            );
        }
    }

    #[test]
    fn corpus_check_scans_shared_doctrine_and_companions_and_exempts_other_positions() {
        let project = ProjectSlug::parse("demo").ok();
        let corpus = Corpus::from_sources(
            project,
            None,
            [
                SourceFile {
                    path: "docs/general/concept/a.md".into(),
                    text: snippet_body("Use `instrukt sync` or `instrukt\ndocs get`."),
                },
                SourceFile {
                    path: "docs/domain/sdlc/concept/b.md".into(),
                    text: snippet_body(
                        "Plain instrukt sync prose and `instrukt` bare.\n\n```sh\ninstrukt todo list\n```",
                    ),
                },
                SourceFile {
                    path: "docs/domain/sdlc/concept/b.next-b.md".into(),
                    text: snippet_body("Companion calls `instrukt status`."),
                },
                SourceFile {
                    path: "docs/domain/sdlc/concept/b.retire-b.md".into(),
                    text: String::new(),
                },
                SourceFile {
                    path: "docs/project/concept/p.md".into(),
                    text: snippet_body("Project calls `instrukt sync`.\n\n```sh\ninstrukt docs validate\n```"),
                },
                SourceFile {
                    path: "docs/third-party/tool/t.md".into(),
                    text: "---\ndescription: Tool.\n---\n\n# Tool\n\n`instrukt sync`\n\n## Sources\n\n- https://example.com\n"
                        .into(),
                },
                SourceFile {
                    path: "docs/_compressed/general/concept/a.md".into(),
                    text: "Compressed `instrukt sync`.".into(),
                },
            ],
        );

        let mut notices = Vec::new();
        check(
            &corpus,
            &wiring(&["sync", "docs", "todo", "status"]),
            &mut notices,
        );
        notices.sort();
        notices.dedup();

        assert_eq!(
            notices,
            vec![
                Notice {
                    path: PathBuf::from("docs/domain/sdlc/concept/b.md"),
                    code: NoticeCode::CommandInSharedDoctrine,
                    value: Some("instrukt todo".into()),
                },
                Notice {
                    path: PathBuf::from("docs/domain/sdlc/concept/b.next-b.md"),
                    code: NoticeCode::CommandInSharedDoctrine,
                    value: Some("instrukt status".into()),
                },
                Notice {
                    path: PathBuf::from("docs/general/concept/a.md"),
                    code: NoticeCode::CommandInSharedDoctrine,
                    value: Some("instrukt docs".into()),
                },
                Notice {
                    path: PathBuf::from("docs/general/concept/a.md"),
                    code: NoticeCode::CommandInSharedDoctrine,
                    value: Some("instrukt sync".into()),
                },
            ]
        );
    }

    #[test]
    fn empty_cli_subcommands_produces_no_notices() {
        let corpus = Corpus::from_sources(
            None,
            None,
            [SourceFile {
                path: "docs/general/concept/a.md".into(),
                text: snippet_body("Run `instrukt sync`."),
            }],
        );
        let mut notices = Vec::new();
        check(&corpus, &CoreWiring::default(), &mut notices);
        assert!(notices.is_empty());
    }
}
