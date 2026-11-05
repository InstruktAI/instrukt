//! Placeholder rendering for commands and templates
//! (`docs/project/spec/domain/workflow.md`, "Flavor-facing contract").
//!
//! A placeholder is `{name}`, `name` being dot-separated segments of ASCII
//! letters, digits, `_` and `-`. Any other brace text is literal, and so is
//! `${…}`, which is an environment reference of configuration, not a
//! workflow placeholder.

use std::fmt;

use instrukt_core::ids::{ProjectSlug, TodoSlug};

use super::_facts::{FactName, Facts};

/// Answers configuration paths for placeholders that are neither reserved
/// bindings nor declared facts, resolved as `instrukt config get` does.
pub trait ConfigLookup {
    /// The expanded value at `path`; `Ok(None)` when configuration has no
    /// such key.
    fn lookup(&self, path: &str) -> Result<Option<String>, ConfigLookupError>;
}

impl<F> ConfigLookup for F
where
    F: Fn(&str) -> Result<Option<String>, ConfigLookupError>,
{
    fn lookup(&self, path: &str) -> Result<Option<String>, ConfigLookupError> {
        self(path)
    }
}

/// Configuration could not answer a path (for example, no runtime session
/// variable is set for a `runtime.*` path).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ConfigLookupError(pub String);

/// A rendered command or template, or why it could not be rendered.
pub type Rendering = Result<String, RenderFailure>;

/// Every placeholder that could not be filled; nothing was rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderFailure {
    pub findings: Vec<RenderFinding>,
}

impl fmt::Display for RenderFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.findings.iter().map(ToString::to_string).collect();
        f.write_str(&parts.join("; "))
    }
}

impl std::error::Error for RenderFailure {}

/// One placeholder that could not be filled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RenderFinding {
    /// A declared fact (or the reserved `kind`) has no value; it stays
    /// absent and is not looked up in configuration.
    #[error("`{{{0}}}` names a fact that is absent")]
    Absent(String),
    /// Not a reserved binding, not a declared fact, and not a configuration
    /// path.
    #[error("`{{{0}}}` is not a reserved binding, a declared fact or a configuration path")]
    Unknown(String),
    #[error("`{{{name}}}`: configuration could not answer: {message}")]
    Config { name: String, message: String },
    /// `{brief}` outside an action command.
    #[error("`{{brief}}` is only available in an action's command")]
    BriefUnavailable,
    /// `{brief}` in a command whose template could not be rendered.
    #[error("`{{brief}}` is unavailable because the template could not be rendered")]
    BriefNotRendered,
}

/// A piece of command or template text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece<'t> {
    Literal(&'t str),
    Placeholder(&'t str),
}

/// Splits `text` into literal runs and placeholders.
fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut literal_start = 0;
    let mut cursor = 0;
    while let Some(offset) = text[cursor..].find('{') {
        let open = cursor + offset;
        let after = &text[open + 1..];
        let name_len = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')))
            .unwrap_or(after.len());
        let name = &after[..name_len];
        let closes = after[name_len..].starts_with('}');
        let env_form = text[..open].ends_with('$');
        if closes && !env_form && is_name(name) {
            if literal_start < open {
                out.push(Piece::Literal(&text[literal_start..open]));
            }
            out.push(Piece::Placeholder(name));
            cursor = open + 1 + name_len + 1;
            literal_start = cursor;
        } else {
            cursor = open + 1;
        }
    }
    if literal_start < text.len() {
        out.push(Piece::Literal(&text[literal_start..]));
    }
    out
}

fn is_name(name: &str) -> bool {
    !name.is_empty() && name.split('.').all(|segment| !segment.is_empty())
}

/// The placeholder names in `text`, in order of appearance.
pub(crate) fn placeholders(text: &str) -> impl Iterator<Item = &str> {
    pieces(text).into_iter().filter_map(|piece| match piece {
        Piece::Placeholder(name) => Some(name),
        Piece::Literal(_) => None,
    })
}

/// What placeholders resolve against, in precedence order: reserved
/// bindings, declared facts, configuration.
pub(crate) struct Bindings<'a> {
    pub slug: &'a TodoSlug,
    pub project: &'a ProjectSlug,
    pub facts: &'a Facts,
    /// The rendered template, available to an action's command only.
    pub brief: Option<&'a Rendering>,
    pub config: &'a dyn ConfigLookup,
}

