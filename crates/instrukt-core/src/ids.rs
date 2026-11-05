//! Domain identifiers shared by every instrukt crate.
//!
//! Each identifier is a validated newtype: a value of the type is known to be
//! well formed, so functions taking one never re-check it. Construct with
//! `parse`, or `FromStr`; serde goes through the same validation.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::docs::Taxonomy;

/// A value was not a well-formed identifier of the named kind.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} `{value}`: {rule}")]
pub struct IdError {
    pub kind: &'static str,
    pub value: String,
    pub rule: &'static str,
}

macro_rules! identifier {
    ($(#[$meta:meta])* $name:ident, $kind:literal, $check:path, $rule:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            /// Validates `value` as this identifier.
            pub fn parse(value: &str) -> Result<Self, IdError> {
                if $check(value) {
                    Ok(Self(value.to_owned()))
                } else {
                    Err(IdError { kind: $kind, value: value.to_owned(), rule: $rule })
                }
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = IdError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::parse(s)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let raw = String::deserialize(deserializer)?;
                Self::parse(&raw).map_err(serde::de::Error::custom)
            }
        }
    };
}

identifier!(
    /// A project's slug: the `project` key of its project layer.
    ProjectSlug,
    "project slug",
    is_slug,
    "lowercase letters, digits, `.`, `_` and `-`, starting with a letter or digit"
);

identifier!(
    /// A todo's project-local address.
    TodoSlug,
    "todo slug",
    is_slug,
    "lowercase letters, digits, `.`, `_` and `-`, starting with a letter or digit"
);

identifier!(
    /// A published knowledge domain (`sdlc`, …), or `general`, which is
    /// addressed like one.
    DomainName,
    "domain",
    is_domain,
    "lowercase letters, digits, `.`, `_` and `-`, starting with a letter or digit, and not a reserved name (`project`, `snippets`, `third-party`)"
);

identifier!(
    /// A snippet's derived identity, e.g. `general/policy/x`,
    /// `sdlc/policy/testing` or
    /// `project/<slug>/spec/feature/<context>/<concept>` — always one of the
    /// qualified forms of `general/spec/snippet-authoring-schema`, never a
    /// shorter one. A feature spec's identity is also its spec id.
    SnippetId,
    "snippet id",
    is_snippet_id,
    "`<domain>/<taxonomy>/<rest>` or `project/<slug>/<taxonomy>/<rest>`"
);

identifier!(
    /// A third-party doc's identity: its path below `docs/third-party/`,
    /// without `.md`.
    ThirdPartyId,
    "third-party doc id",
    is_path_id,
    "`/`-separated segments of letters, digits, `.`, `_` and `-`, no empty or `..` segment"
);

identifier!(
    /// A use-case id within one feature spec: `UC-` then a letter or digit,
    /// then letters, digits or hyphens (conventionally `UC-<area><n>`).
    UseCaseId,
    "use-case id",
    is_use_case_id,
    "`UC-` followed by a letter or digit, then letters, digits or hyphens"
);

identifier!(
    /// A full git commit id (40 or 64 lowercase hex characters).
    CommitSha,
    "commit sha",
    is_full_sha,
    "40 or 64 lowercase hexadecimal characters"
);

identifier!(
    /// A session role: free text in core (`general/spec/snippet-authoring-schema`,
/// `role`); membership in a flavor's `roles:` is checked by workflow.
    RoleName,
    "role",
    is_role,
    "non-empty text on one line, without leading or trailing whitespace"
);

identifier!(
    /// The `name` of a runtime-mapping entry (`runtimes.<n>.name`), the
    /// first half of a [`SessionRef`].
    RuntimeName,
    "runtime name",
    is_runtime_name,
    "non-empty text without whitespace, control characters or `:`"
);

identifier!(
    /// A runtime-native session id, as the runtime reports it; instrukt
    /// never mints a second identity for the same session
    /// (`session-continuity`, "Identity and reuse").
    SessionId,
    "session id",
    is_session_id,
    "non-empty text without whitespace or control characters"
);

/// One session of one runtime, written `{runtime}:{id}`: the form artifact
/// writers, stamps and `--session <runtime:id>` use (`session-continuity`,
/// "Identity and reuse"; `approvals`, "Command boundary"). Shared by work's
/// session bindings and stamp actors, workflow's `sessions.bound` fact and
/// the runner's sessions facade.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessionRef {
    pub runtime: RuntimeName,
    pub id: SessionId,
}

