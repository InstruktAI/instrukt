//! Rendering: the read pipeline's text steps, shared by `docs get` and by
//! publication's compilation.
//!
//! One rendering takes a text through, in order: the frontmatter strip, the
//! planned lens, the `@config` configured values and the `Required reads`
//! strip; references are ids and stay as written
//! (`project/instrukt/spec/feature/docs/progressive-retrieval`, read
//! pipeline). The lens goes before `@config`, so hidden content is never
//! resolved. Nothing runs a command or includes a file: a leftover `@exec`
//! or `@include` stays inert text.
//!
//! The two callers differ in how a configured value resolves and what a
//! refusal means; both are the [`RenderContext`].

use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use super::_corpus::{Lens, Version, required_reads_section};
use super::_markers::{self, Side};
use super::_refs::Reference;
use super::_text::{self, Directive};
use super::DocsError;
use crate::config::{Configuration, Env, render_yaml};
use crate::home::Home;
use crate::ids::ProjectSlug;

/// Reads `root/relative` when it exists and lies inside `root` once
/// symbolic links are resolved; `None` when it does not exist. A relative
/// path climbing out of `root`, or a link leading out of it, is refused
/// with the error `outside` makes of the path.
pub(super) fn read_confined(
    root: &Path,
    relative: &Path,
    outside: fn(PathBuf) -> DocsError,
) -> Result<Option<String>, DocsError> {
    let climbs = relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)));
    if climbs {
        return Err(outside(root.join(relative)));
    }
    let path = root.join(relative);
    let resolved = match path.canonicalize() {
        Ok(resolved) => resolved,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => return Err(DocsError::Read { path, source }),
    };
    let root = root.canonicalize().map_err(|source| DocsError::Read {
        path: root.to_path_buf(),
        source,
    })?;
    if !resolved.starts_with(&root) {
        return Err(outside(path));
    }
    std::fs::read_to_string(&resolved)
        .map(Some)
        .map_err(|source| DocsError::Read { path, source })
}

/// When a configured value resolves, and what a refusal means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Moment {
    /// At read: the reader's session; a refusal becomes a notice.
    Read,
    /// At publication: no runtime session; a refusal fails the rendering.
    Publish,
}

/// How `@config` directives resolve: in-process through configuration's
/// `get`, with its interpolation and its refusals. No shell, no
/// subprocess, no working directory, nothing cached.
#[derive(Debug, Clone)]
pub struct Values<'a> {
    configuration: &'a Configuration,
    env: Env,
    moment: Moment,
}

impl<'a> Values<'a> {
    /// At read: `configuration` in the reader's environment, so `runtime.*`
    /// paths resolve for the reader's own session. A refusal replaces the
    /// directive with a notice and is listed in
    /// [`Rendered::config_refusals`].
    pub fn read(configuration: &'a Configuration) -> Self {
        Self {
            configuration,
            env: Env::from_process(),
            moment: Moment::Read,
        }
    }

    /// At publication: the publishing machine's `configuration` with no
    /// runtime session. A `runtime.*` path is refused (validation reports
    /// it first), and so is any value interpolating a session; any refusal
    /// fails the rendering ([`DocsError::ConfigUnresolved`]).
    pub fn publish(configuration: &'a Configuration) -> Self {
        let sessions: Vec<&str> = configuration
            .runtimes()
            .iter()
            .map(|r| r.session_id_env.as_str())
            .collect();
        let env = Env::from_pairs(std::env::vars_os().filter_map(|(k, v)| {
            let k = k.into_string().ok()?;
            (!sessions.contains(&k.as_str())).then_some((k, v.into_string().ok()?))
        }));
        Self {
            configuration,
            env,
            moment: Moment::Publish,
        }
    }

    /// The value at `path`, or why it cannot be resolved.
    fn resolve(&self, path: &str) -> Result<String, String> {
        if self.moment == Moment::Publish && is_runtime(path) {
            return Err("publication has no runtime session".to_owned());
        }
        // As `instrukt config get` prints it: a string as is, anything else,
        // structured values included, as YAML.
        match self.configuration.get(Some(path), &self.env) {
            Ok(serde_yaml_ng::Value::String(text)) => Ok(text),
            Ok(other) => render_yaml(&other).map_err(|err| err.to_string()),
            Err(err) => Err(err.to_string()),
        }
    }
}

/// Whether `path` addresses the active runtime session.
pub(super) fn is_runtime(path: &str) -> bool {
    path == "runtime" || path.starts_with("runtime.")
}

/// Everything a rendering resolves outside the text itself.
#[derive(Debug, Clone)]
pub struct RenderContext<'a> {
    /// The home whose user directory a master's `~/` path references are
    /// read against.
    pub home: &'a Home,
    /// How `@config` directives resolve.
    pub values: &'a Values<'a>,
    /// The slug the text's repository declares, which a master's
    /// `docs/project/…` path references name.
    pub project: Option<&'a ProjectSlug>,
}

/// One configured value that could not be resolved at read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigRefusal {
    pub path: String,
    /// Configuration's refusal, as it states it.
    pub reason: String,
}

