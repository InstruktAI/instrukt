//! `instrukt repo commit`: an attributed commit — the acting agent as
//! author, the session it ran in and the caller's declaration of human
//! involvement as trailers, and the work item when given. What the
//! environment does not resolve is left out, never guessed. In an adopted
//! project it also takes the author's review-routing choice and, for a
//! no-review choice, has work record the commit's exemption once the commit
//! exists (`project/instrukt/spec/feature/repository/attributed-commit`,
//! Review routing).

use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::config::{Configuration, Env};
use crate::git;
use crate::home::Home;
use crate::ids::{CommitSha, TodoSlug};
use crate::interfaces::{CommitExemption, CoreWiring};

use super::_error::RepositoryError;
use super::_git;

/// The trailer keys instrukt owns, each with the door that supplies its
/// fact. A caller's message holding any of them is refused.
pub const OWNED_TRAILERS: [(&str, &str); 4] = [
    (
        "Session",
        "the active runtime's session variable (instrukt writes it when one is set)",
    ),
    ("Human-Involved", "`--human-involved true|false`"),
    ("Todo", "`--todo <slug>`"),
    ("Planned-Todo", "`--todo <slug>`"),
];

/// What the caller asks to commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRequest {
    /// The caller's message: a subject and an explanatory body.
    pub message: String,
    /// The caller's declaration of whether a human was involved.
    pub human_involved: bool,
    /// The work item the commit belongs to.
    pub todo: Option<TodoSlug>,
    /// Commit exactly these paths as they are in the working tree, leaving
    /// the rest of the index as it is; empty commits what is staged.
    pub only: Vec<PathBuf>,
    /// The author's review-routing choice; required in an adopted project.
    pub routing: Option<ReviewRouting>,
}

/// The either-or of `general/policy/commits`: the commit goes to the
/// independent review of a head that contains it, or it owes none for the
/// stated basis. It is never written as a trailer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewRouting {
    Review,
    Exempt(String),
}

impl ReviewRouting {
    /// The choice the command's `--review` flag and `--exempt <basis>`
    /// option state; none when neither is given, refused when both are.
    pub fn from_options(
        review: bool,
        exempt: Option<String>,
    ) -> Result<Option<Self>, RepositoryError> {
        match (review, exempt) {
            (true, Some(_)) => Err(RepositoryError::RoutingBoth),
            (true, None) => Ok(Some(Self::Review)),
            (false, Some(basis)) => Ok(Some(Self::Exempt(basis))),
            (false, None) => Ok(None),
        }
    }
}

/// What an admitted routing choice asks of the commit once it exists.
enum Admitted<'a> {
    /// Nothing is recorded: a review choice, or no choice outside an
    /// adopted project.
    Nothing,
    /// Work records the commit's exemption with this basis.
    Exemption {
        basis: &'a str,
        port: &'a dyn CommitExemption,
    },
}

/// Admits the routing choice before anything is committed: in an adopted
/// project exactly one choice with a non-empty basis, and a no-review
/// choice only while work's exemption admission is wired; outside one a
/// no-review choice is refused, since no review evidence is kept there.
fn admit_routing<'a>(
    adopted: bool,
    routing: Option<&'a ReviewRouting>,
    port: Option<&'a dyn CommitExemption>,
) -> Result<Admitted<'a>, RepositoryError> {
    match (adopted, routing) {
        (_, Some(ReviewRouting::Review)) | (false, None) => Ok(Admitted::Nothing),
        (false, Some(ReviewRouting::Exempt(_))) => Err(RepositoryError::ExemptOutsideProject),
        (true, None) => Err(RepositoryError::RoutingMissing),
        (true, Some(ReviewRouting::Exempt(basis))) if basis.trim().is_empty() => {
            Err(RepositoryError::RoutingBasisEmpty)
        }
        (true, Some(ReviewRouting::Exempt(basis))) => match port {
            Some(port) => Ok(Admitted::Exemption { basis, port }),
            None => Err(RepositoryError::ExemptionUnavailable),
        },
    }
}

