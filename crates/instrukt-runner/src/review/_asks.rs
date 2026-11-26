//! Author asks to a held integration candidate's authoring sessions
//! (`project/instrukt/spec/feature/runner/pull-request-review`, Asking a
//! held candidate's authors): one record per project, commit and session in
//! `review_asks`, written with the `runtime_inputs` row each attempt queues
//! in one short transaction, its outcome taken from that input's runtime
//! acceptance, and retired with its unselected and failed inputs once the
//! canonical branch contains the commit or no branch reaches it. No runtime
//! or GitHub read happens inside a transaction.

use instrukt_core::ids::{CommitSha, ProjectSlug, SessionRef};
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use jiff::Timestamp;

use super::{HeldCandidate, PresenceAnswer, ReviewPass};
use crate::events::{Event, HeldCommitFact, HeldDecision, HeldSessionFact};
use crate::inputs::{self, Entry, InputKind, Intent};
use crate::state::{self, RunnerState, StateError};

/// The runner's acts for a held candidate: each session workflow names is
/// asked once, the sessions the runtime could not say about are reported
/// to the operator, and the integrator is told once every session is
/// answered.
pub(super) fn act(
    pass: &ReviewPass<'_>,
    candidate: &CommitSha,
    held: &HeldCandidate,
) -> Result<(), String> {
    let project = &pass.project.slug;
    let unanswered: Vec<String> = held
        .unanswered()
        .map(|(commit, session)| {
            tracing::warn!(event = "review_presence_unanswered", project = %project, candidate = %candidate, commit = %commit, session = %session);
            format!("{session} (commit {commit})")
        })
        .collect();
    for (commit, session) in &held.asks {
        queue(pass.state, project, candidate, commit, session, pass.now)
            .map_err(|error| error.to_string())?;
    }
    presence_status(
        pass.home,
        project,
        (!unanswered.is_empty())
            .then(|| unanswered.join(", "))
            .as_deref(),
    );
    if held.answered {
        let commits = held
            .commits
            .iter()
            .map(|held| HeldCommitFact {
                commit: held.commit.clone(),
                sessions: held
                    .sessions
                    .iter()
                    .map(|(session, answer)| HeldSessionFact {
                        session: session.clone(),
                        decision: match answer {
                            PresenceAnswer::Held => HeldDecision::Asked,
                            PresenceAnswer::NotHeld | PresenceAnswer::CannotSay => {
                                HeldDecision::NotHeld
                            }
                        },
                    })
                    .collect(),
            })
            .collect();
        pass.log
            .log(&Event::CandidateHeld {
                project: project.clone(),
                candidate: candidate.clone(),
                commits,
            })
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// The operator's report of the sessions the runtime could not say about in
/// the project's latest held-candidate derivation (`instrukt events
/// status`); none removes it.
pub(super) fn presence_status(
    home: &instrukt_core::home::Home,
    project: &ProjectSlug,
    unanswered: Option<&str>,
) {
    let failure = unanswered.map(|sessions| {
        format!("the runtime could not say whether it holds these authoring sessions of the held candidate: {sessions}")
    });
    state::worker_state(
        home,
        &format!("review-presence:{project}"),
        failure.as_deref(),
    );
}

/// What delivery made of an ask's current input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    /// The runtime accepted it, or reported it consumed in a turn.
    Asked,
    /// Delivery ended it without the runtime accepting it.
    Undelivered,
    /// It still waits in delivery.
    Waiting,
}

/// The outcome of an ask whose input is in `input_state` (none when the row
/// is gone), with whether the runtime accepted it. Acceptance decides, not
/// the state: an input can turn unconfirmed without having been accepted.
fn outcome(input_state: Option<&str>, accepted: bool) -> Outcome {
    match input_state {
        _ if accepted => Outcome::Asked,
        Some("entered") => Outcome::Asked,
        Some("failed" | "set_aside" | "unconfirmed") | None => Outcome::Undelivered,
        Some(_) => Outcome::Waiting,
    }
}

/// What a queue pass does for a commit and session the finding names to be
/// asked, from its record's state and attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Queue {
    /// Queue this attempt; for a later attempt, remove the previous one's
    /// ended input first.
    Attempt(u64),
    /// Already queued or asked: nothing more is sent.
    Nothing,
}

