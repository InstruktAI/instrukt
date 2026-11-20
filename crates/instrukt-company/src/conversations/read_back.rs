//! Durable coverage and operator findings for offline reads.
use super::{ConversationError, micros, retain_in};
use instrukt_connector::{Connector, ConnectorError, Incoming};
use instrukt_core::{
    home::Home,
    ids::ProjectSlug,
    state_db::rusqlite::{Transaction, params},
};
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

const ANSWERING_BOUND: SignedDuration = SignedDuration::from_hours(24);
const REPORTING_PERIOD: SignedDuration = SignedDuration::from_hours(30 * 24);
const SCAN_OVERLAP: SignedDuration = SignedDuration::from_secs(60);

pub(crate) fn observe(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    at: Timestamp,
) -> Result<(), StateError> {
    RunnerState::new(home).write(|tx| {
        tx.execute(
            "INSERT INTO connector_read_back(company,connector,first_seen) VALUES(?1,?2,?3)
             ON CONFLICT(company,connector) DO UPDATE SET first_seen=coalesce(first_seen,excluded.first_seen)",
            params![company.as_str(), connector, micros(at)],
        ).map_err(state::sql(&state::db_path(tx)))?;
        Ok(())
    })
}

pub(crate) fn scan(
    home: &Home,
    company: &ProjectSlug,
    key: &str,
    connector: &dyn Connector,
    started: Timestamp,
    quit: &dyn Fn() -> bool,
    nudge: &dyn Fn(),
) -> Result<(), ConnectorError> {
    let state = RunnerState::new(home);
    let failed = |error: StateError| ConnectorError {
        cause: error.to_string(),
    };
    observe(home, company, key, started).map_err(failed)?;
    let oldest = micros(started - ANSWERING_BOUND);
    let from = state.read(|conn, path| {
        conn.query_row(
            "SELECT coalesce(retained_until,min(first_seen,?3)) FROM connector_read_back WHERE company=?1 AND connector=?2",
            params![company.as_str(), key, oldest],
            |row| row.get::<_, i64>(0),
        ).map_err(state::sql(path))
    }).map_err(failed)?.unwrap_or(oldest);
    let from = Timestamp::from_microsecond(from).map_err(|error| ConnectorError {
        cause: error.to_string(),
    })?;

    // Keep the final item for the cursor transaction. All earlier items
    // survive a scan interrupted between native pages.
    let mut last = None;
    let result = connector.read_back(from, quit, &mut |incoming, link| {
        if incoming.connector != key {
            return Err(ConnectorError {
                cause: "unexpected connector during read-back".into(),
            });
        }
        if let Some((previous, link)) = last.replace((incoming, link)) {
            state
                .write(|tx| classify(tx, company, &previous, &link, started))
                .map_err(failed)?;
            nudge();
        }
        Ok(())
    });
    if result.is_ok() && quit() {
        return Err(ConnectorError {
            cause: "read-back stopped".into(),
        });
    }
    state.write(|tx| {
        let path = state::db_path(tx);
        if let Some((last, link)) = last {
            classify(tx, company, &last, &link, started)?;
        }
        if result.is_ok() {
            tx.execute(
                "UPDATE connector_read_back SET retained_until=?3,failure=NULL WHERE company=?1 AND connector=?2",
                params![company.as_str(), key, micros(started - SCAN_OVERLAP)],
            ).map_err(state::sql(&path))?;
        } else {
            tx.execute(
                "UPDATE connector_read_back SET failure=?3 WHERE company=?1 AND connector=?2",
                params![company.as_str(), key, result.as_ref().err().map(|e| e.cause.as_str())],
            ).map_err(state::sql(&path))?;
        }
        tx.execute("DELETE FROM connector_missed_messages WHERE written_us<?1",
            [micros(started - REPORTING_PERIOD)]).map_err(state::sql(&path))?;
        Ok(())
    }).map_err(failed)?;
    nudge();
    result
}

fn classify(
    tx: &Transaction<'_>,
    company: &ProjectSlug,
    item: &Incoming,
    link: &str,
    at: Timestamp,
) -> Result<(), StateError> {
    if item.written_at >= at - ANSWERING_BOUND {
        retain_in(tx, company, item, at)?;
    } else if item.written_at >= at - REPORTING_PERIOD {
        tx.execute(
            "INSERT INTO connector_missed_messages(company,connector,conversation,message,written_us,link)
             SELECT ?1,?2,?3,?4,?5,?6 WHERE NOT EXISTS(
                 SELECT 1 FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
             ) ON CONFLICT DO NOTHING",
            params![company.as_str(), item.connector, item.conversation, item.message, micros(item.written_at), link],
        ).map_err(state::sql(&state::db_path(tx)))?;
    }
    Ok(())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Missed {
    pub company: String,
    pub connector: String,
    pub conversation: String,
    pub message: String,
    pub written_at: Timestamp,
    pub link: String,
}

pub(super) fn missed(
    state: &RunnerState,
    now: Timestamp,
) -> Result<Vec<Missed>, ConversationError> {
    let rows = state.read(|conn, path| {
        let mut query = conn.prepare(
            "SELECT company,connector,conversation,message,written_us,link
             FROM connector_missed_messages WHERE written_us>=?1 ORDER BY written_us,company,connector,conversation,message",
        ).map_err(state::sql(path))?;
        query.query_map([micros(now - REPORTING_PERIOD)], |row| {
            Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?))
        }).map_err(state::sql(path))?.collect::<Result<Vec<_>, _>>().map_err(state::sql(path))
    })?.unwrap_or_default();
    rows.into_iter()
        .map(
            |(company, connector, conversation, message, written_us, link)| {
                Ok(Missed {
                    company,
                    connector,
                    conversation,
                    message,
                    link,
                    written_at: Timestamp::from_microsecond(written_us)
                        .map_err(|error| ConversationError::Refused(error.to_string()))?,
                })
            },
        )
        .collect()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Failure {
    pub company: String,
    pub connector: String,
    pub cause: String,
}

pub(super) fn failures(state: &RunnerState) -> Result<Vec<Failure>, StateError> {
    Ok(state.read(|conn, path| {
        let mut query = conn.prepare(
            "SELECT company,connector,failure FROM connector_read_back WHERE failure IS NOT NULL ORDER BY company,connector",
        ).map_err(state::sql(path))?;
        query.query_map([], |row| Ok(Failure { company: row.get(0)?, connector: row.get(1)?, cause: row.get(2)? }))
            .map_err(state::sql(path))?.collect::<Result<Vec<_>, _>>().map_err(state::sql(path))
    })?.unwrap_or_default())
}