/// The association a commit names with a work item.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TodoTrailer {
    Recorded(TodoSlug),
    Planned(TodoSlug),
}

/// Makes an attributed commit in the repository containing `cwd`, running
/// the project's own hooks as any commit does; returns its id. A message
/// carrying an owned trailer, and a routing choice [`admit_routing`]
/// refuses, are refused before anything is committed. A no-review choice is
/// recorded as the commit's exemption once the commit exists; when that
/// fails the commit stands and [`RepositoryError::ExemptionNotRecorded`]
/// names it with the door.
pub fn commit(
    home: &Home,
    cwd: &Path,
    env: &Env,
    wiring: &CoreWiring,
    request: &CommitRequest,
) -> Result<CommitSha, RepositoryError> {
    let started = Instant::now();
    refuse_owned_trailers(&request.message)?;
    let root = _git::root(cwd)?;
    if git::run(&root, ["branch", "--show-current"])? == crate::github::default_branch(&root)? {
        return Err(RepositoryError::CanonicalAuthoring);
    }
    let configuration = Configuration::load(home, Some(&root))?;
    let admitted = admit_routing(
        configuration.project()?.is_some(),
        request.routing.as_ref(),
        wiring.commit_exemption.as_deref(),
    )?;
    let author = author(&configuration, env, wiring)?;
    let caller = configuration.caller(env)?;
    let session = caller.as_ref().map(ToString::to_string);
    let todo = match &request.todo {
        Some(slug) => Some(todo_trailer(&configuration, wiring, slug)?),
        None => None,
    };
    let message = compose(
        &request.message,
        session.as_deref(),
        request.human_involved,
        todo.as_ref(),
    );
    let mut args: Vec<String> = vec![
        "commit".into(),
        "--quiet".into(),
        "--cleanup=whitespace".into(),
        "--message".into(),
        message,
    ];
    if let Some(author) = &author {
        args.push(format!("--author={author}"));
    }
    if request.only.is_empty() {
        git::run_with_hooks(cwd, &args)?;
    } else {
        commit_only(cwd, &mut args, &request.only)?;
    }
    let sha = CommitSha::parse(&git::run(cwd, ["rev-parse", "HEAD"])?)?;
    tracing::debug!(
        event = "commit_created",
        sha = %sha,
        agent_author = author.is_some(),
        session = session.is_some(),
        human_involved = request.human_involved,
        only_paths = request.only.len(),
        duration_ms = started.elapsed().as_millis() as u64,
    );
    if let Admitted::Exemption { basis, port } = admitted {
        if let Err(cause) = port.exempt(&sha, basis, caller.as_ref()) {
            tracing::warn!(event = "commit_exemption_not_recorded", sha = %sha, error = %cause);
            return Err(RepositoryError::ExemptionNotRecorded {
                commit: sha,
                basis: basis.to_owned(),
                cause,
            });
        }
        tracing::debug!(event = "commit_exemption_recorded", sha = %sha);
    }
    Ok(sha)
}

/// The author of an attributed commit: the verified bot of the acting
/// session's recorded role, else the acting agent's configured author;
/// `None` when neither resolves, leaving git's own.
fn author(
    configuration: &Configuration,
    env: &Env,
    wiring: &CoreWiring,
) -> Result<Option<String>, RepositoryError> {
    let Some(caller) = configuration.caller(env)? else {
        return agent_author(configuration, env);
    };
    match wiring
        .session_role
        .as_ref()
        .map(|lookup| lookup.role(&caller))
        .transpose()?
        .flatten()
    {
        Some(role) => {
            let actors = wiring.github_actors.as_ref().ok_or_else(|| {
                crate::github::AppError::Setup(
                    "verified actor capability unavailable; configure github.apps".into(),
                )
            })?;
            Ok(Some(actors.actor(role.as_str())?.author()))
        }
        None => agent_author(configuration, env),
    }
}

