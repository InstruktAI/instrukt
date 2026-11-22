//! Read fixed role and placement evidence without changing a work claim.
use super::Binding;
use instrukt_core::{
    capabilities::Assignment,
    home::Home,
    ids::SessionRef,
    state_db::{
        StateDbError, StateRead,
        rusqlite::{OptionalExtension, params},
    },
};

const CONFLICT_MESSAGE: &str =
    "session has conflicting recorded assignments; recover its binding/attempt";

struct AssignmentCandidate {
    active: bool,
    attempt: i64,
    assignment: Assignment,
}

fn select_assignment(
    candidates: Vec<AssignmentCandidate>,
) -> Result<Option<Assignment>, &'static str> {
    let mut active_chosen: Option<Assignment> = None;
    let mut released = Vec::new();
    for candidate in candidates {
        if candidate.active {
            if active_chosen
                .as_ref()
                .is_some_and(|previous| previous != &candidate.assignment)
            {
                return Err(CONFLICT_MESSAGE);
            }
            active_chosen = Some(candidate.assignment);
        } else {
            released.push(candidate);
        }
    }
    if active_chosen.is_some() {
        return Ok(active_chosen);
    }
    let Some(first) = released.first() else {
        return Ok(None);
    };
    let scope = (
        &first.assignment.project,
        &first.assignment.todo,
        &first.assignment.role,
        first.assignment.worker,
    );
    if released.iter().any(|candidate| {
        (
            &candidate.assignment.project,
            &candidate.assignment.todo,
            &candidate.assignment.role,
            candidate.assignment.worker,
        ) != scope
    }) {
        return Err(CONFLICT_MESSAGE);
    }
    let Some(max_attempt) = released.iter().map(|candidate| candidate.attempt).max() else {
        return Ok(None);
    };
    let mut latest: Option<Assignment> = None;
    for candidate in released.into_iter().filter(|c| c.attempt == max_attempt) {
        if latest
            .as_ref()
            .is_some_and(|previous| previous != &candidate.assignment)
        {
            return Err(CONFLICT_MESSAGE);
        }
        latest = Some(candidate.assignment);
    }
    Ok(latest)
}

pub fn recorded_assignment(
    home: &Home,
    caller: &SessionRef,
) -> Result<Option<Assignment>, StateDbError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(None);
    };
    let mut stmt = db.conn().prepare("SELECT b.role,b.semantic_key,b.worktree,b.evidence,t.project,t.id,b.session,b.released_at,b.attempt FROM session_bindings b LEFT JOIN todos t ON t.id=b.todo_id WHERE b.session=?1 UNION ALL SELECT b.role,b.semantic_key,b.worktree,b.evidence,t.project,t.id,b.session,b.released_at,b.attempt FROM runtime_generations g JOIN session_bindings b ON b.semantic_key=g.semantic_key LEFT JOIN todos t ON t.id=b.todo_id WHERE g.session=?1 AND g.generation>0").map_err(StateDbError::sqlite(db.path()))?;
    let rows = stmt
        .query_map([caller.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, Option<String>>(7)?,
                r.get::<_, i64>(8)?,
            ))
        })
        .map_err(StateDbError::sqlite(db.path()))?;
    let mut candidates = Vec::new();
    for row in rows {
        let (role, key, workspace, evidence, project, todo, current_session, released_at, attempt) =
            row.map_err(StateDbError::sqlite(db.path()))?;
        let binding: Binding = serde_json::from_str(&evidence).map_err(|e| {
            StateDbError::invalid(
                db.path(),
                format!("binding {key}: {e}; recover the recorded attempt"),
            )
        })?;
        let current_session = current_session
            .map(|id| {
                SessionRef::parse(&id).map_err(|e| StateDbError::invalid(db.path(), e.to_string()))
            })
            .transpose()?;
        if binding.role.as_str() != role
            || binding.session != current_session
            || binding.attempt.semantic_key != key
            || binding.worktree.as_deref() != workspace.as_deref().map(std::path::Path::new)
            || binding.attempt.assignment
                == instrukt_runner::interfaces::AssignmentKind::Unspecified
        {
            return Err(StateDbError::invalid(
                db.path(),
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
                        .map_err(|error| StateDbError::invalid(db.path(), error.to_string()))
                })
                .transpose()?,
            workspace: workspace.map(Into::into),
            project: Some(
                instrukt_core::ids::ProjectSlug::parse(&project)
                    .map_err(|error| StateDbError::invalid(db.path(), error.to_string()))?,
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
    select_assignment(candidates).map_err(|message| StateDbError::invalid(db.path(), message))
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
            Err(CONFLICT_MESSAGE)
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
    fn released_attempts_of_same_role_and_todo_select_latest() {
        let first = candidate(false, 1, "prepare-orchestrator", "todo-a", false);
        let second = candidate(false, 2, "prepare-orchestrator", "todo-a", false);
        let expected = second.assignment.clone();
        assert_eq!(select_assignment(vec![first, second]), Ok(Some(expected)));
    }

    #[test]
    fn released_attempts_of_different_roles_return_conflict_error() {
        let first = candidate(false, 1, "prepare-orchestrator", "todo-a", false);
        let second = candidate(false, 2, "work-orchestrator", "todo-a", false);
        assert_eq!(
            select_assignment(vec![first, second]),
            Err(CONFLICT_MESSAGE)
        );
    }
}
