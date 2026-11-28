//! Write-isolated explicit reply outbox (`state/company-replies.db`) serialized
//! exclusively under `state/company-replies.lock` so `instrukt company reply`
//! never waits on `state/writer.lock`.
use std::time::Duration;

use instrukt_core::config::{Configuration, SessionScope, connector_declaration};
use instrukt_core::home::Home;
use instrukt_core::ids::{InputId, SessionRef};
use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension, params};
use instrukt_core::state_db::{Migration, Read, StateRead, Store, StoreDb, StoreError, StoreLock};
use instrukt_runner::keyed;
use instrukt_runner::runtime::RuntimeAdapter;
use instrukt_runner::state::{self, RunnerState, StateError};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};

use super::_relay::split;
use super::_speaker::{SpeakerError, conversation_binding, resolve_active_turn_within};
use super::{ConversationError, LOG_TARGET};

/// Lock acquisition timeout for `state/company-replies.lock`.
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum wait budget for live turn correlation during `instrukt company reply`.
const REPLY_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(5);

/// Retention age for confirmed-posted rows pruned opportunistically on reply writes.
const PRUNE_AGE: SignedDuration = SignedDuration::from_hours(7 * 24);

/// Maximum number of confirmed-posted expired rows pruned per `instrukt company reply` call.
const PRUNE_BATCH: i64 = 100;

const V1: &str = r#"
CREATE TABLE explicit_replies (
    reply_id TEXT NOT NULL,
    piece INTEGER NOT NULL,
    company TEXT NOT NULL,
    connector TEXT NOT NULL,
    conversation TEXT NOT NULL,
    turn_message TEXT,
    turn_input_id TEXT,
    text TEXT NOT NULL,
    queued_at TEXT NOT NULL,
    PRIMARY KEY (reply_id, piece),
    CHECK ((turn_message IS NULL) = (turn_input_id IS NULL))
);
CREATE INDEX explicit_replies_queued_at ON explicit_replies(queued_at);
"#;

const MIGRATIONS: &[Migration] = &[Migration::Sql(V1)];

const STORE_NAME: &str = "the company explicit replies store";

fn store(home: &Home) -> Store {
    Store {
        name: STORE_NAME,
        file: home.company_replies_db_file(),
        lock: StoreLock::Exclusive(home.company_replies_lock_file(), LOCK_TIMEOUT),
        migrations: MIGRATIONS,
    }
}

fn sql_err(
    path: &std::path::Path,
) -> impl Fn(instrukt_core::state_db::rusqlite::Error) -> StoreError + '_ {
    move |source| StoreError::Sqlite {
        store: STORE_NAME,
        path: path.to_path_buf(),
        source,
    }
}

fn db_path(tx: &instrukt_core::state_db::rusqlite::Transaction<'_>) -> std::path::PathBuf {
    state::db_path(tx)
}

/// Closed origin classification matching the paired-NULL `CHECK` constraint on
/// `explicit_replies(turn_message, turn_input_id)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplyOrigin {
    AddressedTurn { message: String, input_id: InputId },
    DirectOrCrossConversation,
}

impl ReplyOrigin {
    fn columns(&self) -> (Option<&str>, Option<&str>) {
        match self {
            Self::AddressedTurn { message, input_id } => {
                (Some(message.as_str()), Some(input_id.as_str()))
            }
            Self::DirectOrCrossConversation => (None, None),
        }
    }
}

/// Summary of an explicit reply queued into `state/company-replies.db`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedReply {
    pub company: String,
    pub connector: String,
    pub conversation: String,
    pub turn: String,
    pub pieces: usize,
}

