//! Read fixed role and placement evidence without changing a work claim.
use super::Binding;
use instrukt_core::{
    capabilities::Assignment,
    home::Home,
    ids::SessionRef,
    state_db::{
        StateDbError, StateRead,
        rusqlite::{Connection, OptionalExtension, params},
    },
};
use instrukt_runner::assignments::RecordedStart;
use std::path::Path;

/// Why a session resolves to no single assignment: the keys of the
/// recorded bindings that disagree. No instrukt command takes a session off
/// a binding, so these go to the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AssignmentConflict {
    /// The disagreeing bindings' semantic keys, each once, in the order the
    /// rows were read.
    pub keys: Vec<String>,
}

impl AssignmentConflict {
    fn of<'a>(candidates: impl IntoIterator<Item = &'a AssignmentCandidate>) -> Self {
        let mut keys: Vec<String> = Vec::new();
        for candidate in candidates {
            if !keys.contains(&candidate.assignment.key) {
                keys.push(candidate.assignment.key.clone());
            }
        }
        Self { keys }
    }
}

impl std::fmt::Display for AssignmentConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "session has conflicting recorded assignments on bindings {}; no instrukt command resolves this session, so report these bindings to the operator",
            quoted(&self.keys)
        )
    }
}

/// `keys` as a list of backquoted names.
pub(crate) fn quoted(keys: &[String]) -> String {
    keys.iter()
        .map(|key| format!("`{key}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) struct AssignmentCandidate {
    active: bool,
    attempt: i64,
    assignment: Assignment,
}

/// The one assignment a session's candidates resolve to. They must share
/// one (project, todo, role, worker) scope, released attempts included;
/// within it the active attempt wins, else the highest released attempt.
/// Candidates that disagree are returned as the conflict naming them.
pub(crate) fn select_assignment(
    candidates: Vec<AssignmentCandidate>,
) -> Result<Option<Assignment>, AssignmentConflict> {
    let Some(first) = candidates.first() else {
        return Ok(None);
    };
    let scope = (
        &first.assignment.project,
        &first.assignment.todo,
        &first.assignment.role,
        first.assignment.worker,
    );
    if candidates.iter().any(|candidate| {
        (
            &candidate.assignment.project,
            &candidate.assignment.todo,
            &candidate.assignment.role,
            candidate.assignment.worker,
        ) != scope
    }) {
        return Err(AssignmentConflict::of(&candidates));
    }
    let mut active_chosen: Option<AssignmentCandidate> = None;
    let mut released = Vec::new();
    for candidate in candidates {
        if candidate.active {
            if let Some(previous) = active_chosen
                .as_ref()
                .filter(|previous| previous.assignment != candidate.assignment)
            {
                return Err(AssignmentConflict::of([previous, &candidate]));
            }
            active_chosen = Some(candidate);
        } else {
            released.push(candidate);
        }
    }
    if let Some(chosen) = active_chosen {
        return Ok(Some(chosen.assignment));
    }
    let Some(max_attempt) = released.iter().map(|candidate| candidate.attempt).max() else {
        return Ok(None);
    };
    let mut latest: Option<AssignmentCandidate> = None;
    for candidate in released.into_iter().filter(|c| c.attempt == max_attempt) {
        if let Some(previous) = latest
            .as_ref()
            .filter(|previous| previous.assignment != candidate.assignment)
        {
            return Err(AssignmentConflict::of([previous, &candidate]));
        }
        latest = Some(candidate);
    }
    Ok(latest.map(|candidate| candidate.assignment))
}

/// One binding row with the worktree a runner start supplies when the row
/// was found through that start.
type BindingRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
);

/// Bindings recorded for the caller's session, directly or through a later
/// runtime generation of their key.
const RECORDED_BINDINGS: &str = "SELECT b.role,b.semantic_key,b.worktree,b.evidence,t.project,t.id,b.session,b.released_at,b.attempt,NULL FROM session_bindings b LEFT JOIN todos t ON t.id=b.todo_id WHERE b.session=?1 UNION ALL SELECT b.role,b.semantic_key,b.worktree,b.evidence,t.project,t.id,b.session,b.released_at,b.attempt,NULL FROM runtime_generations g JOIN session_bindings b ON b.semantic_key=g.semantic_key LEFT JOIN todos t ON t.id=b.todo_id WHERE g.session=?1 AND g.generation>0";

/// The reserved, unreleased attempt a runner start names before work records
/// the caller's session on it: a fresh start whose binding has no session
/// yet, or a successor of the session the start replaces. A binding already
/// on the caller is read through [`RECORDED_BINDINGS`].
const STARTED_BINDING: &str = "SELECT b.role,b.semantic_key,b.worktree,b.evidence,t.project,t.id,b.session,b.released_at,b.attempt,?5 FROM session_bindings b JOIN todos t ON t.id=b.todo_id WHERE b.semantic_key=?3 AND (b.runtime IS NULL OR b.runtime=?2) AND b.released_at IS NULL AND (b.session IS NULL OR b.session=?4) AND b.session IS NOT ?1";

fn binding_rows(
    conn: &Connection,
    path: &Path,
    sql: &str,
    params: impl instrukt_core::state_db::rusqlite::Params,
) -> Result<Vec<BindingRow>, StateDbError> {
    let mut stmt = conn
        .prepare_cached(sql)
        .map_err(StateDbError::sqlite(path))?;
    stmt.query_map(params, |r| {
        Ok((
            r.get(0)?,
            r.get(1)?,
            r.get(2)?,
            r.get(3)?,
            r.get(4)?,
            r.get(5)?,
            r.get(6)?,
            r.get(7)?,
            r.get(8)?,
            r.get(9)?,
        ))
    })
    .map_err(StateDbError::sqlite(path))?
    .collect::<Result<Vec<_>, _>>()
    .map_err(StateDbError::sqlite(path))
}

/// The caller's assignment among its recorded bindings and the reserved
/// attempts its runner `starts` name; [`select_assignment`] decides among
/// several attempts.
pub fn recorded_assignment(
    home: &Home,
    caller: &SessionRef,
    starts: &[RecordedStart],
) -> Result<Option<Assignment>, StateDbError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(None);
    };
    let (conn, path) = (db.conn(), db.path());
    // Every row comes from one read snapshot. A binding work records on
    // the caller between two separate reads would be missed by
    // RECORDED_BINDINGS before the write and excluded by STARTED_BINDING
    // after it.
    let candidates = {
        let snapshot = conn
            .unchecked_transaction()
            .map_err(StateDbError::sqlite(path))?;
        assignment_candidates(&snapshot, path, caller, starts)?
    };
    select_assignment(candidates)
        .map_err(|conflict| StateDbError::invalid(path, conflict.to_string()))
}

