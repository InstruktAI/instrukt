//! Company home adoption (`project/instrukt/spec/domain/repository`,
//! Company home adoption): `instrukt init --company <key>` gives a company
//! its home on this computer as the checkout of the company's repository,
//! creating the repository first when configuration's decision says so
//! (`project/instrukt/spec/feature/configuration/company-homes`,
//! Generation).
//!
//! Adoptions take turns under one lock. A creation checks its
//! prerequisites, then records its intent before the root commit is
//! composed and before anything on GitHub is changed; an adoption that
//! finds the intent resumes that creation. The root commit is composed in a
//! staging checkout, and the home's clone is made ready beside it, moved
//! into place whole and then synced.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{self, Configuration, Env, Generation};
use crate::fsx::{LockGuard, atomic_write};
use crate::git;
use crate::home::Home;
use crate::ids::{GitHubLogin, GitHubOrganisation, ProjectSlug};
use crate::interfaces::{AccountKind, CompanyRepositories, CoreWiring, Creation, GitHubAccounts};
use crate::publication::{self, CompanyName, RepositoryView};

use super::_error::RepositoryError;
use super::_hooks::{self, HooksReport};

/// How long an adoption waits for another adoption on this computer, which
/// holds the lock across a few GitHub calls, a push and a clone.
const GENERATION_LOCK_TIMEOUT: Duration = Duration::from_secs(600);

/// The default branch of a company's repository.
const MAIN: &str = "main";

/// What a company's home that is in place but unpublished needs: its
/// recovery, as adoption's diagnostics state it.
const NOT_PUBLISHED: &str =
    "the company's home is in place and not published; `instrukt sync` in it publishes it";

/// What the caller asks of `instrukt init --company`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyRequest {
    /// The company's key.
    pub key: ProjectSlug,
    /// The name people write the company by; needed only to create.
    pub name: Option<CompanyName>,
    /// The company's GitHub administrator; needed only to create.
    pub admin: Option<GitHubLogin>,
    /// The caller's declaration of human involvement for a created
    /// repository's root commit; `None` when it declared nothing.
    pub human_involved: Option<bool>,
}

/// What an adoption reaches outside the repository context: work's GitHub
/// boundary, the caller's environment and the optional-module wiring the
/// root commit and the sync read.
pub struct CompanyPorts<'a> {
    pub accounts: &'a dyn GitHubAccounts,
    pub repositories: &'a dyn CompanyRepositories,
    pub env: &'a Env,
    pub wiring: &'a CoreWiring,
    pub now: jiff::Timestamp,
}

/// How the company's repository came to be the home's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompanyOrigin {
    /// This adoption created it with the skeleton.
    Created,
    /// It existed: the company's by contract, nothing on GitHub changed.
    ByContract,
}

/// What the home's sync did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompanySync {
    Published,
    Unchanged,
    /// The sync refused or failed: why. The home stays.
    NotPublished(String),
}

/// What `instrukt init --company` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanyReport {
    pub origin: CompanyOrigin,
    pub organisation: GitHubOrganisation,
    pub repository: ProjectSlug,
    pub home: PathBuf,
    /// A creation cut short earlier was completed from its intent.
    pub resumed: bool,
    /// The flags given that this adoption did not use.
    pub unused: Vec<&'static str>,
    /// Whether the settings document was written; `false` when the
    /// repository already held one.
    pub settings_written: bool,
    pub hooks: HooksReport,
    pub sync: CompanySync,
}

/// The creation intent: what a creation under way needs to complete,
/// recorded once its prerequisites are checked, before the root commit is
/// composed and before anything on GitHub is changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Intent {
    organisation: GitHubOrganisation,
    repository: ProjectSlug,
    name: String,
    admin: GitHubLogin,
    /// The caller's declaration for the root commit.
    human_involved: bool,
}

/// What pushing the root commit found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pushed {
    /// The repository took the root commit.
    Accepted,
    /// The repository already holds the root commit as its `main`.
    AlreadyThere,
    /// The repository holds other history: it is someone's by contract.
    OtherHistory,
}

