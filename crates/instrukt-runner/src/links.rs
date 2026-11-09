//! Admit two-thread conversations and relay finished turns with durable handover evidence.
mod _diagnostics;
mod _management;
mod _turns;
mod _watch;
use crate::runtime::{
    RuntimeAdapter,
    conversations::{ConversationSource, SourcePosition},
};
pub use _management::{Discussion, Operation, manage};
pub(crate) use _watch::{Worker, record as record_failure};
use instrukt_core::{
    capabilities::RoleClass,
    config::Agents,
    home::Home,
    ids::SessionRef,
    state_db::{
        StateDb, StateDbError, StateRead,
        rusqlite::{OptionalExtension, params},
    },
};
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error(transparent)]
    Lock(#[from] instrukt_core::fsx::FsError),
    #[error(transparent)]
    Db(#[from] StateDbError),
    #[error(transparent)]
    Sql(#[from] instrukt_core::state_db::rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Source(#[from] crate::runtime::conversations::SourceError),
    #[error(transparent)]
    Config(#[from] instrukt_core::config::ConfigError),
    #[error(transparent)]
    Runtime(#[from] crate::sessions::SessionsError),
    #[error("conversation link: native {stage} failed: {diagnostic}; handover remains uncertain")]
    Native {
        stage: &'static str,
        diagnostic: Box<crate::runtime::NativeDiagnostic>,
    },
    #[error("conversation notification reader stopped: {0}; restart instrukt run")]
    Stopped(Box<LinkError>),
    #[error("conversation link: {0}")]
    Refused(String),
}
/// Two native members; role and scope always resolve from their recorded assignments.
pub struct Opening<'a> {
    pub a: &'a SessionRef,
    pub b: &'a SessionRef,
}
#[derive(Debug, Serialize)]
pub struct Status {
    pub id: String,
    pub a: SessionRef,
    pub b: SessionRef,
    pub active: bool,
    pub failure: Option<String>,
    pub directions: Vec<Progress>,
}
#[derive(Debug, Serialize)]
pub struct Progress {
    pub source: String,
    pub target: String,
    pub observed: SourcePosition,
    pub delivered_turn: Option<String>,
    pub pending: usize,
    #[serde(skip)]
    retained_turn: Option<String>,
    pub uncertain: bool,
    pub receipts: std::collections::BTreeMap<String, Disposition>,
    pub failure: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Disposition {
    Empty,
    Accepted { at: jiff::Timestamp },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Pending {
    turn: String,
    content: String,
    sending: bool,
    origin: Origin,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Origin {
    Turn,
    Opening { message: String },
    Closing { message: String },
}
#[derive(Clone, Copy)]
enum DeliveryMode {
    Relay,
    Management,
}
struct Direction {
    link: String,
    source: SessionRef,
    target: SessionRef,
    position: SourcePosition,
    seen: std::collections::BTreeSet<String>,
    pending: Vec<Pending>,
    delivered: Option<String>,
    receipts: std::collections::BTreeMap<String, Disposition>,
}

pub fn open(
    home: &Home,
    source: &dyn ConversationSource,
    work: crate::assignments::WorkLookup,
    request: &Opening<'_>,
) -> Result<String, LinkError> {
    open_plan(
        home,
        source,
        work,
        &OpeningPlan {
            pair: request,
            contribution: None,
        },
    )
}
struct OpeningPlan<'a> {
    pair: &'a Opening<'a>,
    contribution: Option<&'a dyn Fn() -> Result<Pending, LinkError>>,
}
fn open_plan(
    home: &Home,
    source: &dyn ConversationSource,
    work: crate::assignments::WorkLookup,
    plan: &OpeningPlan<'_>,
) -> Result<String, LinkError> {
    let request = plan.pair;
    if request.a == request.b || request.a.runtime != request.b.runtime {
        return Err(LinkError::Refused(
            "a link needs two distinct threads in the same runtime source".into(),
        ));
    }
    let runtime = instrukt_core::config::Configuration::load_global(home)?
        .runner_runtime()?
        .ok_or(crate::sessions::SessionsError::NoRuntime)?;
    if request.a.runtime.as_str() != runtime.name() {
        return Err(LinkError::Runtime(
            crate::sessions::SessionsError::UnknownRuntime(request.a.runtime.to_string()),
        ));
    }
    let agents = Agents::load(home)?;
    let a_assignment = crate::assignments::resolve(home, request.a, work)?;
    let b_assignment = crate::assignments::resolve(home, request.b, work)?;
    for (member, assignment) in [
        (request.a, a_assignment.as_ref()),
        (request.b, b_assignment.as_ref()),
    ] {
        if let Some(assignment) = assignment {
            if agents.capabilities(assignment.role.as_str())?.class == RoleClass::Focused {
                return Err(LinkError::Refused(format!(
                    "focused role `{}` on {member} reports through its own output and assigned artifacts",
                    assignment.role
                )));
            }
            if assignment
                .dispatcher
                .as_ref()
                .is_some_and(|s| s == request.a || s == request.b)
            {
                return Err(LinkError::Refused(
                    "a worker reports to its dispatcher through native child-result delivery"
                        .into(),
                ));
            }
        }
    }
    for link in status(home)?.into_iter().filter(|link| link.active) {
        if (&link.a == request.a && &link.b == request.b)
            || (&link.a == request.b && &link.b == request.a)
        {
            return Ok(link.id);
        }
        if [&link.a, &link.b]
            .iter()
            .any(|member| **member == *request.a || **member == *request.b)
        {
            return Err(LinkError::Refused(format!(
                "{} is already linked with {}; close that conversation before adding a third peer",
                link.a, link.b
            )));
        }
    }
    let (a, b) = if request.a.to_string() < request.b.to_string() {
        (request.a, request.b)
    } else {
        (request.b, request.a)
    };
    let mut observed = std::collections::BTreeSet::new();
    let mut position = None;
    let mut seen = std::collections::BTreeMap::<String, std::collections::BTreeSet<String>>::new();
    loop {
        let batch = source.changed_since(position.as_ref())?;
        if let Some(failure) = batch.turn_failures.iter().find(|failure| {
            failure.conversation == request.a.id || failure.conversation == request.b.id
        }) {
            return Err(LinkError::Refused(format!(
                "completed-turn evidence unavailable: {}",
                failure.cause
            )));
        }
        observed.extend(batch.conversations.into_iter().map(|id| id.to_string()));
        for turn in batch.completed_turns {
            seen.entry(turn.conversation.to_string())
                .or_default()
                .insert(turn.native_turn.to_string());
        }
        position = Some(batch.position);
        if !batch.has_more {
            break;
        }
    }
    if !observed.contains(a.id.as_str()) || !observed.contains(b.id.as_str()) {
        return Err(LinkError::Refused("one of the native source conversations is missing; restore or select the native threads before linking".into()));
    }
    let position =
        position.ok_or_else(|| LinkError::Refused("source supplied no opening position".into()))?;
    let contribution = plan.contribution.map(|prepare| prepare()).transpose()?;
    let id = crate::keyed::request_id()?;
    let mut db = StateDb::open(home)?;
    let opened = db.write::<_,LinkError>(|tx| {
        let mut q=tx.prepare("SELECT id,a,b FROM conversation_links WHERE active=1 AND (a IN (?1,?2) OR b IN (?1,?2))")?;
        let rows=q.query_map(params![a.to_string(),b.to_string()],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
        if let Some((existing,x,y)) = rows.into_iter().next() {
            if x==a.to_string() && y==b.to_string(){return Ok(existing)}
            return Err(LinkError::Refused(format!("{x} is already linked with {y}; close that conversation before adding a third peer")))
        }
        tx.execute("INSERT INTO conversation_links VALUES (?1,?2,?3,?4,1,NULL)",params![id,source.identity(),a.to_string(),b.to_string()])?;
        for (from,to) in [(a,b),(b,a)] {
            let pending=if from==request.a {contribution.as_ref().map(|pending| {let mut pending=pending.clone();pending.turn=format!("opening:{id}");vec![pending]}).unwrap_or_default()}else{Vec::new()};
            tx.execute("INSERT INTO conversation_relay_positions(link,source,target,position_json,seen_json,pending_json) VALUES (?1,?2,?3,?4,?5,?6)",params![id,from.to_string(),to.to_string(),serde_json::to_string(&position)?,serde_json::to_string(&seen.get(from.id.as_str()).cloned().unwrap_or_default())?,serde_json::to_string(&pending)?])?;
        }
        Ok(id.clone())
    })?;
    if opened == id {
        tracing::info!(event="conversation_opened",link=%id,source=%request.a,target=%request.b,position=%position.reference());
    }
    Ok(opened)
}

pub fn close(home: &Home, member: &SessionRef) -> Result<(), LinkError> {
    let current = status(home)?
        .into_iter()
        .find(|link| link.active && (&link.a == member || &link.b == member));
    let Some(current) = current else {
        return Ok(());
    };
    let _lock = link_lock(home, &current.id)?;
    close_link(home, &current.id, member)
}
fn link_lock(home: &Home, id: &str) -> Result<instrukt_core::fsx::LockGuard, LinkError> {
    Ok(instrukt_core::fsx::LockGuard::acquire(
        &home.locks_dir().join(format!("conversation-{id}.lock")),
        std::time::Duration::from_secs(120),
    )?)
}
fn close_link(home: &Home, id: &str, member: &SessionRef) -> Result<(), LinkError> {
    let mut db = StateDb::open(home)?;
    let changed = db.write::<_, LinkError>(|tx| {
        Ok(tx.execute(
            "UPDATE conversation_links SET active=0 WHERE id=?1 AND active=1 AND (a=?2 OR b=?2)",
            params![id, member.to_string()],
        )?)
    })?;
    if changed > 0 {
        _diagnostics::closed(home, id, member)?;
    }
    Ok(())
}

pub fn status(home: &Home) -> Result<Vec<Status>, LinkError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(Vec::new());
    };
    let mut q = db
        .conn()
        .prepare("SELECT id,a,b,active,failure FROM conversation_links ORDER BY rowid")?;
    let rows = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, Option<String>>(4)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut result = Vec::new();
    for (id, a, b, active, failure) in rows {
        let mut q=db.conn().prepare("SELECT source,target,position_json,delivered_turn,pending_json,failure,receipts_json FROM conversation_relay_positions WHERE link=?1 ORDER BY source")?;
        let directions = q
            .query_map([&id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                    r.get::<_, String>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .map(
                |(source, target, pos, delivered_turn, pending, failure, receipts)| {
                    let pending = serde_json::from_str::<Vec<Pending>>(&pending)?;
                    Ok(Progress {
                        source,
                        target,
                        observed: serde_json::from_str(&pos)?,
                        delivered_turn: delivered_turn.clone(),
                        retained_turn: pending
                            .first()
                            .map(|p| p.turn.clone())
                            .or_else(|| delivered_turn.clone()),
                        pending: pending.len(),
                        uncertain: pending.iter().any(|item| item.sending),
                        receipts: serde_json::from_str(&receipts)?,
                        failure,
                    })
                },
            )
            .collect::<Result<Vec<_>, LinkError>>()?;
        result.push(Status {
            id,
            a: a.parse()
                .map_err(|e| LinkError::Refused(format!("invalid link member: {e}")))?,
            b: b.parse()
                .map_err(|e| LinkError::Refused(format!("invalid link member: {e}")))?,
            active,
            failure,
            directions,
        });
    }
    Ok(result)
}
fn directions(home: &Home) -> Result<Vec<Direction>, LinkError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(Vec::new());
    };
    let mut q=db.conn().prepare("SELECT p.link,p.source,p.target,p.position_json,p.seen_json,p.pending_json,p.delivered_turn,p.receipts_json FROM conversation_relay_positions p JOIN conversation_links l ON l.id=p.link WHERE l.active=1 ORDER BY l.rowid,p.source")?;
    let rows = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    rows.into_iter()
        .map(
            |(link, source, target, position, seen, pending, delivered, receipts)| {
                Ok(Direction {
                    link,
                    source: source
                        .parse()
                        .map_err(|e| LinkError::Refused(format!("invalid source: {e}")))?,
                    target: target
                        .parse()
                        .map_err(|e| LinkError::Refused(format!("invalid target: {e}")))?,
                    position: serde_json::from_str(&position)?,
                    seen: serde_json::from_str(&seen)?,
                    pending: serde_json::from_str(&pending)?,
                    delivered,
                    receipts: serde_json::from_str(&receipts)?,
                })
            },
        )
        .collect()
}
fn retain(home: &Home, d: &Direction) -> Result<(), LinkError> {
    write_direction(home, d, None)
}
fn write_direction(home: &Home, d: &Direction, origin: Option<&Origin>) -> Result<(), LinkError> {
    let mut db = StateDb::open(home)?;
    db.write::<_,LinkError>(|tx| {
        tx.execute("UPDATE conversation_relay_positions SET position_json=?3,seen_json=?4,pending_json=?5,delivered_turn=?6,receipts_json=?7,failure=NULL WHERE link=?1 AND source=?2",params![d.link,d.source.to_string(),serde_json::to_string(&d.position)?,serde_json::to_string(&d.seen)?,serde_json::to_string(&d.pending)?,d.delivered,serde_json::to_string(&d.receipts)?])?;
        if matches!(origin,Some(Origin::Closing{..})) {tx.execute("UPDATE conversation_links SET active=0 WHERE id=?1",[&d.link])?;}
        Ok(())
    })?;
    if matches!(origin, Some(Origin::Closing { .. })) {
        _diagnostics::closed(home, &d.link, &d.source)?;
    }
    Ok(())
}

fn active(home: &Home, id: &str) -> Result<bool, LinkError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(false);
    };
    Ok(db
        .conn()
        .query_row(
            "SELECT active FROM conversation_links WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(false))
}
fn failure(home: &Home, link: &str, source: Option<&str>, message: &str) -> Result<(), LinkError> {
    let mut db = StateDb::open(home)?;
    db.write::<_,LinkError>(|tx|{match source {Some(source)=>{tx.execute("UPDATE conversation_relay_positions SET failure=?3 WHERE link=?1 AND source=?2",params![link,source,message])?;},None=>{tx.execute("UPDATE conversation_links SET failure=?2 WHERE id=?1",params![link,message])?;}} Ok(())})
}
/// One catch-up on native source change. No scheduled/tick caller invokes this path.
pub fn relay(
    home: &Home,
    source: &dyn ConversationSource,
    adapter: &dyn RuntimeAdapter,
) -> Result<(), LinkError> {
    for initial in directions(home)? {
        let _lock = link_lock(home, &initial.link)?;
        let Some(mut d) = directions(home)?
            .into_iter()
            .find(|d| d.link == initial.link && d.source == initial.source)
        else {
            continue;
        };
        let result = relay_direction(home, source, adapter, &mut d);
        if result.is_ok() {
            let mut db = StateDb::open(home)?;
            db.write::<_, LinkError>(|tx| {
                tx.execute(
                    "UPDATE conversation_links SET failure=NULL WHERE id=?1",
                    [&d.link],
                )?;
                Ok(())
            })?;
        }
        if let Err(error) = result {
            _diagnostics::relay_failure(home, &d, &error)?;
        }
    }
    Ok(())
}
fn relay_direction(
    home: &Home,
    source: &dyn ConversationSource,
    adapter: &dyn RuntimeAdapter,
    d: &mut Direction,
) -> Result<(), LinkError> {
    {
        let db = StateRead::open(home)?
            .ok_or_else(|| LinkError::Refused("link state disappeared".into()))?;
        let identity: String = db.conn().query_row(
            "SELECT source_identity FROM conversation_links WHERE id=?1",
            [&d.link],
            |r| r.get(0),
        )?;
        if identity != source.identity() {
            return Err(LinkError::Refused(
                "selected conversation source changed; restore the recorded source".into(),
            ));
        }
    }
    deliver(home, adapter, d, DeliveryMode::Relay)?;
    loop {
        if !active(home, &d.link)? {
            return Ok(());
        }
        let batch = source.changed_since(Some(&d.position))?;
        if batch.missing.iter().any(|id| id == &d.source.id) {
            return Err(LinkError::Refused(
                "linked source conversation is missing; restore its native source content".into(),
            ));
        }
        if let Some(failure) = batch
            .turn_failures
            .iter()
            .find(|failure| failure.conversation == d.source.id)
        {
            return Err(LinkError::Refused(format!(
                "completed-turn evidence unavailable: {}",
                failure.cause
            )));
        }
        {
            let turns = _turns::finished(
                &batch.completed_turns,
                d.source.id.as_str(),
                &format!("{}:{}", source.identity(), d.source.id),
                &format!("{}:{}", source.identity(), d.target.id),
            );
            for (id, content) in turns {
                if d.seen.insert(id.clone()) {
                    d.pending.push(Pending {
                        turn: id,
                        content,
                        sending: false,
                        origin: Origin::Turn,
                    });
                }
            }
        }
        d.position = batch.position;
        retain(home, d)?;
        deliver(home, adapter, d, DeliveryMode::Relay)?;
        if !batch.has_more {
            break;
        }
    }
    Ok(())
}
fn deliver(
    home: &Home,
    adapter: &dyn RuntimeAdapter,
    d: &mut Direction,
    mode: DeliveryMode,
) -> Result<(), LinkError> {
    let mut summary = _diagnostics::Deliveries::new(d);
    while let Some(p) = d.pending.first() {
        if matches!(mode, DeliveryMode::Management) && matches!(p.origin, Origin::Turn) {
            return Ok(());
        }
        if !active(home, &d.link)? {
            return Ok(());
        }
        if p.content.is_empty() && !matches!(p.origin, Origin::Turn) {
            return Err(LinkError::Refused(
                "retained management contribution lost its framing; recover the link handover"
                    .into(),
            ));
        }
        if p.content.is_empty() {
            let completed = d.pending.remove(0);
            d.receipts
                .insert(completed.turn.clone(), Disposition::Empty);
            d.delivered = Some(completed.turn);
            retain(home, d)?;
            summary.completed(d, false);
            continue;
        }
        if p.sending {
            let observation = adapter
                .observe_input(&d.target, &p.content, jiff::Timestamp::now())
                .map_err(|error| LinkError::Native {
                    stage: "acceptance observation",
                    diagnostic: Box::new(adapter.diagnostic("observe", &error)),
                })?;
            if observation.state == crate::runtime::InputState::Unknown {
                return Err(LinkError::Refused(format!(
                    "handover for turn {} remains uncertain; recover native acceptance evidence",
                    p.turn
                )));
            }
        } else {
            d.pending[0].sending = true;
            retain(home, d)?;
            if !active(home, &d.link)? {
                return Ok(());
            }
            adapter
                .send(&d.target, &d.pending[0].content)
                .map_err(|error| LinkError::Native {
                    stage: "send",
                    diagnostic: Box::new(adapter.diagnostic("send", &error)),
                })?;
        }
        let completed = d.pending.remove(0);
        d.receipts.insert(
            completed.turn.clone(),
            Disposition::Accepted {
                at: jiff::Timestamp::now(),
            },
        );
        if matches!(completed.origin, Origin::Turn) {
            d.delivered = Some(completed.turn);
        }
        write_direction(home, d, Some(&completed.origin))?;
        summary.completed(d, true);
    }
    Ok(())
}