/// Makes the root commit of the repository at `root` from everything its
/// working tree holds, on the branch it stands on: the one commit instrukt
/// composes on a default branch, because it creates that branch
/// (`project/instrukt/spec/domain/repository`, Company home adoption). It
/// is attributed as [`commit`] attributes, `Human-Involved:` as the caller
/// declares it; the repository has no hooks to run.
pub(super) fn root_commit(
    home: &Home,
    root: &Path,
    env: &Env,
    wiring: &CoreWiring,
    message: &str,
    human_involved: bool,
) -> Result<CommitSha, RepositoryError> {
    refuse_owned_trailers(message)?;
    let configuration = Configuration::load(home, Some(root))?;
    let author = author(&configuration, env, wiring)?;
    let session = session(&configuration, env)?;
    let message = compose(message, session.as_deref(), human_involved, None);
    git::run(root, ["add", "--all"])?;
    let mut args: Vec<String> = vec![
        "commit".into(),
        "--quiet".into(),
        "--cleanup=whitespace".into(),
        "--message".into(),
        message,
    ];
    if let Some(author) = &author {
        args.push(format!("--author={author}"));
    }
    git::run(root, &args)?;
    Ok(CommitSha::parse(&git::run(root, ["rev-parse", "HEAD"])?)?)
}

/// Commits exactly `paths`: untracked ones are made known to the index
/// (intent to add) so `--only` takes them, and made unknown again when the
/// commit does not happen.
fn commit_only(
    cwd: &Path,
    args: &mut Vec<String>,
    paths: &[PathBuf],
) -> Result<(), RepositoryError> {
    let mut listing = vec![
        "ls-files".to_owned(),
        "-z".to_owned(),
        "--others".to_owned(),
        "--exclude-standard".to_owned(),
        "--".to_owned(),
    ];
    listing.extend(paths.iter().map(|path| path.display().to_string()));
    let untracked: Vec<String> = git::run(cwd, &listing)?
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect();
    if !untracked.is_empty() {
        let mut add = vec![
            "add".to_owned(),
            "--intent-to-add".to_owned(),
            "--".to_owned(),
        ];
        add.extend(untracked.iter().cloned());
        git::run(cwd, &add)?;
    }
    args.push("--only".into());
    args.push("--".into());
    args.extend(paths.iter().map(|path| path.display().to_string()));
    let committed = git::run_with_hooks(cwd, &*args);
    if committed.is_err() && !untracked.is_empty() {
        let mut forget = vec![
            "rm".to_owned(),
            "--cached".to_owned(),
            "--quiet".to_owned(),
            "--ignore-unmatch".to_owned(),
            "--".to_owned(),
        ];
        forget.extend(untracked);
        if let Err(err) = git::run(cwd, &forget) {
            tracing::warn!(event = "commit_index_not_restored", error = %err);
        }
    }
    committed?;
    Ok(())
}