/// What a creation does next, from whether it resumes an intent, what
/// GitHub answered to creating the repository and, once pushed, what the
/// push found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Push,
    /// Set the default branch and make the administrator.
    Finish,
    /// Check the repository out by contract, nothing more on GitHub.
    ByContract,
}

fn next_step(resuming: bool, created: Creation, pushed: Option<Pushed>) -> Step {
    match (created, pushed) {
        // Without an intent, an existing repository is someone's: nothing
        // is pushed to it.
        (Creation::AlreadyExists, None) if !resuming => Step::ByContract,
        // With one, it may be this computer's own earlier creation: the push
        // decides.
        (_, None) => Step::Push,
        (_, Some(Pushed::Accepted | Pushed::AlreadyThere)) => Step::Finish,
        (_, Some(Pushed::OtherHistory)) => Step::ByContract,
    }
}

/// Adopts the home of the company `request.key`.
pub fn adopt_company(
    home: &Home,
    request: &CompanyRequest,
    ports: &CompanyPorts<'_>,
) -> Result<CompanyReport, RepositoryError> {
    let key = &request.key;
    let generation = home.state_dir().join("company-generation");
    crate::fsx::create_dir_all(&generation).map_err(|source| RepositoryError::Create {
        path: generation.clone(),
        source,
    })?;
    let _lock = LockGuard::acquire(
        &home.state_dir().join("company-generation.lock"),
        GENERATION_LOCK_TIMEOUT,
    )?;
    let home_dir = home.company_dir(key);
    let home_exists = present(&home_dir)?;
    config::admit_home(&home_dir, home_exists)?;
    let staging = generation.join(key.as_str());
    let intent_path = staging.join("intent.yaml");
    // Read at entry: the run writes the intent itself.
    let resuming = present(&intent_path)?;
    // Configuration names the repository; adoption consumes that name.
    let named = config::company_repository(key)?;
    let global = Configuration::load_global(home)?;
    let host = crate::github::api_host(global.github_api_url()?);
    let mut unused = Vec::new();
    let (organisation, repository, intent) = if resuming {
        let intent = read_intent(&intent_path)?;
        unused.extend(given_flags(request));
        (
            intent.organisation.clone(),
            intent.repository.clone(),
            Some(intent),
        )
    } else {
        let declared = global
            .github_org()?
            .map(str::trim)
            .filter(|org| !org.is_empty());
        let kind = declared
            .map(|org| ports.accounts.account_kind(org))
            .transpose()?;
        let reachable = match (declared.map(GitHubOrganisation::parse), kind) {
            (Some(Ok(org)), Some(AccountKind::Organisation)) => {
                reachable(&staging_parent(&staging), &url(&host, &org, &named))?
            }
            _ => false,
        };
        match config::decide_generation(&named, &home_dir, home_exists, declared, kind, reachable)?
        {
            Generation::CheckOut {
                organisation,
                repository,
            } => {
                unused.extend(given_flags(request));
                (organisation, repository, None)
            }
            Generation::Create {
                organisation,
                repository,
            } => {
                let name = request
                    .name
                    .as_ref()
                    .ok_or(RepositoryError::CompanyFlagMissing { flag: "--name" })?;
                let admin = request
                    .admin
                    .as_ref()
                    .ok_or(RepositoryError::CompanyFlagMissing { flag: "--admin" })?;
                let admin_kind = ports.accounts.account_kind(admin.as_str())?;
                if admin_kind != AccountKind::Personal {
                    return Err(RepositoryError::CompanyAdminNotPersonal {
                        login: admin.to_string(),
                        kind: admin_kind,
                    });
                }
                ports.repositories.ready(&organisation, ports.now)?;
                let intent = Intent {
                    organisation: organisation.clone(),
                    repository: repository.clone(),
                    name: name.as_str().to_owned(),
                    admin: admin.clone(),
                    human_involved: request.human_involved.unwrap_or(false),
                };
                write_intent(&intent_path, &intent)?;
                (organisation, repository, Some(intent))
            }
        }
    };
    let remote = url(&host, &organisation, &repository);
    let origin = match &intent {
        Some(intent) => create(home, key, &staging, &remote, intent, resuming, ports)?,
        None => CompanyOrigin::ByContract,
    };
    let (settings_written, hooks) = check_out(home, key, &staging, &remote, &home_dir)?;
    let sync = match publication::sync(&publication::SyncRequest {
        home,
        cwd: &home_dir,
        wiring: ports.wiring,
        now: ports.now,
    }) {
        Ok(report) => match report.outcome {
            publication::SyncOutcome::Published { .. } => CompanySync::Published,
            publication::SyncOutcome::Unchanged { .. } => CompanySync::Unchanged,
            publication::SyncOutcome::Refused { refusal, .. } => {
                tracing::warn!(
                    event = "company_home_publication_refused",
                    company = %key,
                    reason = %refusal,
                    "{}", NOT_PUBLISHED
                );
                CompanySync::NotPublished(refusal.to_string())
            }
        },
        Err(error) => {
            tracing::warn!(
                event = "company_home_publication_failed",
                company = %key,
                error = %error,
                "{}", NOT_PUBLISHED
            );
            CompanySync::NotPublished(error.to_string())
        }
    };
    tracing::info!(
        event = "company_home_adopted",
        company = %key,
        organisation = %organisation,
        repository = %repository,
        created = origin == CompanyOrigin::Created,
        resumed = resuming,
    );
    Ok(CompanyReport {
        origin,
        organisation,
        repository,
        home: home_dir,
        resumed: resuming,
        unused,
        settings_written,
        hooks,
        sync,
    })
}