fn queue_decision(record: Option<(&str, u64)>) -> Queue {
    match record {
        None => Queue::Attempt(1),
        Some(("undelivered", attempt)) => Queue::Attempt(attempt + 1),
        Some(_) => Queue::Nothing,
    }
}

/// The fixed wording of an ask: no agent text enters it.
fn ask_content(project: &ProjectSlug, candidate: &CommitSha, commit: &CommitSha) -> String {
    let ask = format!(
        "Commit {commit}, which names you as its authoring session, is in integration candidate \
         {candidate} of project {project} with neither an independent review nor a recorded \
         exemption, so the candidate is held until you decide under general/policy/commits. \
         If it owes no independent review, record why with \
         `instrukt review exempt --commit {commit} --basis <basis>`. If it does, ask for the \
         independent review of a head that contains it with `instrukt review start`."
    );
    serde_json::json!({
        "project": project.as_str(),
        "candidate": candidate.as_str(),
        "commit": commit.as_str(),
        "ask": ask,
    })
    .to_string()
}

fn input_id(project: &ProjectSlug, commit: &str, attempt: u64) -> String {
    format!("review-ask:{project}:{commit}:{attempt}")
}

/// Takes each queued ask's outcome from its current input.
pub(super) fn sync(
    state: &RunnerState,
    project: &ProjectSlug,
    now: Timestamp,
) -> Result<(), StateError> {
    state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        let queued: Vec<(String, String, String)> = {
            let mut query = tx
                .prepare("SELECT commit_sha,session,input_id FROM review_asks WHERE project=?1 AND state='queued'")
                .map_err(state::sql(&path))?;
            query
                .query_map([project.as_str()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
                .map_err(state::sql(&path))?
                .collect::<Result<_, _>>()
                .map_err(state::sql(&path))?
        };
        for (commit, session, input) in queued {
            let Ok(target) = SessionRef::parse(&session) else {
                continue;
            };
            let current = tx
                .query_row(
                    "SELECT state,accepted_at IS NOT NULL,failure FROM runtime_inputs WHERE runtime=?1 AND session=?2 AND input_id=?3",
                    params![target.runtime.as_str(), target.id.as_str(), input],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, r.get::<_, Option<String>>(2)?)),
                )
                .optional()
                .map_err(state::sql(&path))?;
            let (input_state, accepted, failure) = match &current {
                Some((input_state, accepted, failure)) => (Some(input_state.as_str()), *accepted, failure.clone()),
                None => (None, false, Some("the ask's input is gone".to_owned())),
            };
            let (next, cause) = match outcome(input_state, accepted) {
                Outcome::Waiting => continue,
                Outcome::Asked => ("asked", None),
                Outcome::Undelivered => (
                    "undelivered",
                    Some(failure.unwrap_or_else(|| format!("delivery ended the ask {}", input_state.unwrap_or("gone")))),
                ),
            };
            tx.execute(
                "UPDATE review_asks SET state=?4,cause=?5,changed_at=?6 WHERE project=?1 AND commit_sha=?2 AND session=?3",
                params![project.as_str(), commit, session, next, cause, now.to_string()],
            )
            .map_err(state::sql(&path))?;
            tracing::info!(event = "review_ask_outcome", project = %project, commit = %commit, session = %session, outcome = next, cause = cause.as_deref());
        }
        Ok(())
    })
}

