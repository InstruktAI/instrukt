//! Rendering: the read pipeline's text steps, shared by `docs get` and by
//! publication's compilation.
//!
//! One rendering takes a text through, in order: the frontmatter strip, the
//! `@include` expansion, the planned lens, the `@exec` read-time commands,
//! the inline references and the `Required reads` strip
//! (`local/projects/instrukt/spec/feature/docs/progressive-retrieval`, read
//! pipeline). The frontmatter goes first so nothing in it ever runs; the
//! lens goes before `@exec` so hidden content never runs.
//!
//! The two callers differ in where fragments come from and in what a
//! read-time command runs with and what its failure means; both are the
//! [`RenderContext`].

use std::io::Read as _;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use super::_corpus::{Corpus, Lens, Version, required_reads_section};
use super::_markers::{self, Side};
use super::_refs::{self, Reference};
use super::_text::{self, Directive};
use super::DocsError;
use crate::config::RuntimeEntry;
use crate::home::Home;
use crate::ids::ProjectSlug;

/// How long one read-time command may run, at read and at publish.
pub const EXEC_TIME_LIMIT: Duration = Duration::from_secs(10);

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

/// What a read-time command's failure does to the rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OnFailure {
    /// The directive becomes an inline notice; the rendering reports it.
    Notice,
    /// The rendering fails.
    Fail,
}

/// How `@exec` read-time commands run: in which directory, with which
/// environment, and what a failure means. Every command runs through
/// `/bin/sh -c`, is given [`EXEC_TIME_LIMIT`], and is never cached.
#[derive(Debug, Clone)]
pub struct ExecPolicy {
    cwd: PathBuf,
    removed_env: Vec<String>,
    on_failure: OnFailure,
}

impl ExecPolicy {
    /// At read: the caller's working directory and environment. A failure
    /// or the time limit becomes an inline notice naming the command, and
    /// the rendering reports it in [`Rendered::exec_failures`].
    pub fn read(cwd: &Path) -> Self {
        Self {
            cwd: cwd.to_path_buf(),
            removed_env: Vec::new(),
            on_failure: OnFailure::Notice,
        }
    }

    /// At publish: the project root as working directory, and the
    /// environment without the session variable of any runtime in the
    /// mapping, so no session's value is compiled into shared text. A
    /// failure or the time limit fails the rendering
    /// ([`DocsError::ExecFailed`]).
    pub fn publish(project_root: &Path, runtimes: &[RuntimeEntry]) -> Self {
        Self {
            cwd: project_root.to_path_buf(),
            removed_env: runtimes.iter().map(|r| r.session_id_env.clone()).collect(),
            on_failure: OnFailure::Fail,
        }
    }
}

/// Everything a rendering resolves outside the text itself.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext<'a> {
    /// Where `@include` fragments come from; also the home whose user
    /// directory `~/` references expand to.
    pub fragments: Fragments<'a>,
    /// How `@exec` directives run.
    pub exec: &'a ExecPolicy,
    /// The root of the repository the text belongs to, against which
    /// `docs/…` references resolve; `None` for text read from the home,
    /// whose relative references stay as written.
    pub repo_root: Option<&'a Path>,
    /// The slug that repository declares, which local required reads name.
    pub project: Option<&'a ProjectSlug>,
}

/// Why a read-time command failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecOutcome {
    /// It exited with this non-zero status.
    Exited(i32),
    /// A signal ended it.
    Signalled,
    /// It was still running, or still holding its output open, when the
    /// time limit passed; it was killed.
    TimedOut,
}

impl std::fmt::Display for ExecOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecOutcome::Exited(code) => write!(f, "exit status {code}"),
            ExecOutcome::Signalled => f.write_str("ended by a signal"),
            ExecOutcome::TimedOut => {
                write!(f, "time limit of {} s reached", EXEC_TIME_LIMIT.as_secs())
            }
        }
    }
}

/// One read-time command that failed during a rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecFailure {
    pub command: String,
    pub outcome: ExecOutcome,
}