/// Refuses a message with a line whose key is a trailer instrukt owns, in
/// any letter case.
fn refuse_owned_trailers(message: &str) -> Result<(), RepositoryError> {
    for (index, line) in message.lines().enumerate() {
        let Some((key, _)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if let Some((owned, door)) = OWNED_TRAILERS
            .iter()
            .find(|(owned, _)| owned.eq_ignore_ascii_case(key))
        {
            return Err(RepositoryError::OwnedTrailer {
                line: index + 1,
                text: line.to_owned(),
                key: owned,
                door,
            });
        }
    }
    Ok(())
}

/// The verified agent's configured git author; `None` when agent identity
/// is unresolved. A known agent without a usable author is refused.
fn agent_author(
    configuration: &Configuration,
    env: &Env,
) -> Result<Option<String>, RepositoryError> {
    let Some(caller) = configuration.caller_identity(env)? else {
        return Ok(None);
    };
    let Some(agent) = caller.agent else {
        return Ok(None);
    };
    if configuration.agent_git_author(&agent)?.is_none() {
        return Err(RepositoryError::AgentAuthorMissing { agent });
    }
    let author = configuration.get_str(&format!("agents.{agent}.git_author"), env)?;
    if !is_name_and_email(&author) {
        return Err(RepositoryError::AgentAuthorMalformed {
            agent,
            value: author,
        });
    }
    Ok(Some(author))
}

/// `Name <email>`: anything else makes git search earlier commits for a
/// matching author, which would be a guess.
fn is_name_and_email(author: &str) -> bool {
    let Some((name, rest)) = author.split_once('<') else {
        return false;
    };
    let Some(email) = rest.strip_suffix('>') else {
        return false;
    };
    !name.trim().is_empty() && !email.trim().is_empty() && !email.contains(['<', '>'])
}

/// `<runtime>:<id>` of the active runtime; `None` when no runtime's session
/// variable is set.
fn session(configuration: &Configuration, env: &Env) -> Result<Option<String>, RepositoryError> {
    Ok(configuration
        .caller(env)?
        .map(|session| session.to_string()))
}

/// The association to write for `slug`: as given while todos is off (or the
/// project has no slug to ask about); with todos on, recorded when the item
/// exists and planned otherwise.
fn todo_trailer(
    configuration: &Configuration,
    wiring: &CoreWiring,
    slug: &TodoSlug,
) -> Result<TodoTrailer, RepositoryError> {
    let (Some(existence), Some(project)) = (&wiring.todo_existence, configuration.project()?)
    else {
        return Ok(TodoTrailer::Recorded(slug.clone()));
    };
    Ok(if existence.todo_exists(&project, slug)? {
        TodoTrailer::Recorded(slug.clone())
    } else {
        TodoTrailer::Planned(slug.clone())
    })
}

/// The caller's message followed, after a blank line, by instrukt's
/// trailers in their fixed order.
fn compose(
    message: &str,
    session: Option<&str>,
    human_involved: bool,
    todo: Option<&TodoTrailer>,
) -> String {
    let mut out = message.trim_end().to_owned();
    out.push_str("\n\n");
    if let Some(session) = session {
        out.push_str(&format!("{}: {session}\n", OWNED_TRAILERS[0].0));
    }
    out.push_str(&format!("{}: {human_involved}\n", OWNED_TRAILERS[1].0));
    match todo {
        Some(TodoTrailer::Recorded(slug)) => {
            out.push_str(&format!("{}: {slug}\n", OWNED_TRAILERS[2].0))
        }
        Some(TodoTrailer::Planned(slug)) => {
            out.push_str(&format!("{}: {slug}\n", OWNED_TRAILERS[3].0))
        }
        None => {}
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_owned_key_is_refused_in_any_letter_case_and_others_pass() {
        assert!(refuse_owned_trailers("subject\n\nbody\nSigned-off-by: a <a@b>").is_ok());
        for (key, _) in OWNED_TRAILERS {
            let message = format!("subject\n\n{}: x", key.to_lowercase());
            match refuse_owned_trailers(&message) {
                Err(RepositoryError::OwnedTrailer {
                    line, key: refused, ..
                }) => {
                    assert_eq!(line, 3);
                    assert_eq!(refused, key);
                }
                other => panic!("{key}: {other:?}"),
            }
        }
    }

    #[test]
    fn only_name_and_email_is_taken_as_an_author() {
        assert!(is_name_and_email("Agent Smith <agent@example.test>"));
        for bad in ["agent", "<a@b>", "Agent <>", "Agent <a@b", "A <a<b>"] {
            assert!(!is_name_and_email(bad), "{bad}");
        }
    }

    #[test]
    fn trailers_follow_a_blank_line_in_their_fixed_order() {
        let slug = TodoSlug::parse("t").unwrap();
        let composed = compose(
            "subject\n\nbody\n\n",
            Some("host:1"),
            true,
            Some(&TodoTrailer::Planned(slug)),
        );
        let lines: Vec<&str> = composed.lines().collect();
        let keys: Vec<&str> = lines[4..]
            .iter()
            .map(|line| line.split_once(':').map_or("", |(key, _)| key))
            .collect();
        assert!(lines[3].is_empty());
        assert_eq!(
            keys,
            [
                OWNED_TRAILERS[0].0,
                OWNED_TRAILERS[1].0,
                OWNED_TRAILERS[3].0
            ]
        );
        let bare = compose("subject", None, false, None);
        assert_eq!(bare.lines().count(), 3);
    }

    struct Recording;

    impl CommitExemption for Recording {
        fn exempt(
            &self,
            _commit: &CommitSha,
            _basis: &str,
            _caller: Option<&crate::ids::SessionRef>,
        ) -> Result<(), crate::interfaces::InterfaceError> {
            Ok(())
        }
    }

    #[test]
    fn both_options_together_are_refused_and_each_alone_is_the_choice() {
        assert!(matches!(
            ReviewRouting::from_options(true, Some("typo".into())),
            Err(RepositoryError::RoutingBoth)
        ));
        assert_eq!(
            ReviewRouting::from_options(true, None).unwrap(),
            Some(ReviewRouting::Review)
        );
        assert!(matches!(
            ReviewRouting::from_options(false, Some("typo".into())),
            Ok(Some(ReviewRouting::Exempt(_)))
        ));
        assert_eq!(ReviewRouting::from_options(false, None).unwrap(), None);
    }

    #[test]
    fn an_adopted_project_requires_one_choice_with_a_basis_and_a_wired_admission() {
        let port: &dyn CommitExemption = &Recording;
        let review = ReviewRouting::Review;
        let exempt = ReviewRouting::Exempt("fixes a typo".into());
        let blank = ReviewRouting::Exempt(" \n".into());
        assert!(matches!(
            admit_routing(true, None, Some(port)),
            Err(RepositoryError::RoutingMissing)
        ));
        assert!(matches!(
            admit_routing(true, Some(&blank), Some(port)),
            Err(RepositoryError::RoutingBasisEmpty)
        ));
        assert!(matches!(
            admit_routing(true, Some(&exempt), None),
            Err(RepositoryError::ExemptionUnavailable)
        ));
        assert!(matches!(
            admit_routing(true, Some(&review), None),
            Ok(Admitted::Nothing)
        ));
        assert!(matches!(
            admit_routing(true, Some(&exempt), Some(port)),
            Ok(Admitted::Exemption { .. })
        ));
    }

    #[test]
    fn outside_an_adopted_project_no_choice_is_needed_and_an_exemption_is_refused() {
        let port: &dyn CommitExemption = &Recording;
        let exempt = ReviewRouting::Exempt("fixes a typo".into());
        assert!(matches!(
            admit_routing(false, None, None),
            Ok(Admitted::Nothing)
        ));
        assert!(matches!(
            admit_routing(false, Some(&ReviewRouting::Review), None),
            Ok(Admitted::Nothing)
        ));
        assert!(matches!(
            admit_routing(false, Some(&exempt), Some(port)),
            Err(RepositoryError::ExemptOutsideProject)
        ));
    }

    #[test]
    fn every_routing_refusal_names_both_options_and_the_policy() {
        let refusals = [
            RepositoryError::RoutingMissing,
            RepositoryError::RoutingBoth,
            RepositoryError::RoutingBasisEmpty,
            RepositoryError::ExemptOutsideProject,
            RepositoryError::ExemptionUnavailable,
        ]
        .map(|refusal| refusal.to_string());
        insta::assert_snapshot!("routing-refusals", refusals.join("\n\n"));
    }
}