/// The attempts `caller` may resolve to, read through `conn` — a read
/// snapshot or a write transaction, which then sees its own writes: its
/// recorded bindings, and the reserved attempts its runner `starts` name.
pub(crate) fn assignment_candidates(
    conn: &Connection,
    path: &Path,
    caller: &SessionRef,
    starts: &[RecordedStart],
) -> Result<Vec<AssignmentCandidate>, StateDbError> {
    let session = caller.to_string();
    let mut rows = binding_rows(conn, path, RECORDED_BINDINGS, params![session])?;
    for start in starts {
        rows.extend(binding_rows(
            conn,
            path,
            STARTED_BINDING,
            params![
                session,
                caller.runtime.as_str(),
                start.semantic_key,
                start.replaced_session,
                start.worktree,
            ],
        )?);
    }
    let mut candidates = Vec::new();
    for row in rows {
        let (
            role,
            key,
            workspace,
            evidence,
            project,
            todo,
            current_session,
            released_at,
            attempt,
            started_workspace,
        ) = row;
        let binding: Binding = serde_json::from_str(&evidence).map_err(|e| {
            StateDbError::invalid(
                path,
                format!("binding {key}: {e}; recover the recorded attempt"),
            )
        })?;
        let current_session = current_session
            .map(|id| {
                SessionRef::parse(&id).map_err(|e| StateDbError::invalid(path, e.to_string()))
            })
            .transpose()?;
        if binding.role.as_str() != role
            || binding.session != current_session
            || binding.attempt.semantic_key != key
            || binding.worktree.as_deref() != workspace.as_deref().map(std::path::Path::new)
            || u64::try_from(attempt).ok() != Some(binding.attempt.number)
            || binding.attempt.released_at.map(|t| t.to_string()) != released_at
            || binding.attempt.assignment
                == instrukt_runner::interfaces::AssignmentKind::Unspecified
        {
            return Err(StateDbError::invalid(
                path,
                format!(
                    "binding {key} has contradictory or missing assignment facts; recover the binding/attempt"
                ),
            ));
        }
        let dispatcher = binding
            .attempt
            .native_assignment
            .as_ref()
            .and_then(|native| native.dispatcher.clone());
        let procedure = binding
            .attempt
            .native_assignment
            .as_ref()
            .and_then(|native| native.procedure.clone());
        let assignment = Assignment {
            worker: binding.attempt.assignment
                != instrukt_runner::interfaces::AssignmentKind::Driver,
            role: binding.role.clone(),
            key,
            procedure: procedure
                .map(|id| {
                    instrukt_core::ids::SnippetId::parse(&id)
                        .map_err(|error| StateDbError::invalid(path, error.to_string()))
                })
                .transpose()?,
            workspace: workspace.or(started_workspace).map(Into::into),
            project: Some(
                instrukt_core::ids::ProjectSlug::parse(&project)
                    .map_err(|error| StateDbError::invalid(path, error.to_string()))?,
            ),
            todo: Some(todo),
            platform: false,
            company: None,
            dispatcher,
        };
        candidates.push(AssignmentCandidate {
            active: released_at.is_none(),
            attempt,
            assignment,
        });
    }
    Ok(candidates)
}

