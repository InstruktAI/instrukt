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
    // The third branch admits the start this runner recorded for the current
    // generation before the responsibility records that start's session.
    let mut stmt = db.conn().prepare("SELECT semantic_key,exact_home,checkout,declaration,procedure FROM procedure_responsibilities WHERE session=?1 UNION ALL SELECT g.semantic_key,r.exact_home,json_extract(g.request,'$.checkout'),g.declaration,r.procedure FROM procedure_generations g JOIN procedure_responsibilities r ON r.responsibility=g.responsibility WHERE g.session=?1 UNION ALL SELECT p.semantic_key,p.exact_home,p.checkout,p.declaration,p.procedure FROM runtime_bindings b JOIN procedure_responsibilities p ON p.semantic_key=b.semantic_key WHERE b.runtime=?2 AND b.session=?1 AND p.session IS NULL").map_err(StateDbError::sqlite(db.path()))?;
    let rows = stmt
        .query_map(
            instrukt_core::state_db::rusqlite::params![caller.to_string(), caller.runtime.as_str()],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                ))
            },
        )
        .map_err(StateDbError::sqlite(db.path()))?;
    let mut found = None;
    for row in rows {
        let (key, exact_home, checkout, declaration, procedure) =
            row.map_err(StateDbError::sqlite(db.path()))?;
        let declaration: serde_json::Value = serde_json::from_str(&declaration)
            .map_err(|e| StateDbError::invalid(db.path(), format!("standing key {key}: {e}")))?;
        let role=declaration["role"].as_str().filter(|s|!s.is_empty()).ok_or_else(||StateDbError::invalid(db.path(),format!("standing key {key} lost its initialized role; recover its assignment evidence")))?;
        let project = exact_home.strip_prefix("project:").map(str::to_owned);
        let company = exact_home
            .strip_prefix("company:")
            .map(|name| {
                ProjectSlug::parse(name)
                    .map_err(|e| StateDbError::invalid(db.path(), e.to_string()))
            })
            .transpose()?;
        if exact_home != "platform" && project.is_none() && company.is_none() {
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
            company,
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

/// The current start this runner recorded for a caller's native session: the
/// semantic key it started, the session that start replaces, and the
/// worktree the runtime placed it in once known. An owner that has not yet
/// recorded the session (or still names the replaced one) decides from this
/// fact whether the caller is the start of its object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedStart {
    pub semantic_key: String,
    pub replaced_session: Option<String>,
    pub worktree: Option<String>,
}

/// The caller's current recorded start in this runner's own start records.
fn recorded_start(home: &Home, caller: &SessionRef) -> Result<Option<RecordedStart>, StateDbError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(None);
    };
    let mut stmt = db
        .conn()
        .prepare("SELECT semantic_key,replaced_session,worktree FROM runtime_bindings WHERE runtime=?1 AND session=?2")
        .map_err(StateDbError::sqlite(db.path()))?;
    let starts = stmt
        .query_map(
            instrukt_core::state_db::rusqlite::params![caller.runtime.as_str(), caller.to_string()],
            |r| {
                Ok(RecordedStart {
                    semantic_key: r.get(0)?,
                    replaced_session: r.get(1)?,
                    worktree: r.get(2)?,
                })
            },
        )
        .map_err(StateDbError::sqlite(db.path()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(StateDbError::sqlite(db.path()))?;
    match <[RecordedStart; 1]>::try_from(starts) {
        Ok([start]) => Ok(Some(start)),
        Err(starts) if starts.is_empty() => Ok(None),
        Err(_) => Err(StateDbError::invalid(
            db.path(),
            format!(
                "{caller} is recorded as the start of several keys; recover its runtime binding"
            ),
        )),
    }
}

/// Work owns its bindings; runner composes those facts with its own
/// standing/start records and supplies the caller's recorded start.
pub type WorkLookup =
    fn(&Home, &SessionRef, Option<&RecordedStart>) -> Result<Option<Assignment>, StateDbError>;
/// The company owns its conversation sessions' bindings, as work owns its
/// own: the assignment of a session one of its conversations bound, or of the
/// recorded start of one of its conversations.
pub type CompanyLookup =
    fn(&Home, &SessionRef, Option<&RecordedStart>) -> Result<Option<Assignment>, StateDbError>;
pub fn resolve(
    home: &Home,
    caller: &SessionRef,
    work: WorkLookup,
    company: CompanyLookup,
) -> Result<Option<Assignment>, StateDbError> {
    let start = recorded_start(home, caller)?;
    let work = work(home, caller, start.as_ref())?;
    let standing = match standing_assignment(home, caller)? {
        Some(standing) => Some(standing),
        None => company(home, caller, start.as_ref())?,
    };
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
        let known:bool=db.conn().query_row("SELECT EXISTS(SELECT 1 FROM runtime_bindings WHERE runtime=?1 AND session=?2 AND kind IN ('todo','work','task','standing'))",instrukt_core::state_db::rusqlite::params![caller.runtime.as_str(),caller.to_string()],|r|r.get(0)).map_err(StateDbError::sqlite(db.path()))?;
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