/// Queue an explicit reply from an active company conversation session into the
/// write-isolated `state/company-replies.db` outbox, verbatim and split to the
/// connector kind's declared message character limit.
pub fn queue_explicit_reply(
    home: &Home,
    caller: &SessionRef,
    adapter: &dyn RuntimeAdapter,
    requested_conversation: Option<&str>,
    text: &str,
    now: Timestamp,
) -> Result<QueuedReply, ConversationError> {
    let Some(binding) = conversation_binding(home, caller).map_err(StateError::from)? else {
        return Err(ConversationError::Refused(
            "this session is not bound to an active company conversation".into(),
        ));
    };
    if text.trim().is_empty() {
        return Err(ConversationError::Refused(
            "reply text must not be empty".into(),
        ));
    }
    let target = validate_target_conversation(home, &binding, requested_conversation)?;
    let cfg = Configuration::load_company(&home.for_inspection(), &binding.company)?;
    let declared = cfg
        .connectors()?
        .into_iter()
        .find(|declared| declared.key == binding.connector)
        .ok_or_else(|| {
            ConversationError::Refused(format!(
                "connector `{}` is no longer declared for company `{}`",
                binding.connector, binding.company
            ))
        })?;
    let declaration = connector_declaration(&declared.kind).ok_or_else(|| {
        ConversationError::Refused(format!(
            "connector kind `{}` is not registered in this build",
            declared.kind
        ))
    })?;
    let message_chars = declaration.message_chars;
    let parts = split(text, message_chars);
    if parts.is_empty() {
        return Err(ConversationError::Refused(
            "reply text must not be empty".into(),
        ));
    }
    let origin = resolve_reply_origin(home, caller, &binding, &target, adapter)?;
    let reply_id = format!("reply:{}", keyed::request_id()?);
    let mut db = StoreDb::open(&store(home)).map_err(|err| {
        ConversationError::Refused(format!("cannot open company replies outbox: {err}"))
    })?;
    let (turn_message, turn_input_id) = origin.columns();
    let queued_at = now.to_string();
    db.write::<_, StoreError>(|tx| {
        let path = db_path(tx);
        for (piece, part) in parts.iter().enumerate() {
            tx.execute(
                "INSERT INTO explicit_replies(reply_id,piece,company,connector,conversation,turn_message,turn_input_id,text,queued_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    reply_id,
                    i64::try_from(piece).unwrap_or(i64::MAX),
                    binding.company.as_str(),
                    binding.connector,
                    target,
                    turn_message,
                    turn_input_id,
                    part,
                    queued_at,
                ],
            )
            .map_err(sql_err(&path))?;
        }
        Ok(())
    })
    .map_err(|err| ConversationError::Refused(format!("cannot write company replies outbox: {err}")))?;
    prune_posted_replies(home, &mut db, now);
    Ok(QueuedReply {
        company: binding.company.to_string(),
        connector: binding.connector,
        conversation: target,
        turn: reply_id,
        pieces: parts.len(),
    })
}

pub(super) fn validate_target_conversation(
    home: &Home,
    binding: &super::_speaker::ConversationBinding,
    requested_conversation: Option<&str>,
) -> Result<String, ConversationError> {
    match binding.session_scope {
        SessionScope::Conversation => {
            if let Some(requested) = requested_conversation
                && requested != binding.conversation
            {
                return Err(ConversationError::Refused(format!(
                    "conversation `{requested}` is outside this session's bound conversation `{}`",
                    binding.conversation
                )));
            }
            Ok(binding.conversation.clone())
        }
        SessionScope::Company => {
            let requested = requested_conversation
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ConversationError::Refused(
                        "--conversation is required when the connector uses session_scope: company"
                            .into(),
                    )
                })?;
            let admitted = match StateRead::open(home).map_err(StateError::from)? {
                Some(db) => db
                    .conn()
                    .query_row(
                        "SELECT EXISTS(
                            SELECT 1 FROM conversation_messages m
                            WHERE m.company=?1 AND m.connector=?2 AND m.conversation=?3
                              AND m.admitted_at IS NOT NULL AND m.state <> 'refused'
                              AND NOT EXISTS (
                                  SELECT 1 FROM connected_conversations cc
                                  WHERE cc.company=m.company AND cc.connector=m.connector
                                    AND cc.conversation=m.conversation AND cc.session_scope='conversation'
                              )
                        )",
                        params![binding.company.as_str(), binding.connector, requested],
                        |row| row.get::<_, bool>(0),
                    )
                    .map_err(state::sql(db.path()))?,
                None => false,
            };
            if !admitted {
                return Err(ConversationError::Refused(format!(
                    "conversation `{requested}` has no admitted message for connector `{}` in company `{}`",
                    binding.connector, binding.company
                )));
            }
            Ok(requested.to_owned())
        }
    }
}