/// A rendered text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    /// The text after every step.
    pub text: String,
    /// The references of its `## Required reads` section, read after the
    /// lens and before the section was stripped.
    pub required_reads: Vec<Reference>,
    /// The read-time commands that failed, in order; always empty under
    /// [`ExecPolicy::publish`], where a failure is an error.
    pub exec_failures: Vec<ExecFailure>,
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
    let included = expand_includes(body, &cx.fragments)?;
    let scan = _markers::scan(&included);
    let lensed = _markers::resolve(&included, &scan, |block| match lens {
        Lens::Target(slug) if block.slug == slug.as_str() => Side::Delivered,
        _ => Side::Current,
    });
    let (executed, exec_failures) = run_execs(&lensed, cx.exec)?;
    // The section is stripped before references expand, which leaves the
    // text the same and its references as written.
    let (stripped, required_reads) = strip_required_reads(&executed, cx);
    let text = expand_references(&stripped, cx);
    Ok(Rendered {
        text,
        required_reads,
        exec_failures,
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

fn run_execs(body: &str, policy: &ExecPolicy) -> Result<(String, Vec<ExecFailure>), DocsError> {
    let mut replacements = Vec::new();
    let mut failures = Vec::new();
    for directive in _text::directives(&_text::lines(body)) {
        let Directive::Exec { command, span } = directive else {
            continue;
        };
        match run_command(&command, policy)? {
            Ok(stdout) => replacements.push((span, fenced(&stdout))),
            Err((outcome, stderr)) if policy.on_failure == OnFailure::Fail => {
                return Err(DocsError::ExecFailed {
                    command,
                    outcome,
                    stderr,
                });
            }
            Err((outcome, _)) => {
                replacements.push((
                    span,
                    format!("> instrukt: read-time command `{command}` failed: {outcome}"),
                ));
                failures.push(ExecFailure { command, outcome });
            }
        }
    }
    Ok((splice(body, replacements), failures))
}

/// Non-empty output as a fenced block whose fence is longer than any
/// backtick run inside it; nothing for empty output.
fn fenced(stdout: &str) -> String {
    let output = stdout.trim_end_matches(['\n', '\r']);
    if output.trim().is_empty() {
        return String::new();
    }
    let longest = output.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    let fence = "`".repeat(longest.max(2) + 1);
    format!("{fence}\n{output}\n{fence}")
}

/// Runs one command; its standard output, or why it failed with what it
/// wrote to standard error. Spawning or waiting failing at all is an
/// error.
fn run_command(
    command: &str,
    policy: &ExecPolicy,
) -> Result<Result<String, (ExecOutcome, String)>, DocsError> {
    let started = Instant::now();
    let deadline = started + EXEC_TIME_LIMIT;
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(command)
        .current_dir(&policy.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in &policy.removed_env {
        cmd.env_remove(name);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        // Its own process group, so the time limit ends everything it
        // started.
        cmd.process_group(0);
    }
    let exec_error = |source| DocsError::Exec {
        command: command.to_owned(),
        source,
    };
    let mut child = cmd.spawn().map_err(exec_error)?;
    let (sender, outputs) = mpsc::channel();
    for (stream, pipe) in [
        (Stream::Stdout, child.stdout.take().map(PipeReader::Out)),
        (Stream::Stderr, child.stderr.take().map(PipeReader::Err)),
    ] {
        let sender = sender.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let read = match pipe {
                Some(mut pipe) => pipe.read_to_end(&mut bytes).map(|_| ()),
                None => Ok(()),
            };
            // The receiver is gone only when the command was abandoned.
            if sender.send((stream, read.map(|()| bytes))).is_err() {
                tracing::trace!(event = "docs_exec_output_abandoned");
            }
        });
    }
    drop(sender);
    let mut status = wait_until(&mut child, deadline).map_err(exec_error)?;
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    // Both streams must close before the deadline too: a command that
    // leaves something running with its output open has not finished.
    let mut open = 2;
    while status.is_some() && open > 0 {
        let left = deadline.saturating_duration_since(Instant::now());
        match outputs.recv_timeout(left) {
            Ok((Stream::Stdout, read)) => stdout = read.map_err(exec_error)?,
            Ok((Stream::Stderr, read)) => stderr = read.map_err(exec_error)?,
            Err(mpsc::RecvTimeoutError::Timeout) => status = None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        open -= 1;
    }
    let outcome = match status {
        None => {
            kill_group(&mut child);
            Err(ExecOutcome::TimedOut)
        }
        Some(status) if status.success() => Ok(String::from_utf8_lossy(&stdout).into_owned()),
        Some(status) => Err(failed(status)),
    };
    let stderr = String::from_utf8_lossy(&stderr).into_owned();
    tracing::debug!(
        event = "docs_exec_completed",
        command,
        cwd = %policy.cwd.display(),
        outcome = %match &outcome {
            Ok(_) => "success".to_owned(),
            Err(outcome) => outcome.to_string(),
        },
        stderr = %stderr,
        duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    );
    Ok(outcome.map_err(|outcome| (outcome, stderr)))
}

#[derive(Debug, Clone, Copy)]
enum Stream {
    Stdout,
    Stderr,
}

enum PipeReader {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

impl std::io::Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            PipeReader::Out(pipe) => pipe.read(buf),
            PipeReader::Err(pipe) => pipe.read(buf),
        }
    }
}

/// Waits for `child` until `deadline`; `None` when it is still running
/// then.
fn wait_until(
    child: &mut std::process::Child,
    deadline: Instant,
) -> std::io::Result<Option<ExitStatus>> {
    let mut pause = Duration::from_millis(1);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(None);
        }
        std::thread::sleep(pause.min(deadline - now));
        pause = (pause * 2).min(Duration::from_millis(50));
    }
}

