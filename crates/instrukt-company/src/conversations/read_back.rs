//! Read-back coverage and findings live outside the live intake writer.
use super::{_intake::retain_in, _store::micros, ConversationError};
use instrukt_connector::{
    Connector, ConnectorError, Incoming, ReadBackCoverage, ReadBackGap, ReadBackPage,
};
use instrukt_core::{
    config::SessionScope,
    fsx::{self, FsError},
    home::Home,
    ids::{ProjectSlug, RoleName},
    state_db::{
        self,
        rusqlite::{Connection, OpenFlags, OptionalExtension, params},
    },
};
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};

const ANSWERING_BOUND: SignedDuration = SignedDuration::from_hours(24);
const REPORTING_PERIOD: SignedDuration = SignedDuration::from_hours(30 * 24);
const SCAN_OVERLAP: SignedDuration = SignedDuration::from_secs(60);

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS scans(
    company TEXT, connector TEXT, first_seen INTEGER NOT NULL, retained_until INTEGER,
    started INTEGER, lower_bound INTEGER, position TEXT, page TEXT, retry_at INTEGER, failure TEXT,
    PRIMARY KEY(company, connector)
);
CREATE TABLE IF NOT EXISTS seen(
    company TEXT, connector TEXT, channel TEXT, message TEXT, conversation TEXT,
    written_us INTEGER NOT NULL, missed_link TEXT,
    PRIMARY KEY(company, connector, channel, message)
);
CREATE INDEX IF NOT EXISTS seen_by_time ON seen(written_us);
CREATE TABLE IF NOT EXISTS channel_gaps(
    company TEXT NOT NULL, connector TEXT NOT NULL, channel TEXT NOT NULL,
    direct INTEGER NOT NULL, lower_bound INTEGER NOT NULL, first_scan INTEGER NOT NULL,
    through INTEGER NOT NULL, gap_id TEXT NOT NULL DEFAULT (lower(hex(randomblob(16)))),
    PRIMARY KEY(company,connector,channel)
);
CREATE TABLE IF NOT EXISTS retired_channel_gaps(
    gap_id TEXT NOT NULL PRIMARY KEY,
    company TEXT NOT NULL, connector TEXT NOT NULL, channel TEXT NOT NULL,
    lower_bound INTEGER NOT NULL, through INTEGER NOT NULL, retired_at INTEGER NOT NULL
);
";

fn store_path(home: &Home) -> PathBuf {
    home.state_dir().join("connector-read-back.sqlite")
}

fn open(home: &Home, write: bool) -> Result<Option<Connection>, StateError> {
    use std::os::unix::fs::PermissionsExt as _;
    let path = store_path(home);
    let conn = if write {
        for file in [
            path.clone(),
            path.with_extension("sqlite-journal"),
            path.with_extension("sqlite-wal"),
            path.with_extension("sqlite-shm"),
        ] {
            if std::fs::symlink_metadata(&file).is_ok_and(|meta| meta.file_type().is_symlink()) {
                return Err(StateError::Symlink { path: file });
            }
        }
        let exists = path.exists();
        let dir = home.state_dir();
        fsx::create_dir_all(&dir).map_err(|source| FsError::Write { path: dir, source })?;
        let conn = fsx::open_database(&path, OpenFlags::default()).map_err(state::sql(&path))?;
        conn.busy_timeout(Duration::from_secs(2))
            .map_err(state::sql(&path))?;
        fsx::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).map_err(|source| {
            FsError::Write {
                path: path.clone(),
                source,
            }
        })?;
        if !exists {
            conn.pragma_update(None, "journal_mode", "WAL")
                .map_err(state::sql(&path))?;
        }
        initialize(&conn).map_err(state::sql(&path))?;
        conn
    } else {
        let Some(conn) = state_db::inspect_file(&path)? else {
            return Ok(None);
        };
        conn
    };
    if !write
        && !conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='scans')",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(state::sql(&path))?
    {
        return Ok(None);
    }
    Ok(Some(conn))
}