/// Renders `text`. Fails with every unfillable placeholder; never invents
/// a value.
pub(crate) fn render(text: &str, bindings: &Bindings<'_>) -> Rendering {
    let mut out = String::with_capacity(text.len());
    let mut findings = Vec::new();
    for piece in pieces(text) {
        match piece {
            Piece::Literal(literal) => out.push_str(literal),
            Piece::Placeholder(name) => match resolve(name, bindings) {
                Ok(value) => out.push_str(&value),
                Err(finding) => findings.push(finding),
            },
        }
    }
    if findings.is_empty() {
        Ok(out)
    } else {
        Err(RenderFailure { findings })
    }
}

fn resolve(name: &str, bindings: &Bindings<'_>) -> Result<String, RenderFinding> {
    match name {
        "slug" => return Ok(bindings.slug.to_string()),
        "project" => return Ok(bindings.project.to_string()),
        "brief" => {
            return match bindings.brief {
                None => Err(RenderFinding::BriefUnavailable),
                Some(Ok(brief)) => Ok(brief.clone()),
                Some(Err(_)) => Err(RenderFinding::BriefNotRendered),
            };
        }
        _ => {}
    }
    // `kind` is reserved and also a declared fact; both read the same value.
    if let Some(fact) = FactName::parse(name) {
        return bindings
            .facts
            .get(fact)
            .map(|value| value.render())
            .ok_or_else(|| RenderFinding::Absent(name.to_owned()));
    }
    match bindings.config.lookup(name) {
        Ok(Some(value)) => Ok(value),
        Ok(None) => Err(RenderFinding::Unknown(name.to_owned())),
        Err(err) => Err(RenderFinding::Config {
            name: name.to_owned(),
            message: err.0,
        }),
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::super::_facts::FactValue;
    use super::*;

    fn no_config(_: &str) -> Result<Option<String>, ConfigLookupError> {
        Ok(None)
    }

    fn slug() -> TodoSlug {
        TodoSlug::parse("first-build").unwrap()
    }

    fn project() -> ProjectSlug {
        ProjectSlug::parse("instrukt").unwrap()
    }

    #[test]
    fn placeholders_are_names_in_braces_and_other_braces_are_literal() {
        let text = "a {slug} b ${HOME} c {not a name} d {} e {x..y} f {prd.digest}{";
        assert_eq!(
            placeholders(text).collect::<Vec<_>>(),
            vec!["slug", "prd.digest"]
        );
    }

    #[test]
    fn reserved_bindings_resolve_first() {
        let facts = Facts::new()
            .with("kind", FactValue::Text("bug".into()))
            .unwrap();
        let config = |path: &str| -> Result<Option<String>, ConfigLookupError> {
            Ok(Some(format!("config:{path}")))
        };
        let brief_text = "the brief".to_owned();
        let brief: Rendering = Ok(brief_text.clone());
        let (slug, project) = (slug(), project());
        let bindings = Bindings {
            slug: &slug,
            project: &project,
            facts: &facts,
            brief: Some(&brief),
            config: &config,
        };
        assert_eq!(
            render("{slug}|{project}|{kind}|{brief}", &bindings),
            Ok(format!("{slug}|{project}|bug|{brief_text}"))
        );
    }

    #[test]
    fn an_absent_declared_fact_does_not_fall_through_to_configuration() {
        let config =
            |path: &str| -> Result<Option<String>, ConfigLookupError> { Ok(Some(path.to_owned())) };
        let (slug, project, facts) = (slug(), project(), Facts::new());
        let bindings = Bindings {
            slug: &slug,
            project: &project,
            facts: &facts,
            brief: None,
            config: &config,
        };
        assert_eq!(
            render("{finalize.sha} {runtime.session_id}", &bindings),
            Err(RenderFailure {
                findings: vec![RenderFinding::Absent("finalize.sha".into())]
            })
        );
        assert_eq!(
            render("{runtime.session_id}", &bindings),
            Ok("runtime.session_id".to_owned())
        );
    }

    #[test]
    fn unknown_names_and_configuration_failures_are_findings() {
        let config = |path: &str| -> Result<Option<String>, ConfigLookupError> {
            if path.starts_with("runtime.") {
                Err(ConfigLookupError(
                    "no runtime session variable is set".into(),
                ))
            } else {
                Ok(None)
            }
        };
        let (slug, project, facts) = (slug(), project(), Facts::new());
        let bindings = Bindings {
            slug: &slug,
            project: &project,
            facts: &facts,
            brief: None,
            config: &config,
        };
        assert_eq!(
            render("{nope} {runtime.session_id} {brief}", &bindings),
            Err(RenderFailure {
                findings: vec![
                    RenderFinding::Unknown("nope".into()),
                    RenderFinding::Config {
                        name: "runtime.session_id".into(),
                        message: "no runtime session variable is set".into()
                    },
                    RenderFinding::BriefUnavailable,
                ]
            })
        );
    }

    #[test]
    fn lists_render_space_separated() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        let facts = Facts::new()
            .with(
                "claim_commits.unreviewed",
                FactValue::ShaList(vec![
                    instrukt_core::ids::CommitSha::parse(&a).unwrap(),
                    instrukt_core::ids::CommitSha::parse(&b).unwrap(),
                ]),
            )
            .unwrap();
        let (slug, project) = (slug(), project());
        let bindings = Bindings {
            slug: &slug,
            project: &project,
            facts: &facts,
            brief: None,
            config: &no_config,
        };
        assert_eq!(
            render("{claim_commits.unreviewed}", &bindings),
            Ok(format!("{a} {b}"))
        );
    }

    /// Placeholder names drawn from every resolution class.
    fn name() -> impl Strategy<Value = &'static str> {
        prop::sample::select(vec![
            "slug",
            "project",
            "kind",
            "prd.digest",
            "worktree.path",
            "build.complete",
            "runtime.session_id",
            "github.api_url",
            "unknown.name",
        ])
    }

    proptest! {
        /// Rendering either fills every placeholder from its binding or
        /// renders nothing; it never invents a value.
        #[test]
        fn rendering_never_invents_values(
            parts in prop::collection::vec(("[a-z ]{0,4}", name()), 0..6),
            kind in prop::option::of(prop::sample::select(vec!["feature", "bug"])),
            digest in prop::option::of("[a-f0-9]{0,6}"),
            complete in prop::option::of(any::<bool>()),
            has_session in any::<bool>(),
        ) {
            let mut facts = Facts::new();
            if let Some(kind) = kind {
                facts.insert("kind", FactValue::Text(kind.into())).unwrap();
            }
            if let Some(digest) = &digest {
                facts.insert("prd.digest", FactValue::Text(digest.clone())).unwrap();
            }
            if let Some(complete) = complete {
                facts.insert("build.complete", FactValue::Bool(complete)).unwrap();
            }
            let config = move |path: &str| -> Result<Option<String>, ConfigLookupError> {
                Ok(match path {
                    "runtime.session_id" if has_session => Some("s-1".into()),
                    "github.api_url" => Some("http://gh".into()),
                    _ => None,
                })
            };
            let (slug, project) = (slug(), project());
            let bindings = Bindings {
                slug: &slug, project: &project, facts: &facts, brief: None, config: &config,
            };
            let text: String = parts.iter().map(|(lit, n)| format!("{lit}{{{n}}}")).collect();
            let expected: Option<String> = parts
                .iter()
                .map(|(lit, n)| {
                    let value = match *n {
                        "slug" => Some("first-build".to_owned()),
                        "project" => Some("instrukt".to_owned()),
                        "kind" => kind.map(str::to_owned),
                        "prd.digest" => digest.clone().filter(|d| !d.is_empty()),
                        "worktree.path" | "unknown.name" => None,
                        "build.complete" => complete.map(|c| c.to_string()),
                        "runtime.session_id" => has_session.then(|| "s-1".to_owned()),
                        "github.api_url" => Some("http://gh".to_owned()),
                        _ => None,
                    };
                    value.map(|v| format!("{lit}{v}"))
                })
                .collect();
            match (render(&text, &bindings), expected) {
                (Ok(rendered), Some(expected)) => prop_assert_eq!(rendered, expected),
                (Err(failure), None) => prop_assert!(!failure.findings.is_empty()),
                (got, expected) => prop_assert!(false, "rendered {:?}, expected {:?}", got, expected),
            }
        }
    }
}
