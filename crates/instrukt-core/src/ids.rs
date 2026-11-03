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
    /// A published knowledge domain (`general`, `sdlc`, …).
    DomainName,
    "domain",
    is_domain,
    "lowercase letters, digits, `.`, `_` and `-`, starting with a letter or digit, and not a reserved name (`local`, `projects`, `project`, `snippets`)"
);

identifier!(
    /// A snippet's derived identity, e.g. `sdlc/policy/testing` or
    /// `local/projects/<slug>/spec/feature/<context>/<concept>` — always one
    /// of the two qualified forms of `general/spec/snippet-authoring-schema`,
    /// never a shorter one. A feature spec's identity is also its spec id.
    SnippetId,
    "snippet id",
    is_snippet_id,
    "`<domain>/<taxonomy>/<rest>` or `local/projects/<slug>/<taxonomy>/<rest>`"
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
    /// A use-case id within one feature spec: `UC-<area><n>`.
    UseCaseId,
    "use-case id",
    is_use_case_id,
    "`UC-` followed by an uppercase area code and a number"
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

fn is_slug(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_ascii_lowercase() || first.is_ascii_digit() => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
}

/// Names the identity grammar and the docs tree reserve; no published
/// domain carries one.
const RESERVED_DOMAINS: [&str; 4] = ["local", "projects", "project", "snippets"];

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
        ["local", "projects", slug, taxonomy, rest @ ..] => {
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

fn is_use_case_id(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("UC-") else {
        return false;
    };
    let area_len = rest.chars().take_while(|c| c.is_ascii_uppercase()).count();
    let number = &rest[area_len..];
    area_len > 0 && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit())
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
        assert!(SnippetId::parse("local/projects/instrukt/spec/feature/docs/x").is_ok());
        for bad in [
            "",
            "a//b",
            "a/../b",
            "/abs",
            "a/b/",
            "testing",
            "sdlc/policy",
            "sdlc/notes/x",
            "local/policy/x",
            "projects/policy/x",
            "local/projects/Bad/spec/x",
            "local/projects/demo/spec",
        ] {
            assert!(SnippetId::parse(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn domains_refuse_reserved_names() {
        assert!(DomainName::parse("general").is_ok());
        for bad in ["local", "projects", "project", "snippets", "Sdlc"] {
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
    fn use_case_ids_need_area_and_number() {
        assert!(UseCaseId::parse("UC-CL1").is_ok());
        assert!(UseCaseId::parse("UC-ST12").is_ok());
        for bad in ["UC-1", "UC-CL", "uc-CL1", "CL1", "UC-cl1"] {
            assert!(UseCaseId::parse(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn shas_are_full_lowercase_hex() {
        assert!(CommitSha::parse(&"a".repeat(40)).is_ok());
        assert!(CommitSha::parse(&"0".repeat(64)).is_ok());
        assert!(CommitSha::parse("abc1234").is_err());
        assert!(CommitSha::parse(&"A".repeat(40)).is_err());
    }

    #[test]
    fn serde_goes_through_validation() {
        let ok: ProjectSlug = serde_yaml_ng::from_str("instrukt").unwrap();
        assert_eq!(ok.as_str(), "instrukt");
        assert!(serde_yaml_ng::from_str::<ProjectSlug>("Not A Slug").is_err());
    }
}