impl SessionRef {
    /// Parses `{runtime}:{id}`, splitting at the first `:`; the id may
    /// itself contain `:`.
    pub fn parse(value: &str) -> Result<Self, IdError> {
        let refused = || IdError {
            kind: "session reference",
            value: value.to_owned(),
            rule: "`<runtime>:<id>`, a runtime name, `:`, then a session id",
        };
        let (runtime, id) = value.split_once(':').ok_or_else(refused)?;
        Ok(Self {
            runtime: RuntimeName::parse(runtime).map_err(|_| refused())?,
            id: SessionId::parse(id).map_err(|_| refused())?,
        })
    }
}

impl fmt::Display for SessionRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.runtime, self.id)
    }
}

impl FromStr for SessionRef {
    type Err = IdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl Serialize for SessionRef {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for SessionRef {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// The id a test binding names: a feature spec plus one of its use cases.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct UseCaseRef {
    pub spec: SnippetId,
    pub use_case: UseCaseId,
}

impl fmt::Display for UseCaseRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.spec, self.use_case)
    }
}

impl SnippetId {
    /// Whether the id names a project snippet (`project/<slug>/…`).
    pub fn is_project(&self) -> bool {
        self.0.starts_with("project/")
    }

    /// The domain a published snippet's id opens with (`general`, `sdlc`,
    /// …); `None` for a project snippet.
    pub fn domain(&self) -> Option<DomainName> {
        if self.is_project() {
            return None;
        }
        DomainName::parse(self.0.split('/').next()?).ok()
    }
}

fn is_slug(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// Names the identity grammar and the home's published tree reserve; no
/// published domain carries one. `project` opens a project snippet's id;
/// `snippets` and `third-party` are directories of the published tree.
const RESERVED_DOMAINS: [&str; 3] = ["project", "snippets", "third-party"];

fn is_domain(value: &str) -> bool {
    is_slug(value) && !RESERVED_DOMAINS.contains(&value)
}

fn is_role(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && !value.chars().any(char::is_control)
}

fn is_snippet_id(value: &str) -> bool {
    if !is_path_id(value) {
        return false;
    }
    let segments: Vec<&str> = value.split('/').collect();
    match segments.as_slice() {
        ["project", slug, taxonomy, rest @ ..] => {
            is_slug(slug) && Taxonomy::from_dir(taxonomy).is_some() && !rest.is_empty()
        }
        [domain, taxonomy, rest @ ..] => {
            is_domain(domain) && Taxonomy::from_dir(taxonomy).is_some() && !rest.is_empty()
        }
        _ => false,
    }
}

fn is_path_id(value: &str) -> bool {
    !value.is_empty()
        && value.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
}

/// `UC-`, then a letter or digit, then letters, digits or hyphens
/// (`use-case-coverage`, "Use cases").
fn is_use_case_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("UC-") else {
        return false;
    };
    rest.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric())
        && rest.chars().all(is_use_case_char)
}

fn is_use_case_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-'
}

impl UseCaseId {
    /// The first use-case id token in `text`: `UC-` where no letter, digit,
    /// `_` or `-` precedes it, extended over the id characters that follow.
    /// A token that is not a well-formed id (`UC-` followed by no letter or
    /// digit) is passed over.
    pub fn first_in(text: &str) -> Option<Self> {
        let mut from = 0;
        while let Some(at) = text[from..].find("UC-").map(|i| from + i) {
            let after = at + 3;
            let starts_token = text[..at]
                .chars()
                .next_back()
                .is_none_or(|c| !is_use_case_char(c) && c != '_');
            let len = text[after..]
                .find(|c: char| !is_use_case_char(c))
                .unwrap_or(text.len() - after);
            if starts_token && let Ok(id) = Self::parse(&text[at..after + len]) {
                return Some(id);
            }
            from = after;
        }
        None
    }
}

fn is_session_id(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(|c| c.is_whitespace() || c.is_control())
}

fn is_runtime_name(value: &str) -> bool {
    is_session_id(value) && !value.contains(':')
}