fn initialize(conn: &Connection) -> instrukt_core::state_db::rusqlite::Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(SCHEMA)?;
    tx.commit()
}

pub(crate) fn observe(
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    at: Timestamp,
) -> Result<(), StateError> {
    let Some(conn) = open(home, true)? else {
        return Ok(());
    };
    observed(&conn, home, company, connector, at)
}

fn observed(
    conn: &Connection,
    home: &Home,
    company: &ProjectSlug,
    connector: &str,
    at: Timestamp,
) -> Result<(), StateError> {
    let path = state::db_path(conn);
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM scans WHERE company=?1 AND connector=?2)",
            [company.as_str(), connector],
            |row| row.get(0),
        )
        .map_err(state::sql(&path))?;
    if !exists {
        // Migration 26 supplies the earliest retained timestamp, never coverage.
        let seed = RunnerState::new(home)
            .read(|intake, path| {
                intake.query_row(
            "SELECT first_seen FROM connector_read_back_seed WHERE company=?1 AND connector=?2",
            [company.as_str(), connector], |row| row.get::<_, Option<i64>>(0),
        ).optional().map_err(state::sql(path))
            })?
            .flatten()
            .flatten();
        conn.execute(
            "INSERT OR IGNORE INTO scans(company,connector,first_seen) VALUES(?1,?2,?3)",
            params![company.as_str(), connector, seed.unwrap_or(micros(at))],
        )
        .map_err(state::sql(&path))?;
    }
    Ok(())
}

struct Progress {
    started: i64,
    lower: i64,
    position: Option<String>,
    page: Option<String>,
    retry_at: Option<i64>,
}

struct ScanWindow {
    from: Timestamp,
    started: Timestamp,
}

fn retry_channels(
    conn: &Connection,
    owner: &str,
    key: &str,
) -> Result<Vec<ReadBackGap>, ConnectorError> {
    let mut query = conn
        .prepare(
            "SELECT channel,direct,lower_bound,first_scan FROM channel_gaps
         WHERE company=?1 AND connector=?2 ORDER BY channel",
        )
        .map_err(failed)?;
    let rows = query
        .query_map([owner, key], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .map_err(failed)?;
    rows.map(|row| {
        let (channel, direct, from, started) = row.map_err(failed)?;
        Ok(ReadBackGap {
            channel,
            direct,
            from: Timestamp::from_microsecond(from).map_err(failed)?,
            window_started: Timestamp::from_microsecond(started).map_err(failed)?,
        })
    })
    .collect()
}

fn retire_expired(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    now: Timestamp,
) -> instrukt_core::state_db::rusqlite::Result<()> {
    let cutoff = micros(now - REPORTING_PERIOD);
    tx.execute(
        "INSERT INTO retired_channel_gaps(gap_id,company,connector,channel,lower_bound,through,retired_at)
         SELECT gap_id,company,connector,channel,lower_bound,min(through,?1),?2 FROM channel_gaps
         WHERE lower_bound<min(through,?1)
         ON CONFLICT(gap_id) DO UPDATE SET through=excluded.through,retired_at=excluded.retired_at
         WHERE excluded.through>retired_channel_gaps.through",
        params![cutoff, micros(now)],
    )?;
    tx.execute("DELETE FROM channel_gaps WHERE through<=?1", [cutoff])?;
    tx.execute(
        "UPDATE channel_gaps SET lower_bound=?1 WHERE lower_bound<?1",
        [cutoff],
    )?;
    tx.execute(
        "DELETE FROM retired_channel_gaps WHERE retired_at<?1 AND NOT EXISTS (
            SELECT 1 FROM channel_gaps WHERE channel_gaps.gap_id=retired_channel_gaps.gap_id
         )",
        [cutoff],
    )?;
    // An interrupted scan may still need identities from its saved window.
    tx.execute(
        "DELETE FROM seen WHERE written_us<?1 AND NOT EXISTS (
            SELECT 1 FROM scans WHERE scans.company=seen.company AND scans.connector=seen.connector
            AND started IS NOT NULL AND seen.written_us>=scans.lower_bound
         )",
        [cutoff],
    )?;
    Ok(())
}

