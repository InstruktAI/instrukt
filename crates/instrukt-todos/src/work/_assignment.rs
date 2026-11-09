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

pub fn recorded_assignment(
    home: &Home,
    caller: &SessionRef,
) -> Result<Option<Assignment>, StateDbError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(None);
    };
    let mut stmt = db.conn().prepare("SELECT b.role,b.semantic_key,b.worktree,b.evidence,t.project,t.id FROM session_bindings b LEFT JOIN todos t ON t.id=b.todo_id WHERE b.session=?1 ORDER BY b.rowid").map_err(StateDbError::sqlite(db.path()))?;
    let rows = stmt
        .query_map([caller.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
            ))
        })
        .map_err(StateDbError::sqlite(db.path()))?;
    let mut found = None;
    for row in rows {
        let (role, key, workspace, evidence, project, todo) =
            row.map_err(StateDbError::sqlite(db.path()))?;
        let binding: Binding = serde_json::from_str(&evidence).map_err(|e| {
            StateDbError::invalid(
                db.path(),
                format!("binding {key}: {e}; recover the recorded attempt"),
            )
        })?;
        if binding.role.as_str() != role
            || binding.session.as_ref() != Some(caller)
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
            dispatcher,
        };
        if found
            .as_ref()
            .is_some_and(|previous| previous != &assignment)
        {
            return Err(StateDbError::invalid(
                db.path(),
                "session has conflicting recorded assignments; recover its binding/attempt",
            ));
        }
        found = Some(assignment);
    }
    Ok(found)
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