/// The folder git is run from to read a remote before the staging checkout
/// exists: the folder that holds it.
fn staging_parent(staging: &Path) -> PathBuf {
    staging
        .parent()
        .map_or_else(|| staging.to_path_buf(), Path::to_path_buf)
}

/// The flags of `request` an adoption that creates nothing leaves unused.
fn given_flags(request: &CompanyRequest) -> Vec<&'static str> {
    [
        (request.name.is_some(), "--name"),
        (request.admin.is_some(), "--admin"),
        (request.human_involved.is_some(), "--human-involved"),
    ]
    .into_iter()
    .filter_map(|(given, flag)| given.then_some(flag))
    .collect()
}

/// The company repository's address for git, on the GitHub host the API
/// address names; git reaches it with the computer's own credentials.
fn url(host: &str, organisation: &GitHubOrganisation, repository: &ProjectSlug) -> String {
    format!("https://{host}/{organisation}/{repository}.git")
}

/// Completes the creation `intent` names: composes the root commit (or
/// reuses the one staging holds), has GitHub create the repository, pushes
/// the root commit without force, then sets the default branch and makes
/// the administrator; or finds the repository someone's by contract and
/// removes the staging checkout.
fn create(
    home: &Home,
    key: &ProjectSlug,
    staging: &Path,
    remote: &str,
    intent: &Intent,
    resuming: bool,
    ports: &CompanyPorts<'_>,
) -> Result<CompanyOrigin, RepositoryError> {
    let skeleton = staging.join("skeleton");
    let root = compose_root(home, key, &skeleton, intent, ports)?;
    let (org, repo) = (&intent.organisation, &intent.repository);
    let created = ports.repositories.create(org, repo, ports.now)?;
    let mut step = next_step(resuming, created, None);
    if step == Step::Push {
        step = next_step(
            resuming,
            created,
            Some(push_root(&skeleton, remote, &root)?),
        );
    }
    match step {
        Step::Finish => {
            ports
                .repositories
                .set_default_branch(org, repo, MAIN, ports.now)?;
            ports
                .repositories
                .grant_admin(org, repo, &intent.admin, ports.now)?;
            Ok(CompanyOrigin::Created)
        }
        Step::ByContract | Step::Push => {
            remove(staging)?;
            Ok(CompanyOrigin::ByContract)
        }
    }
}