/// An unreadable company remains possible. Its connectors must keep their gaps.
fn configured(home: &Home) -> Result<String, super::ConversationError> {
    use instrukt_core::config::{ConfigError, company_homes};
    let mut configured = Vec::new();
    for (company, config) in company_homes(home)? {
        match config.and_then(|config| config.connectors()) {
            Ok(connectors) => configured.extend(
                connectors
                    .into_iter()
                    .map(|connector| (company.to_string(), Some(connector.key))),
            ),
            Err(ConfigError::NoLayer { .. }) => {}
            Err(_) => configured.push((company.to_string(), None)),
        }
    }
    Ok(serde_json::to_string(&configured)?)
}

fn maintain(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    now: Timestamp,
    configured: Option<&str>,
) -> instrukt_core::state_db::rusqlite::Result<()> {
    if let Some(configured) = configured {
        let absent = "NOT EXISTS(SELECT 1 FROM json_each(?1)
            WHERE value->>0=company AND (value->>1 IS NULL OR value->>1=connector))";
        tx.execute(
            &format!("INSERT INTO retired_channel_gaps(gap_id,company,connector,channel,lower_bound,through,retired_at)
             SELECT gap_id,company,connector,channel,lower_bound,through,?2 FROM channel_gaps
             WHERE {absent}
             ON CONFLICT(gap_id) DO UPDATE SET through=excluded.through,retired_at=excluded.retired_at
             WHERE excluded.through>retired_channel_gaps.through"),
            params![configured, micros(now)],
        )?;
        for table in ["channel_gaps", "seen", "scans"] {
            tx.execute(&format!("DELETE FROM {table} WHERE {absent}"), [configured])?;
        }
    }
    retire_expired(tx, now)
}

pub(crate) fn maintain_at_start(home: &Home, now: Timestamp) -> Result<(), ConnectorError> {
    if !store_path(home).exists() {
        return Ok(());
    }
    let Some(mut conn) = open(home, true).map_err(failed)? else {
        return Ok(());
    };
    let configured = configured(home);
    if let Err(error) = &configured {
        tracing::warn!(event = "read_back_declarations_unreadable", %error);
    }
    let tx = conn.transaction().map_err(failed)?;
    maintain(&tx, now, configured.as_deref().ok()).map_err(failed)?;
    tx.commit().map_err(failed)
}

fn failed(error: impl std::fmt::Display) -> ConnectorError {
    ConnectorError::new(error.to_string())
}

/// Where one scan left the connector's read-back.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Scanned {
    /// It read what it could; `catch_up` asks for a fresh window at once.
    Done { catch_up: bool },
    /// The read waits for the outside gate's wait of its last read, whose
    /// moment `until` is stored. A live pause the service named no moment
    /// for runs out at `pause` on the monotonic source; otherwise the wait
    /// lasts until the host clock reaches `until`.
    Waiting {
        until: Timestamp,
        pause: Option<Instant>,
    },
}

/// The stored moment a read still waits for: one the host clock has not
/// reached, unless it is the pause `waited_out` that this process already
/// waited out on the monotonic source.
fn stored_wait(
    retry_at: Option<i64>,
    now: Timestamp,
    waited_out: Option<Timestamp>,
) -> Option<i64> {
    retry_at
        .filter(|at| micros(now) < *at)
        .filter(|at| waited_out.is_none_or(|out| micros(out) != *at))
}