fn is_full_sha(value: &str) -> bool {
    (value.len() == 40 || value.len() == 64)
        && value
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_accept_lowercase_kebab_and_refuse_the_rest() {
        assert!(ProjectSlug::parse("instrukt").is_ok());
        assert!(TodoSlug::parse("first-build.v2_a").is_ok());
        for bad in ["", "Instrukt", "-lead", "has space", "a/b"] {
            assert!(ProjectSlug::parse(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn snippet_ids_are_path_shaped() {
        assert!(SnippetId::parse("sdlc/policy/rust/core").is_ok());
        assert!(SnippetId::parse("project/instrukt/spec/feature/docs/x").is_ok());
        for bad in [
            "",
            "a//b",
            "a/../b",
            "/abs",
            "a/b/",
            "testing",
            "sdlc/policy",
            "sdlc/notes/x",
            "project/policy/x",
            "local/projects/demo/spec/x",
            "project/Bad/spec/x",
            "project/demo/spec",
        ] {
            assert!(SnippetId::parse(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn a_snippet_id_tells_its_domain_or_that_it_is_a_project_one() {
        let general = SnippetId::parse("general/policy/x").unwrap();
        let project = SnippetId::parse("project/demo/spec/x").unwrap();
        assert_eq!(general.domain(), DomainName::parse("general").ok());
        assert_eq!((project.is_project(), project.domain()), (true, None));
    }

    #[test]
    fn domains_refuse_reserved_names() {
        assert!(DomainName::parse("general").is_ok());
        for bad in ["project", "snippets", "third-party", "Sdlc"] {
            assert!(DomainName::parse(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn roles_are_free_text_on_one_line() {
        assert!(RoleName::parse("integrator").is_ok());
        assert!(RoleName::parse("Security Reviewer").is_ok());
        for bad in ["", " lead", "trail ", "two\nlines"] {
            assert!(RoleName::parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn third_party_ids_are_path_shaped() {
        assert!(ThirdPartyId::parse("rust/tokio").is_ok());
        assert!(ThirdPartyId::parse("a//b").is_err());
    }

    #[test]
    fn use_case_ids_are_uc_then_a_letter_or_digit_then_letters_digits_or_hyphens() {
        for good in [
            "UC-CL1",
            "UC-ST12",
            "UC-TH1-alt",
            "UC-SFV1b",
            "UC-1",
            "UC-cl1",
        ] {
            assert!(UseCaseId::parse(good).is_ok(), "{good} should be accepted");
        }
        for bad in [
            "uc-CL1", "CL1", "UC-", "UC--1", "UC-TH 1", "UC-TH_1", "UC-TH1:",
        ] {
            assert!(UseCaseId::parse(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn the_first_use_case_token_of_a_text_is_found_whole() {
        let heading = "#### UC-TH1-alt: and UC-TH2";
        assert_eq!(
            UseCaseId::first_in(heading),
            Some(UseCaseId::parse(&heading[5..15]).unwrap())
        );
        let glued = "XUC-AB1 then UC-CD2.";
        assert_eq!(
            UseCaseId::first_in(glued),
            Some(UseCaseId::parse(&glued[13..19]).unwrap())
        );
        assert_eq!(UseCaseId::first_in("UC- and UC-- only"), None);
    }

    #[test]
    fn shas_are_full_lowercase_hex() {
        assert!(CommitSha::parse(&"a".repeat(40)).is_ok());
        assert!(CommitSha::parse(&"0".repeat(64)).is_ok());
        assert!(CommitSha::parse("abc1234").is_err());
        assert!(CommitSha::parse(&"A".repeat(40)).is_err());
    }

    #[test]
    fn session_refs_split_at_the_first_colon() {
        let session = SessionRef::parse("tmux:main:3").unwrap();
        let expected = SessionRef {
            runtime: RuntimeName::parse("tmux").unwrap(),
            id: SessionId::parse("main:3").unwrap(),
        };
        assert_eq!(session, expected);
        assert_eq!(SessionRef::parse(&session.to_string()).unwrap(), session);
        for bad in ["", "tmux", ":id", "tmux:", "tm ux:id", "tmux:a b"] {
            assert!(SessionRef::parse(bad).is_err(), "{bad:?} should be refused");
        }
        let round: SessionRef = serde_yaml_ng::from_str("claude:abc").unwrap();
        let again: SessionRef =
            serde_yaml_ng::from_str(&serde_yaml_ng::to_string(&round).unwrap()).unwrap();
        assert_eq!(again, round);
    }

    #[test]
    fn serde_goes_through_validation() {
        let ok: ProjectSlug = serde_yaml_ng::from_str("instrukt").unwrap();
        assert_eq!(ok, ProjectSlug::parse("instrukt").unwrap());
        assert!(serde_yaml_ng::from_str::<ProjectSlug>("Not A Slug").is_err());
    }
}