/// The root commit of the company skeleton in `skeleton`: the one already
/// there, or one composed from the skeleton configuration and publication
/// supply and committed attributed.
fn compose_root(
    home: &Home,
    key: &ProjectSlug,
    skeleton: &Path,
    intent: &Intent,
    ports: &CompanyPorts<'_>,
) -> Result<String, RepositoryError> {
    if let Some(head) = head(skeleton)? {
        return Ok(head);
    }
    remove(skeleton)?;
    crate::fsx::create_dir_all(skeleton).map_err(|source| RepositoryError::Create {
        path: skeleton.to_path_buf(),
        source,
    })?;
    git::run(skeleton, ["init", "--quiet", "--initial-branch", MAIN])?;
    config::write_skeleton(skeleton, key)?;
    publication::refresh_editor_schemas(home.root(), skeleton)?;
    // The skeleton's knowledge index selects nothing yet.
    atomic_write(
        &skeleton.join("docs/index.yaml"),
        b"expanded: []\nlisted: []\n",
    )?;
    let rendering = publication::render_repository(RepositoryView::WorkingTree(skeleton))?;
    if !rendering.findings.is_empty() {
        return Err(RepositoryError::Skeleton {
            findings: rendering
                .findings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        });
    }
    publication::write_rendering(skeleton, &rendering)?;
    let name = CompanyName::parse(&intent.name).map_err(|error| RepositoryError::Skeleton {
        findings: error.to_string(),
    })?;
    let plugin = publication::plan_plugin(skeleton, key, &name)?;
    if !plugin.findings.is_empty() {
        return Err(RepositoryError::Skeleton {
            findings: plugin
                .findings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        });
    }
    publication::write_plugin(skeleton, &plugin)?;
    let sha = super::_commit::root_commit(
        home,
        skeleton,
        ports.env,
        ports.wiring,
        "feat: start the company's repository\n\nThe company skeleton every company receives: the project layer, the ignore\nfile, the knowledge index, the AGENTS.md rendered from it, and the\ncompany's skill with the catalog that lists it.",
        intent.human_involved,
    )?;
    Ok(sha.to_string())
}

/// Pushes `root` as `main` of `remote` without force, and what the remote
/// then holds decides: the root commit, or other history.
fn push_root(skeleton: &Path, remote: &str, root: &str) -> Result<Pushed, RepositoryError> {
    let judged = |main: Option<String>| match main {
        Some(main) if main == root => Some(Pushed::AlreadyThere),
        Some(_) => Some(Pushed::OtherHistory),
        None => None,
    };
    if let Some(found) = judged(remote_main(skeleton, remote)?) {
        return Ok(found);
    }
    match git::run_remote(
        skeleton,
        ["push", "--quiet", remote, "main:refs/heads/main"],
    ) {
        Ok(_) => Ok(Pushed::Accepted),
        // A push another writer raced: what the remote holds now decides.
        Err(error) => judged(remote_main(skeleton, remote)?).ok_or(error.into()),
    }
}

/// The commit `main` of `remote` names; `None` when it has no `main`.
fn remote_main(dir: &Path, remote: &str) -> Result<Option<String>, RepositoryError> {
    let listing = git::run_remote(dir, ["ls-remote", remote, "refs/heads/main"])?;
    Ok(listing.split_whitespace().next().map(ToOwned::to_owned))
}

/// Whether git reaches `remote` with the computer's own credentials.
fn reachable(dir: &Path, remote: &str) -> Result<bool, RepositoryError> {
    let output = git::run_output_remote(dir, ["ls-remote", "--exit-code", remote, "HEAD"])?;
    // `--exit-code` answers 2 for a repository holding no matching ref: an
    // empty repository is reached.
    Ok(matches!(output.status.code(), Some(0 | 2)))
}

/// Clones `remote` into the staging checkout (completing a clone already
/// there), gives it its settings document and hook sections, moves it into
/// place as `home_dir` and removes the staging checkout.
fn check_out(
    home: &Home,
    key: &ProjectSlug,
    staging: &Path,
    remote: &str,
    home_dir: &Path,
) -> Result<(bool, HooksReport), RepositoryError> {
    let clone = staging.join("home");
    if head(&clone)?.is_none() {
        remove(&clone)?;
        crate::fsx::create_dir_all(staging).map_err(|source| RepositoryError::Create {
            path: staging.to_path_buf(),
            source,
        })?;
        git::run_remote(staging, ["clone", "--quiet", remote, "home"])?;
    }
    let settings_written = config::write_company_settings(&clone, key)?;
    let mut hooks = _hooks::install_at(home, &clone)?;
    if let Some(companies) = home_dir.parent() {
        crate::fsx::create_dir_all(companies).map_err(|source| RepositoryError::Create {
            path: companies.to_path_buf(),
            source,
        })?;
    }
    crate::fsx::rename(&clone, home_dir).map_err(|source| RepositoryError::Create {
        path: home_dir.to_path_buf(),
        source,
    })?;
    remove(staging)?;
    // The sections were installed before the move: they are reported where
    // they now stand.
    let moved = |path: &Path| match path.strip_prefix(&clone) {
        Ok(rest) => home_dir.join(rest),
        Err(_) => path.to_path_buf(),
    };
    hooks.dir.path = moved(&hooks.dir.path);
    for hook in &mut hooks.hooks {
        hook.file = moved(&hook.file);
    }
    Ok((settings_written, hooks))
}