pub fn target_identity(
    home: &Home,
    project: &str,
    address: &str,
) -> Result<Option<String>, StateDbError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(None);
    };
    if super::TodoId::parse(address).is_some() {
        return db
            .conn()
            .query_row(
                "SELECT id FROM todos WHERE project=?1 AND id=?2",
                params![project, address],
                |r| r.get(0),
            )
            .optional()
            .map_err(StateDbError::sqlite(db.path()));
    }
    let address: super::TodoAddress = address
        .parse()
        .map_err(|e: super::AddressError| StateDbError::invalid(db.path(), e.to_string()))?;
    let mut query=db.conn().prepare("SELECT id,status FROM todos WHERE project=?1 AND slug=?2 AND (?3 IS NULL OR substr(id,1,length(?3))=?3) ORDER BY CASE WHEN status IN ('active','iceboxed','structural_holder') THEN 0 ELSE 1 END,rowid DESC").map_err(StateDbError::sqlite(db.path()))?;
    let rows = query
        .query_map(
            params![project, address.slug.as_str(), address.identity],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(StateDbError::sqlite(db.path()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StateDbError::sqlite(db.path()))?;
    match rows.as_slice() {
        [] => Ok(None),
        [(id, _)] => Ok(Some(id.clone())),
        [(id, status), ..]
            if address.identity.is_none()
                && matches!(status.as_str(), "active" | "iceboxed" | "structural_holder") =>
        {
            Ok(Some(id.clone()))
        }
        _ => Err(StateDbError::invalid(
            db.path(),
            "ambiguous todo target; address the retained todo by durable identity",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use instrukt_core::ids::{ProjectSlug, RoleName};

    fn candidate(
        active: bool,
        attempt: i64,
        role: &str,
        todo: &str,
        worker: bool,
    ) -> AssignmentCandidate {
        AssignmentCandidate {
            active,
            attempt,
            assignment: Assignment {
                worker,
                role: RoleName::parse(role).unwrap(),
                key: format!("instrukt:todo:demo/{todo}:{role}:{attempt}"),
                procedure: None,
                workspace: Some(format!("/tmp/wt/{todo}").into()),
                project: Some(ProjectSlug::parse("demo").unwrap()),
                todo: Some(todo.into()),
                platform: false,
                company: None,
                dispatcher: None,
            },
        }
    }

    fn conflict(keys: &[&str]) -> AssignmentConflict {
        AssignmentConflict {
            keys: keys.iter().map(|key| (*key).to_owned()).collect(),
        }
    }

    #[test]
    fn empty_candidates_return_none() {
        assert_eq!(select_assignment(Vec::new()), Ok(None));
    }

    #[test]
    fn active_attempt_supersedes_released_attempt_on_same_session() {
        let released = candidate(false, 1, "prepare-orchestrator", "todo-a", false);
        let active = candidate(true, 2, "prepare-orchestrator", "todo-a", false);
        let expected = active.assignment.clone();
        assert_eq!(
            select_assignment(vec![released, active]),
            Ok(Some(expected))
        );
    }

    #[test]
    fn conflicting_active_attempts_return_error() {
        let first = candidate(true, 1, "prepare-orchestrator", "todo-a", false);
        let second = candidate(true, 2, "prepare-orchestrator", "todo-a", false);
        assert_eq!(
            select_assignment(vec![first, second]),
            Err(conflict(&[
                "instrukt:todo:demo/todo-a:prepare-orchestrator:1",
                "instrukt:todo:demo/todo-a:prepare-orchestrator:2",
            ]))
        );
    }

    #[test]
    fn identical_active_rows_deduplicate() {
        let first = candidate(true, 2, "prepare-orchestrator", "todo-a", false);
        let duplicate = candidate(true, 2, "prepare-orchestrator", "todo-a", false);
        let expected = first.assignment.clone();
        assert_eq!(
            select_assignment(vec![first, duplicate]),
            Ok(Some(expected))
        );
    }

    #[test]
    fn active_attempt_with_released_attempt_from_other_todo_returns_conflict_error() {
        let released = candidate(false, 1, "prepare-orchestrator", "todo-a", false);
        let active = candidate(true, 1, "prepare-orchestrator", "todo-b", false);
        assert_eq!(
            select_assignment(vec![released, active]),
            Err(conflict(&[
                "instrukt:todo:demo/todo-a:prepare-orchestrator:1",
                "instrukt:todo:demo/todo-b:prepare-orchestrator:1",
            ]))
        );
    }

    #[test]
    fn released_attempts_of_same_role_and_todo_select_latest() {
        let first = candidate(false, 1, "prepare-orchestrator", "todo-a", false);
        let second = candidate(false, 2, "prepare-orchestrator", "todo-a", false);
        let expected = second.assignment.clone();
        assert_eq!(select_assignment(vec![second, first]), Ok(Some(expected)));
    }

    #[test]
    fn released_attempts_of_different_roles_return_conflict_error() {
        let first = candidate(false, 1, "prepare-orchestrator", "todo-a", false);
        let second = candidate(false, 2, "work-orchestrator", "todo-a", false);
        assert_eq!(
            select_assignment(vec![first, second]),
            Err(conflict(&[
                "instrukt:todo:demo/todo-a:prepare-orchestrator:1",
                "instrukt:todo:demo/todo-a:work-orchestrator:2",
            ]))
        );
    }

    #[test]
    fn the_conflict_names_its_bindings_and_sends_them_to_the_operator() {
        let message = conflict(&["k:builder:1", "k:build-reviewer:1"]).to_string();
        assert_eq!(
            message,
            "session has conflicting recorded assignments on bindings `k:builder:1`, `k:build-reviewer:1`; no instrukt command resolves this session, so report these bindings to the operator"
        );
    }
}
