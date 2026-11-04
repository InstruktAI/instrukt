//! Rendering: the read pipeline's text steps, shared by `docs get` and by
//! publication's compilation.
//!
//! One rendering takes a text through, in order: the frontmatter strip, the
//! `@include` expansion, the planned lens, the `@config` configured values,
//! the inline references and the `Required reads` strip
//! (`local/projects/instrukt/spec/feature/docs/progressive-retrieval`, read
//! pipeline). The lens goes before `@config`, so hidden content is never
//! resolved. Nothing runs a command: a leftover `@exec` stays inert text.
//!
//! The two callers differ in where fragments come from and in how a
//! configured value resolves and what a refusal means; both are the
//! [`RenderContext`].

use std::ops::Range;
use std::path::{Component, Path, PathBuf};

use super::_corpus::{Corpus, Lens, Version, required_reads_section};
use super::_markers::{self, Side};
use super::_refs::{self, Reference};
use super::_text::{self, Directive};
use super::DocsError;
use crate::config::{Configuration, Env, render_yaml};
use crate::home::Home;
use crate::ids::ProjectSlug;

/// Where `@include` fragments come from.
#[derive(Debug, Clone, Copy)]
pub enum Fragments<'a> {
    /// The home's published fragments (`~/.instrukt/docs/snippets/`): a
    /// reader's `docs get`.
    Home(&'a Home),
    /// The corpus's own fragments (`docs/global/snippets/` of its committed
    /// state), then the home's: publication compiling its project.
    CorpusFirst(&'a Corpus),
}

impl Fragments<'_> {
    fn home(&self) -> &Home {
        match self {
            Fragments::Home(home) => home,
            Fragments::CorpusFirst(corpus) => corpus.home(),
        }
    }

    /// The body of the fragment `include_path` (`docs/snippets/<name>.md`)
    /// names; `None` when no fragment of that name exists.
    fn body(&self, include_path: &str) -> Result<Option<String>, DocsError> {
        if let Fragments::CorpusFirst(corpus) = self
            && let Some(body) = corpus.fragment(include_path)
        {
            return Ok(Some(body.to_owned()));
        }
        let Some(name) = include_path
            .strip_prefix("docs/snippets/")
            .filter(|name| name.ends_with(".md"))
        else {
            return Ok(None);
        };
        let root = self.home().fragments_dir();
        let outside = |path| DocsError::OutsideHome { path };
        let Some(text) = read_confined(&root, Path::new(name), outside)? else {
            return Ok(None);
        };
        Ok(Some(_text::split_frontmatter(&text).body.to_owned()))
    }
}

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
    /// Where `@include` fragments come from; also the home whose user
    /// directory `~/` references expand to.
    pub fragments: Fragments<'a>,
    /// How `@config` directives resolve.
    pub values: &'a Values<'a>,
    /// The root of the repository the text belongs to, against which
    /// `docs/…` references resolve; `None` for text read from the home,
    /// whose relative references stay as written.
    pub repo_root: Option<&'a Path>,
    /// The slug that repository declares, which local required reads name.
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
    let lensed = include_and_lens(body, lens, cx)?;
    let required_reads = section_reads(&lensed, cx);
    let (resolved, config_refusals) = resolve_values(&lensed, cx.values)?;
    // The section is stripped before references expand, which leaves the
    // text the same.
    let stripped = strip_required_reads(&resolved);
    let text = expand_references(&stripped, cx);
    Ok(Rendered {
        text,
        required_reads,
        config_refusals,
    })
}

/// The required reads a rendering of `body` through `lens` reports, read
/// without resolving any configured value: fragments are included and the
/// lens applied first, so a fragment's required reads count and hidden
/// ones do not. A value is fenced and never adds a reference, so this is
/// exactly [`Rendered::required_reads`].
pub(super) fn dependency_reads(
    body: &str,
    lens: Lens<'_>,
    cx: &RenderContext<'_>,
) -> Result<Vec<Reference>, DocsError> {
    Ok(section_reads(&include_and_lens(body, lens, cx)?, cx))
}

fn include_and_lens(
    body: &str,
    lens: Lens<'_>,
    cx: &RenderContext<'_>,
) -> Result<String, DocsError> {
    let included = expand_includes(body, &cx.fragments)?;
    let scan = _markers::scan(&included);
    Ok(_markers::resolve(&included, &scan, |block| match lens {
        Lens::Target(slug) if block.slug == slug.as_str() => Side::Delivered,
        _ => Side::Current,
    }))
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

fn expand_includes(body: &str, fragments: &Fragments<'_>) -> Result<String, DocsError> {
    let mut replacements = Vec::new();
    for directive in _text::directives(&_text::lines(body)) {
        let Directive::Include { path, span } = directive else {
            continue;
        };
        if let Some(fragment) = fragments.body(&path)? {
            replacements.push((span, fragment.trim_matches(['\n', '\r']).to_owned()));
        }
    }
    Ok(splice(body, replacements))
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

/// Every `@<path>.md` reference outside code and comments, as an absolute
/// path: `~/` expands to the user's home directory, a relative path
/// resolves against the repository root when the text has one.
fn expand_references(body: &str, cx: &RenderContext<'_>) -> String {
    let user_home = cx.fragments.home().user_home();
    let replacements = _text::references(&_text::lines(body))
        .into_iter()
        .filter(|r| r.target.ends_with(".md") && !r.target.contains("://"))
        .filter_map(|r| {
            let absolute = if let Some(rest) = r.target.strip_prefix("~/") {
                user_home.join(rest)
            } else if Path::new(&r.target).is_absolute() {
                return None;
            } else {
                cx.repo_root?.join(&r.target)
            };
            Some((r.span, format!("@{}", absolute.display())))
        })
        .collect();
    splice(body, replacements)
}

/// The references of `body`'s `## Required reads` section.
fn section_reads(body: &str, cx: &RenderContext<'_>) -> Vec<Reference> {
    let lines = _text::lines(body);
    let Some(section) = required_reads_section(&lines) else {
        return Vec::new();
    };
    let user_home = cx.fragments.home().user_home();
    _text::references(&lines)
        .into_iter()
        .filter(|r| section.contains(&r.line))
        .map(|r| _refs::parse(&r.target, cx.project, user_home))
        .collect()
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