/// Queues the ask of `commit` to `session` for `candidate` unless it is
/// already queued or asked: the record and its input in one transaction.
pub(super) fn queue(
    state: &RunnerState,
    project: &ProjectSlug,
    candidate: &CommitSha,
    commit: &CommitSha,
    session: &SessionRef,
    now: Timestamp,
) -> Result<(), StateError> {
    let target = session.to_string();
    state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        let record = tx
            .query_row(
                "SELECT state,attempt,input_id FROM review_asks WHERE project=?1 AND commit_sha=?2 AND session=?3",
                params![project.as_str(), commit.as_str(), target],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?, r.get::<_, String>(2)?)),
            )
            .optional()
            .map_err(state::sql(&path))?;
        let Queue::Attempt(attempt) = queue_decision(record.as_ref().map(|(s, a, _)| (s.as_str(), *a))) else {
            return Ok(());
        };
        if let Some((_, _, previous)) = &record {
            // Its cause is on the record; only an input that ended is removed.
            tx.execute(
                "DELETE FROM runtime_inputs WHERE runtime=?1 AND session=?2 AND input_id=?3 AND state IN ('failed','set_aside','unconfirmed') AND accepted_at IS NULL",
                params![session.runtime.as_str(), session.id.as_str(), previous],
            )
            .map_err(state::sql(&path))?;
        }
        let id = input_id(project, commit.as_str(), attempt);
        let content = ask_content(project, candidate, commit);
        inputs::retain(
            tx,
            session,
            &Entry {
                id: &id,
                kind: InputKind::ReviewAsk,
                assignment: None,
                source_at: None,
                arrived_at: now,
                intent: Intent::Asap,
                content: &content,
            },
        )?;
        tx.execute(
            "INSERT INTO review_asks(project,commit_sha,session,candidate,attempt,input_id,state,cause,queued_at,changed_at) VALUES(?1,?2,?3,?4,?5,?6,'queued',NULL,?7,?7)
             ON CONFLICT(project,commit_sha,session) DO UPDATE SET attempt=excluded.attempt,input_id=excluded.input_id,state='queued',changed_at=excluded.changed_at",
            params![project.as_str(), commit.as_str(), target, candidate.as_str(), attempt, id, now.to_string()],
        )
        .map_err(state::sql(&path))?;
        tracing::info!(event = "review_ask_queued", project = %project, candidate = %candidate, commit = %commit, session = %target, attempt);
        Ok(())
    })
}

/// Retires the project's ask records whose commit the canonical branch
/// contains or no branch of the repository reaches, with their inputs
/// delivery has not selected or that ended failed.
pub(super) fn retire(
    state: &RunnerState,
    project: &ProjectSlug,
    root: &std::path::Path,
    remote: &str,
) -> Result<(), String> {
    let commits: Vec<String> = state
        .read(|conn, path| {
            let mut query = conn
                .prepare("SELECT DISTINCT commit_sha FROM review_asks WHERE project=?1")
                .map_err(state::sql(path))?;
            query
                .query_map([project.as_str()], |r| r.get(0))
                .map_err(state::sql(path))?
                .collect::<Result<_, _>>()
                .map_err(state::sql(path))
        })
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    if commits.is_empty() {
        return Ok(());
    }
    let canonical = format!(
        "refs/remotes/{remote}/{}",
        instrukt_core::git::default_branch(root, remote).map_err(|e| e.to_string())?
    );
    let canonical = canonical.as_str();
    let mut retired = Vec::new();
    for commit in commits {
        if settled(root, &commit, canonical)? {
            retired.push(commit);
        }
    }
    if retired.is_empty() {
        return Ok(());
    }
    state
        .write::<_, StateError>(|tx| {
            let path = state::db_path(tx);
            for commit in &retired {
                let asks: Vec<(String, String)> = {
                    let mut query = tx
                        .prepare("SELECT session,input_id FROM review_asks WHERE project=?1 AND commit_sha=?2")
                        .map_err(state::sql(&path))?;
                    query
                        .query_map(params![project.as_str(), commit], |r| Ok((r.get(0)?, r.get(1)?)))
                        .map_err(state::sql(&path))?
                        .collect::<Result<_, _>>()
                        .map_err(state::sql(&path))?
                };
                for (session, _) in &asks {
                    let Ok(target) = SessionRef::parse(session) else {
                        continue;
                    };
                    tx.execute(
                        "DELETE FROM runtime_inputs WHERE runtime=?1 AND session=?2 AND kind='review_ask' AND input_id GLOB ?3 AND ((state='pending' AND group_id IS NULL) OR state='failed')",
                        params![target.runtime.as_str(), target.id.as_str(), format!("review-ask:{project}:{commit}:*")],
                    )
                    .map_err(state::sql(&path))?;
                }
                tx.execute(
                    "DELETE FROM review_asks WHERE project=?1 AND commit_sha=?2",
                    params![project.as_str(), commit],
                )
                .map_err(state::sql(&path))?;
                tracing::debug!(event = "review_ask_retired", project = %project, commit = %commit, sessions = asks.len());
            }
            Ok(())
        })
        .map_err(|e| e.to_string())
}

