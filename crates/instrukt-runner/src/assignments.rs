//! Resolve runner-owned standing and scheduled assignment evidence.
use instrukt_core::{
    capabilities::Assignment,
    home::Home,
    ids::{ProjectSlug, RoleName, SessionRef, SnippetId},
    state_db::{StateDbError, StateRead},
};

fn standing_assignment(
    home: &Home,
    caller: &SessionRef,
) -> Result<Option<Assignment>, StateDbError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(None);
    };
    let mut stmt = db.conn().prepare("SELECT semantic_key,exact_home,checkout,declaration,procedure FROM procedure_responsibilities WHERE session=?1 UNION ALL SELECT g.semantic_key,r.exact_home,json_extract(g.request,'$.checkout'),g.declaration,r.procedure FROM procedure_generations g JOIN procedure_responsibilities r ON r.responsibility=g.responsibility WHERE g.session=?1").map_err(StateDbError::sqlite(db.path()))?;
    let rows = stmt
        .query_map([caller.to_string()], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .map_err(StateDbError::sqlite(db.path()))?;
    let mut found = None;
    for row in rows {
        let (key, exact_home, checkout, declaration, procedure) =
            row.map_err(StateDbError::sqlite(db.path()))?;
        let declaration: serde_json::Value = serde_json::from_str(&declaration)
            .map_err(|e| StateDbError::invalid(db.path(), format!("standing key {key}: {e}")))?;
        let role=declaration["role"].as_str().filter(|s|!s.is_empty()).ok_or_else(||StateDbError::invalid(db.path(),format!("standing key {key} lost its initialized role; recover its assignment evidence")))?;
        let project = exact_home.strip_prefix("project:").map(str::to_owned);
        if exact_home != "platform" && project.is_none() {
            return Err(StateDbError::invalid(
                db.path(),
                format!("standing key {key} has an unresolved home"),
            ));
        }
        let assignment = Assignment {
            worker: false,
            role: RoleName::parse(role)
                .map_err(|e| StateDbError::invalid(db.path(), e.to_string()))?,
            project: project
                .map(|slug| {
                    ProjectSlug::parse(&slug)
                        .map_err(|e| StateDbError::invalid(db.path(), e.to_string()))
                })
                .transpose()?,
            procedure: Some(
                SnippetId::parse(&procedure)
                    .map_err(|e| StateDbError::invalid(db.path(), e.to_string()))?,
            ),
            todo: None,
            workspace: Some(checkout.into()),
            key,
            platform: exact_home == "platform",
            dispatcher: None,
        };
        if found.as_ref().is_some_and(|a| a != &assignment) {
            return Err(StateDbError::invalid(
                db.path(),
                "conflicting standing assignments; recover the standing key",
            ));
        }
        found = Some(assignment);
    }
    Ok(found)
}

/// Work owns its bindings; runner composes those facts with its own standing/start records.
pub type WorkLookup = fn(&Home, &SessionRef) -> Result<Option<Assignment>, StateDbError>;
pub fn resolve(
    home: &Home,
    caller: &SessionRef,
    work: WorkLookup,
) -> Result<Option<Assignment>, StateDbError> {
    let work = work(home, caller)?;
    let standing = standing_assignment(home, caller)?;
    if work.is_some() && standing.is_some() {
        return Err(StateDbError::invalid(
            &home.state_db_file(),
            format!(
                "{caller} has conflicting work and standing assignments; recover its binding/standing key"
            ),
        ));
    }
    let assignment = work.or(standing);
    if assignment.is_none()
        && let Some(db) = StateRead::open(home)?
    {
        let known:bool=db.conn().query_row("SELECT EXISTS(SELECT 1 FROM runtime_bindings WHERE runtime=?1 AND session=?2 AND kind IN ('todo','work','task','standing'))",instrukt_core::state_db::rusqlite::params![caller.runtime.as_str(),caller.id.as_str()],|r|r.get(0)).map_err(StateDbError::sqlite(db.path()))?;
        if known {
            return Err(StateDbError::invalid(
                db.path(),
                format!(
                    "{caller} has a known start but lost its assignment; recover the affected binding/attempt or standing key"
                ),
            ));
        }
    }
    Ok(assignment)
}