/// Runs synchronous SQLite/filesystem work off a multi-thread Tokio worker
/// without panicking if called under a `current_thread` test runtime.
fn blocking<R>(work: impl FnOnce() -> R) -> R {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(work)
        }
        _ => work(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn scan(
    home: &Home,
    company: &ProjectSlug,
    key: &str,
    role: &RoleName,
    session_scope: SessionScope,
    connector: &dyn Connector,
    clock: &(dyn Fn() -> Timestamp + Sync),
    quit: &(dyn Fn() -> bool + Sync),
    nudge: &(dyn Fn() + Sync),
    waited_out: Option<Timestamp>,
) -> Result<Scanned, ConnectorError> {
    let done = |catch_up| Scanned::Done { catch_up };
    let requested = clock();
    let owner = company.as_str();
    let Some((mut conn, path, opened, remains)) = blocking(|| {
        let Some(mut conn) = open(home, true).map_err(failed)? else {
            return Ok(None);
        };
        observed(&conn, home, company, key, requested).map_err(failed)?;
        let path = state::db_path(&conn);
        let sql = |error| failed(state::sql(&path)(error));
        let configured = configured(home);
        if let Err(error) = &configured {
            tracing::warn!(event = "read_back_declarations_unreadable", %error);
        }
        let tx = conn.transaction().map_err(sql)?;
        // Expire and prune all connectors, even when only this one scans.
        maintain(&tx, requested, configured.as_deref().ok()).map_err(sql)?;
        let opened = tx
            .execute(
                "UPDATE scans SET started=?3,lower_bound=coalesce(retained_until,min(first_seen,?4))
             WHERE company=?1 AND connector=?2 AND started IS NULL",
                params![
                    owner,
                    key,
                    micros(requested),
                    micros(requested - ANSWERING_BOUND)
                ],
            )
            .map_err(sql)?;

        let remains: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM scans WHERE company=?1 AND connector=?2)",
                [owner, key],
                |row| row.get(0),
            )
            .map_err(sql)?;
        tx.commit().map_err(sql)?;
        Ok::<_, ConnectorError>(Some((conn, path, opened, remains)))
    })?
    else {
        return Ok(done(false));
    };
    let sql = |error| failed(state::sql(&path)(error));
    if opened == 0 && !remains {
        return Ok(done(false));
    }

    while !quit() {
        let progress = blocking(|| {
            conn.query_row(
                "SELECT started,lower_bound,position,page,retry_at FROM scans WHERE company=?1 AND connector=?2",
                [owner, key],
                |row| Ok(Progress { started: row.get(0)?, lower: row.get(1)?, position: row.get(2)?,
                    page: row.get(3)?, retry_at: row.get(4)? }),
            )
            .map_err(sql)
        })?;
        if let Some(at) = stored_wait(progress.retry_at, requested, waited_out) {
            return Ok(Scanned::Waiting {
                until: Timestamp::from_microsecond(at).map_err(failed)?,
                pause: None,
            });
        }
        let window = ScanWindow {
            from: Timestamp::from_microsecond(progress.lower).map_err(failed)?,
            started: Timestamp::from_microsecond(progress.started).map_err(failed)?,
        };
        let page: ReadBackPage = match progress.page {
            Some(page) => serde_json::from_str(&page).map_err(failed)?,
            None => {
                let gaps = if progress.position.is_none() {
                    blocking(|| retry_channels(&conn, owner, key))?
                } else {
                    Vec::new()
                };
                let page_result = connector
                    .read_back_page(window.from, progress.position, &gaps)
                    .await;
                match page_result {
                    Ok(page) => {
                        let encoded = serde_json::to_string(&page).map_err(failed)?;
                        blocking(|| {
                            conn.execute(
                                "UPDATE scans SET page=?3 WHERE company=?1 AND connector=?2",
                                params![owner, key, encoded],
                            )
                            .map_err(sql)
                        })?;
                        page
                    }
                    Err(error) => {
                        let retry = error.wait.as_ref().map(|wait| micros(wait.at()));
                        let message = error.to_string();
                        blocking(|| {
                            conn.execute(
                                "UPDATE scans SET retry_at=?3,failure=?4 WHERE company=?1 AND connector=?2",
                                params![owner, key, retry, message],
                            )
                            .map_err(sql)
                        })?;
                        return match error.wait {
                            Some(wait) => Ok(Scanned::Waiting {
                                until: wait.at(),
                                pause: (!wait.named()).then(|| wait.until()),
                            }),
                            None => Err(failed(error)),
                        };
                    }
                }
            }
        };
        if quit() {
            break;
        }
        match blocking(|| {
            retain_page(
                &mut conn,
                home,
                company,
                key,
                role,
                session_scope,
                &page,
                &window,
                clock,
            )
        }) {
            Ok(true) => nudge(),
            Ok(false) => {}
            Err(error) => {
                let message = error.to_string();
                blocking(|| {
                    conn.execute(
                        "UPDATE scans SET failure=?3 WHERE company=?1 AND connector=?2",
                        params![owner, key, message],
                    )
                    .map_err(sql)
                })?;
                return Err(error);
            }
        }
        if page.next.is_none() {
            // A resumed window can end before the most recent connection.
            // Take a fresh window once its earlier coverage is durable.
            return Ok(done(window.started < requested));
        }
    }
    Ok(done(false))
}

