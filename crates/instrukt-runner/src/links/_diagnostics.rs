//! Correlate opaque positions and native references without logging conversation content.
use super::{Direction, LinkError, Opening, status};
use instrukt_core::{
    home::Home,
    ids::SessionRef,
    state_db::{StateDb, rusqlite::params},
};

pub(super) fn relay_failure(
    home: &Home,
    direction: &Direction,
    error: &LinkError,
) -> Result<(), LinkError> {
    let mut db = StateDb::open(home)?;
    let message = error.to_string();
    db.write::<_, LinkError>(|tx| {
        let previous: Option<String> = tx.query_row("SELECT failure FROM conversation_relay_positions WHERE link=?1 AND source=?2", params![direction.link, direction.source.to_string()], |row| row.get(0))?;
        if previous.as_deref() != Some(message.as_str()) {
            tracing::error!(event="conversation_relay_failed",link=%direction.link,source=%direction.source,target=%direction.target,turn=?direction.pending.first().map(|p|p.turn.as_str()).or(direction.delivered.as_deref()),position=%direction.position.reference(),error=%error);
            tx.execute("UPDATE conversation_relay_positions SET failure=?3 WHERE link=?1 AND source=?2", params![direction.link, direction.source.to_string(), message])?;
        }
        Ok(())
    })
}

pub(super) fn management_failure(
    home: &Home,
    id: &str,
    pair: &Opening<'_>,
    error: &LinkError,
) -> Result<(), LinkError> {
    let links = status(home)?;
    let direction = links.iter().find(|link| link.id == id).and_then(|link| {
        link.directions
            .iter()
            .find(|d| d.source == pair.a.to_string())
    });
    tracing::error!(event="discussion_management_failed",link=id,source=%pair.a,target=%pair.b,turn=?direction.and_then(|d|d.retained_turn.as_deref()),position=%direction.map(|d|d.observed.reference()).unwrap_or_else(|| "unavailable".into()),error=%error);
    Ok(())
}
pub(super) fn closed(home: &Home, id: &str, member: &SessionRef) -> Result<(), LinkError> {
    if let Some(link) = status(home)?.into_iter().find(|link| link.id == id) {
        let target = if link.a == *member { &link.b } else { &link.a };
        tracing::info!(event="conversation_closed",link=id,source=%member,target=%target);
    }
    Ok(())
}
pub(super) struct Deliveries {
    link: String,
    source: SessionRef,
    target: SessionRef,
    started: std::time::Instant,
    accepted: usize,
    empty: usize,
    position: String,
}
impl Deliveries {
    pub(super) fn new(direction: &Direction) -> Self {
        Self {
            link: direction.link.clone(),
            source: direction.source.clone(),
            target: direction.target.clone(),
            started: std::time::Instant::now(),
            accepted: 0,
            empty: 0,
            position: direction.position.reference(),
        }
    }
    pub(super) fn completed(&mut self, direction: &Direction, accepted: bool) {
        if accepted {
            self.accepted += 1;
        } else {
            self.empty += 1;
        }
        self.position = direction.position.reference();
    }
}
impl Drop for Deliveries {
    fn drop(&mut self) {
        if self.accepted + self.empty > 0 {
            tracing::debug!(event="conversation_delivery_completed",duration_ms=%self.started.elapsed().as_millis(),link=%self.link,source=%self.source,target=%self.target,accepted=self.accepted,empty=self.empty,position=%self.position);
        }
    }
}