fn resolve_reply_origin(
    home: &Home,
    caller: &SessionRef,
    binding: &super::_speaker::ConversationBinding,
    target_conversation: &str,
    adapter: &dyn RuntimeAdapter,
) -> Result<ReplyOrigin, ConversationError> {
    match resolve_active_turn_within(home, caller, adapter, REPLY_OBSERVATION_TIMEOUT) {
        Ok(Some(active)) => {
            if binding.session_scope == SessionScope::Company
                && active.conversation != target_conversation
            {
                Ok(ReplyOrigin::DirectOrCrossConversation)
            } else {
                Ok(ReplyOrigin::AddressedTurn {
                    message: active.message,
                    input_id: active.input_id,
                })
            }
        }
        Ok(None) => Ok(ReplyOrigin::DirectOrCrossConversation),
        Err(SpeakerError::NotRunning { ref state })
            if state == instrukt_runtime::SessionState::InputReady.as_str() =>
        {
            Ok(ReplyOrigin::DirectOrCrossConversation)
        }
        Err(error) => Err(ConversationError::Refused(error.to_string())),
    }
}

/// Opportunistically delete up to [`PRUNE_BATCH`] outbox rows older than 7 days
/// that are confirmed posted in `state/instrukt.db`. Transient read errors on
/// `state/instrukt.db` are logged and skipped without failing the reply write.
fn prune_posted_replies(home: &Home, db: &mut StoreDb, now: Timestamp) {
    let cutoff = format!("{:.9}", now - PRUNE_AGE);
    let candidates: Vec<(String, i64, String, String, String)> = match db.conn().prepare(
        "SELECT reply_id,piece,company,connector,conversation
         FROM explicit_replies
         WHERE julianday(queued_at) <= julianday(?1)
         ORDER BY queued_at, reply_id, piece
         LIMIT ?2",
    ) {
        Ok(mut stmt) => match stmt
            .query_map(params![cutoff, PRUNE_BATCH], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(target: LOG_TARGET, event = "company_replies_prune_skipped", %error);
                return;
            }
        },
        Err(error) => {
            tracing::warn!(target: LOG_TARGET, event = "company_replies_prune_skipped", %error);
            return;
        }
    };
    if candidates.is_empty() {
        return;
    }
    let state_read = match StateRead::open(home) {
        Ok(Some(read)) => read,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(target: LOG_TARGET, event = "company_replies_prune_skipped", %error);
            return;
        }
    };
    let mut posted = Vec::new();
    for (reply_id, piece, company, connector, conversation) in candidates {
        match state_read.conn().query_row(
            "SELECT EXISTS(
                SELECT 1 FROM conversation_answers
                WHERE company=?1 AND connector=?2 AND conversation=?3
                  AND turn=?4 AND piece=?5 AND posted_at IS NOT NULL
            )",
            params![company, connector, conversation, reply_id, piece],
            |row| row.get::<_, bool>(0),
        ) {
            Ok(true) => posted.push((reply_id, piece)),
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(target: LOG_TARGET, event = "company_replies_prune_skipped", %error);
                return;
            }
        }
    }
    drop(state_read);
    if posted.is_empty() {
        return;
    }
    if let Err(error) = db.write::<_, StoreError>(|tx| {
        let path = db_path(tx);
        for (reply_id, piece) in &posted {
            tx.execute(
                "DELETE FROM explicit_replies WHERE reply_id=?1 AND piece=?2",
                params![reply_id, piece],
            )
            .map_err(sql_err(&path))?;
        }
        Ok(())
    }) {
        tracing::warn!(target: LOG_TARGET, event = "company_replies_prune_skipped", %error);
    }
}