/// Whether `commit` no longer waits on a decision: the canonical branch
/// contains it, or no branch of the repository at `root` reaches it.
fn settled(root: &std::path::Path, commit: &str, canonical: &str) -> Result<bool, String> {
    use instrukt_core::git;
    let object = format!("{commit}^{{commit}}");
    if !git::succeeds(root, ["cat-file", "-e", object.as_str()]).map_err(|e| e.to_string())? {
        return Ok(true);
    }
    if git::succeeds(root, ["merge-base", "--is-ancestor", commit, canonical])
        .map_err(|e| e.to_string())?
    {
        return Ok(true);
    }
    let reaching = git::run(
        root,
        [
            "for-each-ref",
            "--count=1",
            "--contains",
            commit,
            "--format=%(refname)",
            "refs/heads",
            "refs/remotes",
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(reaching.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_ask_counts_as_asked_only_on_acceptance_or_consumption() {
        assert_eq!(outcome(Some("accepted"), true), Outcome::Asked);
        assert_eq!(outcome(Some("unconfirmed"), true), Outcome::Asked);
        assert_eq!(outcome(Some("entered"), false), Outcome::Asked);
        assert_eq!(outcome(Some("unconfirmed"), false), Outcome::Undelivered);
        assert_eq!(outcome(Some("failed"), false), Outcome::Undelivered);
        assert_eq!(outcome(None, false), Outcome::Undelivered);
        assert_eq!(outcome(Some("pending"), false), Outcome::Waiting);
        assert_eq!(outcome(Some("selected"), false), Outcome::Waiting);
    }

    #[test]
    fn only_an_undelivered_ask_is_asked_again_under_the_next_attempt() {
        assert_eq!(queue_decision(None), Queue::Attempt(1));
        assert_eq!(queue_decision(Some(("undelivered", 2))), Queue::Attempt(3));
        assert_eq!(queue_decision(Some(("queued", 1))), Queue::Nothing);
        assert_eq!(queue_decision(Some(("asked", 1))), Queue::Nothing);
    }

    #[test]
    fn the_ask_names_the_policy_and_both_ways_to_decide() {
        let project = ProjectSlug::parse("demo").unwrap();
        let candidate = CommitSha::parse(&"c".repeat(40)).unwrap();
        let commit = CommitSha::parse(&"d".repeat(40)).unwrap();
        let content: serde_json::Value =
            serde_json::from_str(&ask_content(&project, &candidate, &commit)).unwrap();
        assert_eq!(content["commit"], commit.as_str());
        let ask = content["ask"].as_str().unwrap();
        for named in [
            "general/policy/commits",
            &format!("instrukt review exempt --commit {commit} --basis <basis>"),
            "instrukt review start",
        ] {
            assert!(ask.contains(named), "{ask}: {named}");
        }
    }
}