/// Ends the command and everything it started, then reaps it.
fn kill_group(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let group = format!("-{}", child.id());
        match Command::new("kill")
            .args(["-KILL", "--", &group])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) if status.success() => {}
            Ok(status) => tracing::warn!(event = "docs_exec_kill_failed", group, %status),
            Err(err) => tracing::warn!(event = "docs_exec_kill_failed", group, error = %err),
        }
    }
    if let Err(err) = child.kill() {
        tracing::debug!(event = "docs_exec_kill_skipped", error = %err);
    }
    if let Err(err) = child.wait() {
        tracing::warn!(event = "docs_exec_reap_failed", error = %err);
    }
}

fn failed(status: ExitStatus) -> ExecOutcome {
    match status.code() {
        Some(code) => ExecOutcome::Exited(code),
        None => ExecOutcome::Signalled,
    }
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

/// `body` without its `## Required reads` section (heading included), and
/// the references the section holds.
fn strip_required_reads(body: &str, cx: &RenderContext<'_>) -> (String, Vec<Reference>) {
    let lines = _text::lines(body);
    let Some(section) = required_reads_section(&lines) else {
        return (body.to_owned(), Vec::new());
    };
    let user_home = cx.fragments.home().user_home();
    let reads = _text::references(&lines)
        .into_iter()
        .filter(|r| section.contains(&r.line))
        .map(|r| _refs::parse(&r.target, cx.project, user_home))
        .collect();
    let from = lines
        .get(section.start.saturating_sub(1))
        .map_or(body.len(), |line| line.span.start);
    let to = lines
        .get(section.end)
        .map_or(body.len(), |line| line.span.start);
    (splice(body, vec![(from..to, String::new())]), reads)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Configuration;
    use crate::docs::SourceFile;
    use crate::ids::TodoSlug;

    const HEAD: &str = "\n# Work — Procedure\n\n";
    const READ: &str = "~/.instrukt/docs/acme/policy/rules.md";
    const LOCAL_REF: &str = "docs/local/spec/a.md";
    const FRAGMENT_NAME: &str = "shared";
    const FRAGMENT: &str = "Shared text.\n";
    const VALUE: &str = "computed-value";
    const MARKER_FILE: &str = "hidden-ran";
    const SLUG: &str = "add-it";

    fn source() -> String {
        format!(
            "---\ndescription: d\n---\n{HEAD}## Required reads\n\n- @{READ}\n\n## Goal\n\n\
             See @{LOCAL_REF} here.\n<!-- @include: docs/snippets/{FRAGMENT_NAME}.md -->\n\
             <!-- @exec: printf %s {VALUE} -->\n<!-- planned:{SLUG} -->\n\
             <!-- @exec: touch {MARKER_FILE} -->\n<!-- /planned:{SLUG} -->\n"
        )
    }

    struct Fixture {
        _user_home: tempfile::TempDir,
        repo: tempfile::TempDir,
        home: Home,
        project: ProjectSlug,
    }

    fn fixture() -> Fixture {
        let user_home = tempfile::tempdir().unwrap();
        let repo = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(user_home.path().canonicalize().unwrap());
        Fixture {
            _user_home: user_home,
            repo,
            home,
            project: ProjectSlug::parse("demo").unwrap(),
        }
    }

    fn corpus(f: &Fixture) -> Corpus {
        Corpus::from_sources(
            Some(f.project.clone()),
            &f.home,
            [SourceFile {
                path: format!("docs/global/snippets/{FRAGMENT_NAME}.md"),
                text: FRAGMENT.to_owned(),
            }],
        )
    }

    fn at_read<'a>(
        fragments: Fragments<'a>,
        exec: &'a ExecPolicy,
        repo_root: Option<&'a Path>,
    ) -> RenderContext<'a> {
        RenderContext {
            fragments,
            exec,
            repo_root,
            project: None,
        }
    }

    #[test]
    fn every_step_runs_in_order_and_hidden_content_never_executes() {
        let f = fixture();
        let corpus = corpus(&f);
        let exec = ExecPolicy::read(f.repo.path());
        let cx = RenderContext {
            project: Some(&f.project),
            ..at_read(Fragments::CorpusFirst(&corpus), &exec, Some(f.repo.path()))
        };
        let rendered = render(&source(), Lens::Current, &cx).unwrap();
        assert!(!f.repo.path().join(MARKER_FILE).exists());
        assert!(rendered.exec_failures.is_empty());

        // Frontmatter and the Required reads section are gone, the fragment
        // stands where its directive stood, the reference is absolute and
        // the command's output is fenced in place of its directive.
        let got: Vec<&str> = rendered.text.lines().collect();
        let mut expected: Vec<String> = HEAD.lines().map(str::to_owned).collect();
        expected.push("## Goal".to_owned());
        expected.push(String::new());
        expected.push(format!(
            "See @{} here.",
            f.repo.path().join(LOCAL_REF).display()
        ));
        expected.push(FRAGMENT.trim_end().to_owned());
        assert_eq!(got[..expected.len()], expected[..]);
        let fence = got[expected.len()];
        assert!(fence.len() >= 3 && fence.chars().all(|c| c == '`'));
        assert_eq!(got[expected.len() + 1..], [VALUE, fence]);

        let reads: Vec<&str> = rendered
            .required_reads
            .iter()
            .map(|r| r.raw.as_str())
            .collect();
        assert_eq!(reads, [READ]);

        let slug = TodoSlug::parse(SLUG).unwrap();
        let targeted = render(&source(), Lens::Target(&slug), &cx).unwrap();
        assert!(f.repo.path().join(MARKER_FILE).exists());
        assert!(targeted.exec_failures.is_empty());
    }

    #[test]
    fn a_failed_command_is_a_notice_at_read_and_an_error_at_publish() {
        let f = fixture();
        let corpus = corpus(&f);
        let command = "exit 3";
        let text = format!("# T — Spec\n<!-- @exec: {command} -->\n");
        let read = ExecPolicy::read(f.repo.path());
        let cx = at_read(Fragments::CorpusFirst(&corpus), &read, Some(f.repo.path()));
        let rendered = render(&text, Lens::Current, &cx).unwrap();
        assert_eq!(
            rendered.exec_failures,
            [ExecFailure {
                command: command.to_owned(),
                outcome: ExecOutcome::Exited(3),
            }]
        );
        let publish = ExecPolicy::publish(f.repo.path(), &[]);
        let cx = RenderContext {
            exec: &publish,
            ..cx
        };
        assert!(matches!(
            render(&text, Lens::Current, &cx),
            Err(DocsError::ExecFailed {
                outcome: ExecOutcome::Exited(3),
                ..
            })
        ));
    }

    #[test]
    fn publishing_runs_without_the_runtime_session_variables() {
        // HOME stands in for a session variable: every test process has it.
        let f = fixture();
        std::fs::create_dir_all(f.home.root()).unwrap();
        std::fs::write(
            f.home.global_config(),
            "schema_version: 1\nruntimes:\n  - name: cli\n    agent: cli\n    \
             session_id_env: HOME\n    start: cli\n    send: cli send\n    \
             alive: cli alive\n    conversation: cli history\n",
        )
        .unwrap();
        let configuration = Configuration::load_global(&f.home).unwrap();
        let corpus = corpus(&f);
        let text = "<!-- @exec: test -z \"${HOME+set}\" -->\n";
        let publish = ExecPolicy::publish(f.repo.path(), configuration.runtimes());
        let read = ExecPolicy::read(f.repo.path());
        let published = render(
            text,
            Lens::Current,
            &at_read(Fragments::CorpusFirst(&corpus), &publish, None),
        );
        assert!(published.is_ok());
        let at_read = render(
            text,
            Lens::Current,
            &at_read(Fragments::CorpusFirst(&corpus), &read, None),
        )
        .unwrap();
        assert_eq!(at_read.exec_failures.len(), 1);
    }

    #[test]
    fn home_fragments_are_read_inside_the_home_and_a_missing_one_stays_visible() {
        let f = fixture();
        let dir = f.home.fragments_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{FRAGMENT_NAME}.md")), FRAGMENT).unwrap();
        let exec = ExecPolicy::read(f.repo.path());
        let cx = at_read(Fragments::Home(&f.home), &exec, None);
        let found = format!("<!-- @include: docs/snippets/{FRAGMENT_NAME}.md -->\n");
        assert_eq!(render(&found, Lens::Current, &cx).unwrap().text, FRAGMENT);
        let missing = "<!-- @include: docs/snippets/none.md -->\n";
        assert_eq!(render(missing, Lens::Current, &cx).unwrap().text, missing);
        let climbing = "<!-- @include: docs/snippets/../index.md -->\n";
        assert!(matches!(
            render(climbing, Lens::Current, &cx),
            Err(DocsError::OutsideHome { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_fragment_linking_out_of_the_home_is_refused() {
        let f = fixture();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.md"), FRAGMENT).unwrap();
        let dir = f.home.fragments_dir();
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(outside.path().join("secret.md"), dir.join("link.md")).unwrap();
        let exec = ExecPolicy::read(f.repo.path());
        let cx = at_read(Fragments::Home(&f.home), &exec, None);
        assert!(matches!(
            render(
                "<!-- @include: docs/snippets/link.md -->\n",
                Lens::Current,
                &cx
            ),
            Err(DocsError::OutsideHome { .. })
        ));
    }
}