struct OutboxRow {
    reply_id: String,
    piece: i64,
    company: String,
    connector: String,
    conversation: String,
    turn_message: Option<String>,
    text: String,
    queued_at: String,
}

fn read_all_outbox_rows(conn: &Connection) -> Result<Vec<OutboxRow>, ConversationError> {
    let mut stmt = conn
        .prepare(
            "SELECT reply_id,piece,company,connector,conversation,turn_message,text,queued_at
             FROM explicit_replies
             ORDER BY rowid",
        )
        .map_err(|err| {
            ConversationError::Refused(format!("cannot read company replies outbox: {err}"))
        })?;
    stmt.query_map([], |row| {
        Ok(OutboxRow {
            reply_id: row.get(0)?,
            piece: row.get(1)?,
            company: row.get(2)?,
            connector: row.get(3)?,
            conversation: row.get(4)?,
            turn_message: row.get(5)?,
            text: row.get(6)?,
            queued_at: row.get(7)?,
        })
    })
    .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
    .map_err(|err| ConversationError::Refused(format!("cannot read company replies outbox: {err}")))
}

/// Whether `state/company-replies.db` holds any explicit reply row that has not
/// yet been ingested into `conversation_answers`. Surfaces read errors instead
/// of swallowing them so the relay records outages visibly.
pub fn has_uningested_replies(home: &Home) -> Result<bool, ConversationError> {
    let conn = match store(home).read(false).map_err(|err| {
        ConversationError::Refused(format!("cannot inspect company replies outbox: {err}"))
    })? {
        None => return Ok(false),
        Some(Read::Behind) => return Ok(true),
        Some(Read::Current(conn)) => conn,
    };
    let mut stmt = conn
        .prepare("SELECT company,connector,conversation,reply_id,piece FROM explicit_replies")
        .map_err(|err| {
            ConversationError::Refused(format!("cannot query company replies outbox: {err}"))
        })?;
    let keys = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })
        .and_then(|rows| rows.collect::<Result<Vec<_>, _>>())
        .map_err(|err| {
            ConversationError::Refused(format!("cannot query company replies outbox: {err}"))
        })?;
    if keys.is_empty() {
        return Ok(false);
    }
    let Some(state_read) = StateRead::open(home).map_err(StateError::from)? else {
        return Ok(true);
    };
    let mut check = state_read
        .conn()
        .prepare(
            "SELECT EXISTS(
                SELECT 1 FROM conversation_answers
                WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND piece=?5
            )",
        )
        .map_err(state::sql(state_read.path()))?;
    for (company, connector, conversation, reply_id, piece) in keys {
        let present: bool = check
            .query_row(
                params![company, connector, conversation, reply_id, piece],
                |row| row.get(0),
            )
            .map_err(state::sql(state_read.path()))?;
        if !present {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Ingest any un-ingested rows from `state/company-replies.db` into
/// `state/instrukt.db`'s `conversation_answers` table (carrying `source_message`
/// when linked to an addressed turn). Returns the number of newly ingested rows.
pub fn ingest_explicit_replies(
    home: &Home,
    state: &RunnerState,
) -> Result<usize, ConversationError> {
    let rows = match store(home).read(false).map_err(|err| {
        ConversationError::Refused(format!("cannot read company replies outbox: {err}"))
    })? {
        None => return Ok(0),
        Some(Read::Current(conn)) => read_all_outbox_rows(&conn)?,
        Some(Read::Behind) => {
            let db = StoreDb::open(&store(home)).map_err(|err| {
                ConversationError::Refused(format!("cannot migrate company replies outbox: {err}"))
            })?;
            read_all_outbox_rows(db.conn())?
        }
    };
    if rows.is_empty() {
        return Ok(0);
    }
    let inserted = state.write::<_, StateError>(|tx| {
        let path = state::db_path(tx);
        let mut count = 0usize;
        for row in &rows {
            let exists: bool = tx
                .query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM conversation_answers
                        WHERE company=?1 AND connector=?2 AND conversation=?3 AND turn=?4 AND piece=?5
                    )",
                    params![
                        row.company,
                        row.connector,
                        row.conversation,
                        row.reply_id,
                        row.piece
                    ],
                    |r| r.get(0),
                )
                .map_err(state::sql(&path))?;
            if exists {
                continue;
            }
            let admission_position: i64 = if let Some(source_message) = row.turn_message.as_deref()
            {
                tx.query_row(
                    "SELECT admission_position FROM conversation_messages
                     WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
                    params![row.company, row.connector, row.conversation, source_message],
                    |r| r.get::<_, Option<i64>>(0),
                )
                .optional()
                .map_err(state::sql(&path))?
                .flatten()
            } else {
                None
            }
            .unwrap_or_else(|| {
                tx.query_row(
                    "SELECT coalesce(MAX(admission_position), 0) FROM conversation_messages
                     WHERE company=?1 AND connector=?2 AND conversation=?3",
                    params![row.company, row.connector, row.conversation],
                    |r| r.get(0),
                )
                .unwrap_or(0)
            });
            let changed = tx
                .execute(
                    "INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,admission_position,source_message)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                     ON CONFLICT DO NOTHING",
                    params![
                        row.company,
                        row.connector,
                        row.conversation,
                        row.reply_id,
                        row.piece,
                        row.text,
                        admission_position,
                        row.turn_message,
                    ],
                )
                .map_err(state::sql(&path))?;
            if changed > 0
                && let Some(source_message) = row.turn_message.as_deref()
            {
                tx.execute(
                    "UPDATE conversation_messages
                     SET turn_ended_at=coalesce(turn_ended_at,?5)
                     WHERE company=?1 AND connector=?2 AND conversation=?3 AND message=?4",
                    params![
                        row.company,
                        row.connector,
                        row.conversation,
                        source_message,
                        row.queued_at,
                    ],
                )
                .map_err(state::sql(&path))?;
            }
            count += changed;
        }
        Ok(count)
    })?;
    Ok(inserted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use instrukt_core::state_db::StateDb;

    fn test_home() -> (tempfile::TempDir, Home) {
        let dir = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
        let home = Home::at_user_home(dir.path().to_path_buf()).for_writing();
        (dir, home)
    }

    #[test]
    fn explicit_replies_table_enforces_paired_null_check_constraint() {
        let (_dir, home) = test_home();
        let mut db = StoreDb::open(&store(&home)).unwrap();
        // Paired NULL succeeds.
        db.write::<_, StoreError>(|tx| {
            let path = db_path(tx);
            tx.execute(
                "INSERT INTO explicit_replies VALUES ('r1',0,'acme','slack','C1',NULL,NULL,'hi','2026-11-20T10:00:00Z')",
                [],
            )
            .map_err(sql_err(&path))?;
            Ok(())
        })
        .unwrap();
        // Paired non-NULL succeeds.
        db.write::<_, StoreError>(|tx| {
            let path = db_path(tx);
            tx.execute(
                "INSERT INTO explicit_replies VALUES ('r2',0,'acme','slack','C1','m1','cmd-1','hi','2026-11-20T10:00:00Z')",
                [],
            )
            .map_err(sql_err(&path))?;
            Ok(())
        })
        .unwrap();
        // Mismatched NULL / non-NULL fails CHECK constraint.
        let half_null = db.write::<_, StoreError>(|tx| {
            let path = db_path(tx);
            tx.execute(
                "INSERT INTO explicit_replies VALUES ('r3',0,'acme','slack','C1','m1',NULL,'hi','2026-11-20T10:00:00Z')",
                [],
            )
            .map_err(sql_err(&path))?;
            Ok(())
        });
        assert!(half_null.is_err());
        let other_half = db.write::<_, StoreError>(|tx| {
            let path = db_path(tx);
            tx.execute(
                "INSERT INTO explicit_replies VALUES ('r4',0,'acme','slack','C1',NULL,'cmd-1','hi','2026-11-20T10:00:00Z')",
                [],
            )
            .map_err(sql_err(&path))?;
            Ok(())
        });
        assert!(other_half.is_err());
    }

    #[test]
    fn has_uningested_replies_ignores_already_ingested_retained_rows() {
        let (_dir, home) = test_home();
        drop(StateDb::open(&home).unwrap());
        let mut db = StoreDb::open(&store(&home)).unwrap();
        db.write::<_, StoreError>(|tx| {
            let path = db_path(tx);
            tx.execute(
                "INSERT INTO explicit_replies VALUES ('reply:1',0,'acme','slack','C1','m1','cmd-1','answer','2026-11-20T10:00:00Z')",
                [],
            )
            .map_err(sql_err(&path))?;
            Ok(())
        })
        .unwrap();
        drop(db);

        assert!(has_uningested_replies(&home).unwrap());
        let state = RunnerState::new(&home);
        assert_eq!(ingest_explicit_replies(&home, &state).unwrap(), 1);
        // Retained row in explicit_replies does not keep has_uningested_replies pinned true.
        assert!(!has_uningested_replies(&home).unwrap());
        // Re-running ingest is idempotent.
        assert_eq!(ingest_explicit_replies(&home, &state).unwrap(), 0);

        // Verify source_message propagated into conversation_answers.
        let source_msg: Option<String> = state
            .read(|conn, path| {
                conn.query_row(
                    "SELECT source_message FROM conversation_answers WHERE turn='reply:1' AND piece=0",
                    [],
                    |r| r.get(0),
                )
                .map_err(state::sql(path))
            })
            .unwrap()
            .flatten();
        assert_eq!(source_msg.as_deref(), Some("m1"));
    }

    #[test]
    fn opportunistic_prune_deletes_only_confirmed_posted_expired_rows_and_survives_missing_state_db()
     {
        let (_dir, home) = test_home();
        let mut db = StoreDb::open(&store(&home)).unwrap();
        db.write::<_, StoreError>(|tx| {
            let path = db_path(tx);
            tx.execute(
                "INSERT INTO explicit_replies VALUES
                 ('reply:old-posted',0,'acme','slack','C1',NULL,NULL,'a','2026-11-01T10:00:00Z'),
                 ('reply:old-unposted',0,'acme','slack','C1',NULL,NULL,'b','2026-11-01T10:00:00Z'),
                 ('reply:fresh-posted',0,'acme','slack','C1',NULL,NULL,'c','2026-11-19T10:00:00Z')",
                [],
            )
            .map_err(sql_err(&path))?;
            Ok(())
        })
        .unwrap();
        let now: Timestamp = "2026-11-20T10:00:00Z".parse().unwrap();
        // No state/instrukt.db yet: prune skips cleanly without panicking or deleting rows.
        prune_posted_replies(&home, &mut db, now);
        let count_before: i64 = db
            .conn()
            .query_row("SELECT count(*) FROM explicit_replies", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count_before, 3);

        // Create state/instrukt.db and mark two rows posted.
        let state = RunnerState::new(&home);
        drop(StateDb::open(&home).unwrap());
        ingest_explicit_replies(&home, &state).unwrap();
        state
            .write::<_, StateError>(|tx| {
                tx.execute(
                    "UPDATE conversation_answers SET posted_at='2026-11-01T10:01:00Z' WHERE turn IN ('reply:old-posted','reply:fresh-posted')",
                    [],
                )
                .map_err(state::sql(&state::db_path(tx)))?;
                Ok(())
            })
            .unwrap();

        prune_posted_replies(&home, &mut db, now);
        let remaining: Vec<String> = db
            .conn()
            .prepare("SELECT reply_id FROM explicit_replies ORDER BY reply_id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(remaining, vec!["reply:fresh-posted", "reply:old-unposted"]);
    }
}