/// A rendered text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// The text after every step.
    pub text: String,
    /// The references of its `## Required reads` section, read after the
    /// lens and before the section was stripped.
    pub required_reads: Vec<Reference>,
    /// The configured values that could not be resolved, in order; always
    /// empty under [`Values::publish`], where a refusal is an error.
    pub config_refusals: Vec<ConfigRefusal>,
}

impl Version {
    /// This version rendered through `lens`. The pending gate and the
    /// companion are the snippet's view ([`super::Snippet::view`]); this is
    /// every step after them.
    pub fn render(&self, lens: Lens<'_>, cx: &RenderContext<'_>) -> Result<Rendered, DocsError> {
        render_body(self.body(), lens, cx)
    }
}

/// Renders a whole source text — a snippet file, or a non-snippet source
/// such as an `AGENTS.master.md` — through `lens`: its frontmatter, when it
/// has one, is stripped, then every other step runs.
pub fn render(text: &str, lens: Lens<'_>, cx: &RenderContext<'_>) -> Result<Rendered, DocsError> {
    render_body(_text::split_frontmatter(text).body, lens, cx)
}

fn render_body(body: &str, lens: Lens<'_>, cx: &RenderContext<'_>) -> Result<Rendered, DocsError> {
    let lensed = lensed(body, lens);
    let required_reads = section_reads(&lensed, cx);
    let (resolved, config_refusals) = resolve_values(&lensed, cx.values)?;
    // References are ids and stay as written: no step rewrites them.
    let text = strip_required_reads(&resolved);
    Ok(Rendered {
        text,
        required_reads,
        config_refusals,
    })
}

/// The required reads a rendering of `body` through `lens` reports, read
/// without resolving any configured value: the lens is applied first, so
/// hidden required reads do not count. A value is fenced and never adds a reference, so this is
/// exactly [`Rendered::required_reads`].
pub(super) fn dependency_reads(
    body: &str,
    lens: Lens<'_>,
    cx: &RenderContext<'_>,
) -> Result<Vec<Reference>, DocsError> {
    Ok(section_reads(&lensed(body, lens), cx))
}

fn lensed(body: &str, lens: Lens<'_>) -> String {
    let scan = _markers::scan(body);
    _markers::resolve(body, &scan, |block| match lens {
        Lens::Target(slug) if block.slug == slug.as_str() => Side::Delivered,
        _ => Side::Current,
    })
}

/// `text` with each span replaced by its replacement; spans are ordered
/// and disjoint.
fn splice(text: &str, replacements: Vec<(Range<usize>, String)>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (span, replacement) in replacements {
        out.push_str(&text[at..span.start]);
        out.push_str(&replacement);
        at = span.end;
    }
    out.push_str(&text[at..]);
    out
}

/// Every visible `@config` directive replaced by its value as a fenced
/// block, or at read by a notice naming the path and why it was refused.
/// An `@exec` directive is left exactly as written: it runs nothing.
fn resolve_values(
    body: &str,
    values: &Values<'_>,
) -> Result<(String, Vec<ConfigRefusal>), DocsError> {
    let mut replacements = Vec::new();
    let mut refusals = Vec::new();
    for directive in _text::directives(&_text::lines(body)) {
        let Directive::Config { path, span } = directive else {
            continue;
        };
        match values.resolve(&path) {
            Ok(value) => replacements.push((span, fenced(&value))),
            Err(reason) if values.moment == Moment::Publish => {
                return Err(DocsError::ConfigUnresolved { path, reason });
            }
            Err(reason) => {
                replacements.push((
                    span,
                    format!("> instrukt: configured value `{path}` cannot be resolved: {reason}"),
                ));
                refusals.push(ConfigRefusal { path, reason });
            }
        }
    }
    tracing::trace!(
        event = "docs_config_resolved",
        resolved = replacements.len() - refusals.len(),
        refused = refusals.len(),
    );
    Ok((splice(body, replacements), refusals))
}

/// A value as a fenced block whose fence is longer than any backtick run
/// inside it; nothing for an empty value.
fn fenced(value: &str) -> String {
    let output = value.trim_end_matches(['\n', '\r']);
    if output.trim().is_empty() {
        return String::new();
    }
    let longest = output.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}\n{output}\n{fence}")
}

/// The entries of `body`'s `## Required reads` section.
fn section_reads(body: &str, cx: &RenderContext<'_>) -> Vec<Reference> {
    let lines = _text::lines(body);
    super::_corpus::section_reads(&lines, cx.project, cx.home.user_home())
}

/// `body` without its `## Required reads` section, heading included.
fn strip_required_reads(body: &str) -> String {
    let lines = _text::lines(body);
    let Some(section) = required_reads_section(&lines) else {
        return body.to_owned();
    };
    let from = lines
        .get(section.start.saturating_sub(1))
        .map_or(body.len(), |line| line.span.start);
    let to = lines
        .get(section.end)
        .map_or(body.len(), |line| line.span.start);
    splice(body, vec![(from..to, String::new())])
}