fn channel(item: &Incoming) -> &str {
    item.conversation
        .split_once('/')
        .map_or(&item.conversation, |(channel, _)| channel)
}

/// Page contents remain durable until intake commits and scan progress advances.
/// If the process stops between the two, the intake lookup recognises the commit.
#[allow(clippy::too_many_arguments)]
fn retain_page(
    conn: &mut Connection,
    home: &Home,
    company: &ProjectSlug,
    key: &str,
    role: &RoleName,
    session_scope: SessionScope,
    page: &ReadBackPage,
    window: &ScanWindow,
    clock: &dyn Fn() -> Timestamp,
) -> Result<bool, ConnectorError> {
    let window_started = window.started;
    let answer_from = page.window_started.unwrap_or(window_started) - ANSWERING_BOUND;
    let owner = company.as_str();
    let path = state::db_path(conn);
    let sql = |error| failed(state::sql(&path)(error));
    let mut unique = HashSet::new();
    let mut novel = Vec::new();
    for (item, link) in &page.items {
        if item.connector != key {
            return Err(failed("unexpected connector during read-back"));
        }
        if item.written_at > window_started
            || item.written_at < window_started - REPORTING_PERIOD
            || !unique.insert((channel(item), item.message.as_str()))
        {
            continue;
        }
        let seen: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM seen WHERE company=?1 AND connector=?2 AND channel=?3 AND message=?4)",
            params![owner, key, channel(item), item.message], |row| row.get(0),
        ).map_err(sql)?;
        if !seen {
            novel.push((item, link));
        }
    }

    let (inserted, missed) = if novel.is_empty() {
        (false, Vec::new())
    } else {
        RunnerState::new(home)
            .write::<_, StateError>(|tx| {
                // The intake transaction owns the actual arrival time. The saved
                // window only decides which messages it admits.
                let arrived_at = clock();
                let mut inserted = false;
                let mut missed = Vec::with_capacity(novel.len());
                for (item, link) in &novel {
                    if item.written_at >= answer_from {
                        inserted |=
                            retain_in(tx, company, item, role, session_scope, arrived_at)? != 0;
                        missed.push(None);
                    } else {
                        missed.push(miss_in(tx, company, item)?.then_some(*link));
                    }
                }
                tx.execute(
                    "DELETE FROM conversation_missed_identities WHERE rowid IN (
                    SELECT rowid FROM conversation_missed_identities
                    WHERE written_us<?1 ORDER BY written_us LIMIT 100
                )",
                    [micros(window_started - REPORTING_PERIOD)],
                )
                .map_err(state::sql(&state::db_path(tx)))?;
                Ok((inserted, missed))
            })
            .map_err(failed)?
    };

    let tx = conn.transaction().map_err(sql)?;
    for ((item, _), missed) in novel.into_iter().zip(missed) {
        tx.execute(
            "INSERT INTO seen(company,connector,channel,message,conversation,written_us,missed_link)
             VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT DO NOTHING",
            params![owner, key, channel(item), item.message, item.conversation, micros(item.written_at), missed],
        ).map_err(sql)?;
    }
    match &page.coverage {
        Some(ReadBackCoverage::ChannelUnavailable {
            channel,
            direct,
            expected: true,
        }) => {
            tx.execute(
                "INSERT INTO channel_gaps(company,connector,channel,direct,lower_bound,first_scan,through,gap_id)
                 VALUES(?1,?2,?3,?4,?5,?6,?6,lower(hex(randomblob(16))))
                 ON CONFLICT(company,connector,channel) DO UPDATE SET through=max(through,excluded.through)",
                params![owner, key, channel, direct, micros(window.from), micros(window.started)],
            ).map_err(sql)?;
        }
        Some(ReadBackCoverage::ChannelCovered(channel)) => {
            // The provider can take long enough for part of the gap to
            // expire during the same scan. Report that prefix before clearing.
            retire_expired(&tx, clock()).map_err(sql)?;
            tx.execute(
                "DELETE FROM channel_gaps WHERE company=?1 AND connector=?2 AND channel=?3",
                params![owner, key, channel],
            )
            .map_err(sql)?;
        }
        Some(ReadBackCoverage::ChannelUnavailable {
            expected: false, ..
        })
        | None => {}
    }
    if let Some(next) = &page.next {
        tx.execute("UPDATE scans SET position=?3,page=NULL,retry_at=NULL,failure=NULL WHERE company=?1 AND connector=?2",
            params![owner, key, next]).map_err(sql)?;
    } else {
        tx.execute("UPDATE scans SET retained_until=?3,started=NULL,lower_bound=NULL,position=NULL,page=NULL,retry_at=NULL,failure=NULL
                    WHERE company=?1 AND connector=?2",
            params![owner, key, micros(window_started - SCAN_OVERLAP)]).map_err(sql)?;
        tx.execute(
            "DELETE FROM seen WHERE company=?1 AND connector=?2 AND written_us<?3",
            params![owner, key, micros(window_started - REPORTING_PERIOD)],
        )
        .map_err(sql)?;
    }
    tx.commit().map_err(sql)?;
    Ok(inserted)
}