/// The commit `HEAD` names in the repository at `dir`; `None` when `dir`
/// holds no repository with a commit.
fn head(dir: &Path) -> Result<Option<String>, RepositoryError> {
    if !present(&dir.join(".git"))? {
        return Ok(None);
    }
    let output = git::run_output(dir, ["rev-parse", "--verify", "--quiet", "HEAD"])?;
    Ok(output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned()))
}

fn present(path: &Path) -> Result<bool, RepositoryError> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(RepositoryError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn remove(dir: &Path) -> Result<(), RepositoryError> {
    match crate::fsx::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(RepositoryError::Create {
            path: dir.to_path_buf(),
            source,
        }),
    }
}

fn read_intent(path: &Path) -> Result<Intent, RepositoryError> {
    let text = std::fs::read_to_string(path).map_err(|source| RepositoryError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_yaml_ng::from_str(&text).map_err(|error| RepositoryError::CompanyIntent {
        path: path.to_path_buf(),
        reason: error.to_string(),
    })
}

/// Writes `intent` at `path` whole and atomically.
fn write_intent(path: &Path, intent: &Intent) -> Result<(), RepositoryError> {
    let text =
        serde_yaml_ng::to_string(intent).map_err(|error| RepositoryError::CompanyIntent {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })?;
    atomic_write(path, text.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        Creation, GitHubLogin, GitHubOrganisation, Intent, ProjectSlug, Pushed, RepositoryError,
        Step, next_step, read_intent, write_intent,
    };

    #[test]
    fn a_fresh_creation_that_finds_the_repository_takes_it_by_contract_and_pushes_nothing() {
        assert_eq!(
            next_step(false, Creation::AlreadyExists, None),
            Step::ByContract
        );
        assert_eq!(next_step(false, Creation::Created, None), Step::Push);
    }

    #[test]
    fn a_resumed_creation_lets_the_push_decide_whatever_creation_answered() {
        for created in [Creation::Created, Creation::AlreadyExists] {
            assert_eq!(next_step(true, created, None), Step::Push);
            for pushed in [Pushed::Accepted, Pushed::AlreadyThere] {
                assert_eq!(next_step(true, created, Some(pushed)), Step::Finish);
            }
            assert_eq!(
                next_step(true, created, Some(Pushed::OtherHistory)),
                Step::ByContract
            );
        }
    }

    #[test]
    fn a_push_the_repository_takes_finishes_a_fresh_creation() {
        assert_eq!(
            next_step(false, Creation::Created, Some(Pushed::Accepted)),
            Step::Finish
        );
        assert_eq!(
            next_step(false, Creation::Created, Some(Pushed::OtherHistory)),
            Step::ByContract
        );
    }

    #[test]
    fn an_intent_is_read_back_as_it_was_written_and_nothing_else_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("intent.yaml");
        let intent = Intent {
            organisation: GitHubOrganisation::parse("homes").unwrap(),
            repository: ProjectSlug::parse("company-acme").unwrap(),
            name: "Acme".into(),
            admin: GitHubLogin::parse("acme-admin").unwrap(),
            human_involved: true,
        };
        write_intent(&path, &intent).unwrap();
        assert_eq!(read_intent(&path).unwrap(), intent);
        std::fs::write(&path, "organisation: homes\n").unwrap();
        assert!(matches!(
            read_intent(&path),
            Err(RepositoryError::CompanyIntent { .. })
        ));
    }
}