fn miss_in(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    company: &ProjectSlug,
    item: &Incoming,
) -> Result<bool, StateError> {
    let path = state::db_path(tx);
    let identity = params![
        company.as_str(),
        item.connector,
        item.conversation,
        item.message,
        micros(item.written_at)
    ];
    tx.execute(
        "INSERT INTO conversation_missed_identities(company,connector,conversation,message,written_us)
         SELECT ?1,?2,?3,?4,?5 WHERE NOT EXISTS(
             SELECT 1 FROM conversation_messages WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4
             UNION ALL SELECT 1 FROM conversation_context_receipts
             WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4 AND delivery=message
         ) ON CONFLICT DO NOTHING",
        identity,
    ).map_err(state::sql(&path))?;
    // A durable missed marker also recovers a crash after the intake
    // transaction committed and before the scan recorded its finding.
    tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM conversation_missed_identities
         WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4)",
        params![
            company.as_str(),
            item.connector,
            item.conversation,
            item.message
        ],
        |row| row.get(0),
    )
    .map_err(state::sql(&path))
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

pub(super) fn missed(home: &Home, now: Timestamp) -> Result<Vec<Missed>, ConversationError> {
    let Some(conn) = open(home, false)? else {
        return Ok(Vec::new());
    };
    let path = state::db_path(&conn);
    let mut query = conn.prepare(
        "SELECT company,connector,conversation,message,written_us,missed_link
         FROM seen WHERE missed_link IS NOT NULL AND written_us>=?1 ORDER BY written_us,company,connector,conversation,message",
    ).map_err(state::sql(&path))?;
    let rows = query
        .query_map([micros(now - REPORTING_PERIOD)], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .map_err(state::sql(&path))?;
    rows.map(|row| {
        let (company, connector, conversation, message, written_us, link) =
            row.map_err(state::sql(&path))?;
        let written_at = Timestamp::from_microsecond(written_us)
            .map_err(|err| ConversationError::Refused(err.to_string()))?;
        Ok(Missed {
            company,
            connector,
            conversation,
            message,
            written_at,
            link,
        })
    })
    .collect()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Failure {
    pub company: String,
    pub connector: String,
    pub cause: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at: Option<Timestamp>,
}

pub(super) fn failures(home: &Home) -> Result<Vec<Failure>, StateError> {
    let Some(conn) = open(home, false)? else {
        return Ok(Vec::new());
    };
    let path = state::db_path(&conn);
    let mut query = conn.prepare(
        "SELECT company,connector,failure,retry_at FROM scans WHERE failure IS NOT NULL ORDER BY company,connector",
    ).map_err(state::sql(&path))?;
    let rows = query
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(state::sql(&path))?;
    rows.map(|row| {
        let (company, connector, cause, retry_at) = row.map_err(state::sql(&path))?;
        let retry_at = retry_at
            .map(Timestamp::from_microsecond)
            .transpose()
            .map_err(|err| state::invalid(&path, err))?;
        Ok(Failure {
            company,
            connector,
            cause,
            retry_at,
        })
    })
    .collect()
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChannelGap {
    pub company: String,
    pub connector: String,
    pub channel: String,
    pub from: Timestamp,
    pub through: Timestamp,
    pub cause: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retired_at: Option<Timestamp>,
}

pub(super) fn channel_gaps(home: &Home, now: Timestamp) -> Result<Vec<ChannelGap>, StateError> {
    let Some(conn) = open(home, false)? else {
        return Ok(Vec::new());
    };
    let path = state::db_path(&conn);
    // A read-only inspection can precede opening an earlier scan database
    // for writes, when its new tables have not been created.
    let exists: bool = conn
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='channel_gaps')",
            [],
            |row| row.get(0),
        )
        .map_err(state::sql(&path))?;
    if !exists {
        return Ok(Vec::new());
    }
    let mut query = conn
        .prepare(
            "SELECT company,connector,channel,lower_bound,through,NULL FROM channel_gaps
         UNION ALL SELECT company,connector,channel,lower_bound,through,retired_at
         FROM retired_channel_gaps WHERE retired_at>=?1
         ORDER BY company,connector,channel,lower_bound",
        )
        .map_err(state::sql(&path))?;
    let rows = query
        .query_map([micros(now - REPORTING_PERIOD)], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get::<_, Option<i64>>(5)?,
            ))
        })
        .map_err(state::sql(&path))?;
    let time = |us| Timestamp::from_microsecond(us).map_err(|err| state::invalid(&path, err));
    rows.map(|row| {
        let (company, connector, channel, from, through, retired_at) =
            row.map_err(state::sql(&path))?;
        let cause = if retired_at.is_some() {
            "Messages may have been missed in this span."
        } else {
            "Channel unavailable for read-back."
        };
        Ok(ChannelGap {
            company,
            connector,
            channel,
            from: time(from)?,
            through: time(through)?,
            cause: cause.into(),
            retired_at: retired_at.map(time).transpose()?,
        })
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::{micros, stored_wait};
    use jiff::{SignedDuration, Timestamp};

    #[test]
    fn a_stored_moment_holds_until_the_host_clock_reaches_it() {
        let now: Timestamp = "2026-11-20T10:00:00Z".parse().unwrap();
        let at = now + SignedDuration::from_secs(120);
        assert_eq!(stored_wait(Some(micros(at)), now, None), Some(micros(at)));
        assert_eq!(stored_wait(Some(micros(at)), at, None), None);
        assert_eq!(stored_wait(None, now, None), None);
    }

    #[test]
    fn a_pause_waited_out_on_the_monotonic_source_no_longer_holds() {
        let now: Timestamp = "2026-11-20T10:00:00Z".parse().unwrap();
        let at = now + SignedDuration::from_secs(2);
        assert_eq!(stored_wait(Some(micros(at)), now, Some(at)), None);
        let other = now + SignedDuration::from_secs(1);
        assert_eq!(
            stored_wait(Some(micros(at)), now, Some(other)),
            Some(micros(at))
        );
    }
}
