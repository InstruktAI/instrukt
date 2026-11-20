//! The state database's migrations, oldest first. instrukt owns every one;
//! each moves the schema forward by one version and is never edited once
//! shipped. Tables and columns are the `state-database` spec's.

/// One migration: a script, or a step that looks at the database first.
pub(super) enum Migration {
    Sql(&'static str),
    Step(fn(&rusqlite::Transaction<'_>) -> rusqlite::Result<()>),
}
use Migration::{Sql, Step};

/// Every migration this build ships; the schema version is their count.
pub(super) const MIGRATIONS: &[Migration] = &[
    Sql(V1),
    Sql(V2),
    Sql(V3),
    Sql(V4),
    Sql(V5),
    Sql(V6),
    Sql(V7),
    Sql(V8),
    Sql(V9),
    Sql(V10),
    Sql(V11),
    Sql(V12),
    Sql(V13),
    Sql(V14),
    Sql(V15),
    Sql(V16),
    Sql(V17),
    Step(v18),
    Sql(V19),
    Step(v20),
    Step(v21),
    Sql(V22),
    Step(v23),
];

/// Version 22: a consumer's former source no longer in the configuration is
/// `abandoned`: its pending events stay unread in that Redis and its
/// `prepared` fact is kept, so configuring it again drains its own group.
/// The table is rebuilt to widen its `state` check; every row is kept.
/// Index only open dead-letter announcement intents for the journal cap.
const V22: &str = "
CREATE TABLE consumer_sources_v22 (
    consumer TEXT NOT NULL,
    source TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('active', 'retiring', 'retired', 'abandoned')),
    prepared INTEGER NOT NULL,
    PRIMARY KEY (consumer, source)
);
INSERT INTO consumer_sources_v22 (consumer, source, state, prepared)
    SELECT consumer, source, state, prepared FROM consumer_sources;
DROP TABLE consumer_sources;
ALTER TABLE consumer_sources_v22 RENAME TO consumer_sources;
CREATE INDEX IF NOT EXISTS consumer_open_announcements ON consumer_outcomes (
    json_extract(outcome, '$.parked.announced')
) WHERE json_extract(outcome, '$.parked.announcement_receipt') IS NOT NULL
      AND json_extract(outcome, '$.parked.announced') = 0;
";

/// Version 23: the platform's replies owed to a sender carry their reason
/// in the key: `not_own`, the one fixed reply to a sender the agent does not
/// answer, or `unavailable`, the one notice of the sender's current epoch,
/// whose decision is kept as `owed`, `posting` (with when the post was
/// sent) or `posted`. The table is rebuilt for its key; every stored reply
/// is kept as a `not_own` one, in its order. Widen the message-state check
/// so a failed turn remains outstanding without blocking relay work.
fn v23(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    let notice_rows_present: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('conversation_refusals') WHERE name='reason')",
        [],
        |row| row.get(0),
    )?;
    if !notice_rows_present {
        tx.execute_batch(V23_REFUSALS)?;
    }
    // An existing attempt supplies both bounds until a new attempt extends
    // the window. Keep the column if it is already present.
    let latest_present: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('conversation_answers') WHERE name='latest_attempted_at')",
        [],
        |row| row.get(0),
    )?;
    if !latest_present {
        tx.execute_batch("ALTER TABLE conversation_answers ADD COLUMN latest_attempted_at TEXT")?;
    }
    // Pre-metadata attempts retain their timestamp and acquire a resolution
    // only when the relay reads the provider or the attempt window ends.
    for (table, column, kind) in [
        ("conversation_answers", "post_id", "TEXT"),
        ("conversation_answers", "legacy_read_at", "TEXT"),
        ("conversation_answers", "delivery_unknown_at", "TEXT"),
        ("conversation_answers", "receipt_failure_deadline", "TEXT"),
        ("conversation_refusals", "post_id", "TEXT"),
        ("conversation_messages", "admission_position", "INTEGER"),
        ("conversation_answers", "admission_position", "INTEGER"),
        (
            "conversation_refusals",
            "covered_positions",
            "TEXT NOT NULL DEFAULT '[]'",
        ),
    ] {
        let present: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
            [table, column],
            |row| row.get(0),
        )?;
        if !present {
            tx.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {kind}"))?;
        }
    }
    tx.execute_batch(V23_MESSAGES)?;
    // Older records have no admission sequence. Their retained row order
    // supplies a durable baseline without comparing provider or wall clocks.
    tx.execute_batch(
        "UPDATE conversation_messages SET admission_position=rowid
           WHERE admission_position IS NULL AND (sender_name IS NOT NULL OR state IN ('pending','sent','failed','answered'));
         CREATE UNIQUE INDEX IF NOT EXISTS conversation_admission_order
           ON conversation_messages(admission_position);
         UPDATE conversation_answers AS a SET admission_position=(
           SELECT m.admission_position FROM conversation_messages m WHERE m.company=a.company
             AND m.connector=a.connector AND m.conversation=a.conversation AND m.turn=a.turn)
           WHERE a.admission_position IS NULL;",
    )?;
    // Before the final receipt and interval closure were atomic, a runner
    // could have stopped between those two writes.
    tx.execute_batch(
        "UPDATE conversation_messages AS m SET unavailable_since=NULL,turn_update=NULL
         WHERE m.state='answered' AND (m.unavailable_since IS NOT NULL OR m.turn_update IS NOT NULL)
           AND NOT EXISTS(SELECT 1 FROM conversation_answers a WHERE a.company=m.company
             AND a.connector=m.connector AND a.conversation=m.conversation AND a.turn=m.turn
             AND a.posted_at IS NULL AND a.delivery_unknown_at IS NULL)",
    )?;
    Ok(())
}

const V23_MESSAGES: &str = "
CREATE TABLE conversation_messages_v23 (
    company      TEXT NOT NULL,
    connector    TEXT NOT NULL,
    conversation TEXT NOT NULL,
    message      TEXT NOT NULL,
    sender       TEXT,
    sender_name  TEXT,
    sender_email TEXT,
    written_us   INTEGER NOT NULL,
    written_at   TEXT NOT NULL,
    arrived_at   TEXT NOT NULL,
    state        TEXT NOT NULL CHECK (state IN ('received', 'refused', 'pending', 'sent', 'failed', 'answered')),
    text         TEXT NOT NULL,
    context_json TEXT,
    boundary     TEXT,
    content      TEXT,
    attempted_at TEXT,
    turn         TEXT,
    failure      TEXT,
    admitted_at  TEXT,
    unavailable_since TEXT,
    turn_update  TEXT,
    turn_ended_at TEXT,
    admission_position INTEGER,
    PRIMARY KEY (company, connector, conversation, message)
);
INSERT INTO conversation_messages_v23 (rowid, company, connector, conversation, message,
    sender, sender_name, sender_email, written_us, written_at, arrived_at, state, text,
    context_json, boundary, content, attempted_at, turn, failure, admitted_at,
    unavailable_since, turn_update, turn_ended_at, admission_position)
SELECT rowid, company, connector, conversation, message,
    sender, sender_name, sender_email, written_us, written_at, arrived_at, state, text,
    context_json, boundary, content, attempted_at, turn, failure, admitted_at,
    unavailable_since, turn_update, turn_ended_at, admission_position
FROM conversation_messages;
DROP TABLE conversation_messages;
ALTER TABLE conversation_messages_v23 RENAME TO conversation_messages;
";

const V23_REFUSALS: &str = "
CREATE TABLE conversation_refusals_v23 (
    company      TEXT NOT NULL,
    connector    TEXT NOT NULL,
    conversation TEXT NOT NULL,
    sender       TEXT NOT NULL,
    reason       TEXT NOT NULL CHECK (reason IN ('not_own', 'unavailable')),
    received_at  TEXT NOT NULL,
    state        TEXT CHECK (CASE WHEN reason = 'not_own' THEN state IS NULL ELSE coalesce(state IN ('owed', 'posting', 'posted'), 0) END),
    attempted_at TEXT,
    replied_at   TEXT,
    failure      TEXT,
    CHECK (state != 'posting' OR attempted_at IS NOT NULL),
    PRIMARY KEY (company, connector, conversation, sender, reason)
);
INSERT INTO conversation_refusals_v23 (company, connector, conversation, sender, reason, received_at, replied_at, failure)
    SELECT company, connector, conversation, sender, 'not_own', received_at, replied_at, failure FROM conversation_refusals ORDER BY rowid;
DROP TABLE conversation_refusals;
ALTER TABLE conversation_refusals_v23 RENAME TO conversation_refusals;
";

/// Version 21: what a person is shown while their message is unfinished.
/// Each message keeps when its sender was admitted, when its no-work
/// interval opened (unset while none is open), the newest message the agent
/// has written in its turn and when its turn ended. A schema that already has
/// a column keeps it, and every stored row reads with the new columns unset.
fn v21(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    for (table, column) in [
        ("conversation_messages", "admitted_at"),
        ("conversation_messages", "unavailable_since"),
        ("conversation_messages", "turn_update"),
        ("conversation_messages", "turn_ended_at"),
    ] {
        let present: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info(?1) WHERE name=?2)",
            [table, column],
            |row| row.get(0),
        )?;
        if !present {
            tx.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} TEXT;"))?;
        }
    }
    Ok(())
}

/// Version 20: when the post of each answer piece was begun, recorded before
/// the connector can accept it, so a piece whose acceptance went unrecorded
/// is looked for at the connector before it is posted again. A schema that
/// already has the column keeps it.
fn v20(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    let present: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('conversation_answers') WHERE name='attempted_at')",
        [],
        |row| row.get(0),
    )?;
    if !present {
        tx.execute_batch("ALTER TABLE conversation_answers ADD COLUMN attempted_at TEXT;")?;
    }
    Ok(())
}

/// Version 19: the current state of the runner's two history workers,
/// extraction and the query responder. One row per worker, written by that
/// worker when its state changes; a worker without a failure has no row.
/// And the run loop's last skipped wake: its time and cause, one row at most,
/// removed by the next wake that is not skipped. And what the run loop did
/// with the configuration it last admitted: when, and whether it replaced
/// the generation in force or kept it.
const V19: &str = r#"
CREATE TABLE IF NOT EXISTS runner_workers (
    worker  TEXT PRIMARY KEY,
    failure TEXT NOT NULL,
    since   TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runner_skipped_wake (
    id    INTEGER PRIMARY KEY CHECK (id = 1),
    at    TEXT NOT NULL,
    cause TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS runner_admission (
    id      INTEGER PRIMARY KEY CHECK (id = 1),
    at      TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('replaced', 'kept'))
);
"#;

/// Version 18: each addressed message carries its sender's connector
/// identity, name and email address, and a message can be refused. A message
/// stored before keeps the member key it was recorded with as its sender's
/// name and has no identity and no address. The table is rebuilt for its new
/// columns and states only while it still has the earlier shape, so applying
/// this again changes nothing.
fn v18(tx: &rusqlite::Transaction<'_>) -> rusqlite::Result<()> {
    let earlier: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('conversation_messages') WHERE name='member')",
        [],
        |row| row.get(0),
    )?;
    if earlier {
        tx.execute_batch(V18)?;
    }
    Ok(())
}
const V18: &str = r#"
CREATE TABLE conversation_messages_v18 (
    company      TEXT NOT NULL,
    connector    TEXT NOT NULL,
    conversation TEXT NOT NULL,
    message      TEXT NOT NULL,
    sender       TEXT,
    sender_name  TEXT,
    sender_email TEXT,
    written_us   INTEGER NOT NULL,
    written_at   TEXT NOT NULL,
    arrived_at   TEXT NOT NULL,
    state        TEXT NOT NULL CHECK (state IN ('received', 'refused', 'pending', 'sent', 'answered')),
    text         TEXT NOT NULL,
    context_json TEXT,
    boundary     TEXT,
    content      TEXT,
    attempted_at TEXT,
    turn         TEXT,
    failure      TEXT,
    PRIMARY KEY (company, connector, conversation, message)
);
INSERT INTO conversation_messages_v18 (company, connector, conversation, message, sender, sender_name, sender_email, written_us, written_at, arrived_at, state, text, context_json, boundary, content, attempted_at, turn, failure)
SELECT company, connector, conversation, message, NULL, member, NULL, written_us, written_at, arrived_at, state, text, context_json, boundary, content, attempted_at, turn, failure FROM conversation_messages;
DROP TABLE conversation_messages;
ALTER TABLE conversation_messages_v18 RENAME TO conversation_messages;
"#;

/// Version 7: retained conversations, search and transactional extraction positions.
const V7: &str = r#"
CREATE TABLE history_sources (
 source TEXT PRIMARY KEY, position INTEGER NOT NULL DEFAULT 0, imported INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE history_pending (
 source TEXT NOT NULL, native_id TEXT NOT NULL, PRIMARY KEY(source,native_id)
);
CREATE TABLE history_conversations (
 id INTEGER PRIMARY KEY, source TEXT NOT NULL, native_id TEXT NOT NULL, computer TEXT NOT NULL,
 title TEXT NOT NULL, parent TEXT, aliases TEXT NOT NULL, document TEXT NOT NULL,
 body TEXT NOT NULL, limitations TEXT NOT NULL, UNIQUE(source,native_id)
);
CREATE VIRTUAL TABLE history_fts USING fts5(title,body,content='history_conversations',content_rowid='id');
CREATE TRIGGER history_insert AFTER INSERT ON history_conversations BEGIN
 INSERT INTO history_fts(rowid,title,body) VALUES(new.id,new.title,new.body);
END;
CREATE TRIGGER history_delete AFTER DELETE ON history_conversations BEGIN
 INSERT INTO history_fts(history_fts,rowid,title,body) VALUES('delete',old.id,old.title,old.body);
END;
CREATE TRIGGER history_update AFTER UPDATE ON history_conversations BEGIN
 INSERT INTO history_fts(history_fts,rowid,title,body) VALUES('delete',old.id,old.title,old.body);
 INSERT INTO history_fts(rowid,title,body) VALUES(new.id,new.title,new.body);
END;
"#;

/// Version 1: the work tables and the runner tables.
const V1: &str = r#"
-- Work tables, owned and written by work.

CREATE TABLE todos (
    id               TEXT PRIMARY KEY NOT NULL,
    project          TEXT NOT NULL,
    slug             TEXT NOT NULL,
    kind             TEXT NOT NULL,
    status           TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    created_by       TEXT NOT NULL,
    human_involved   INTEGER,
    revision         INTEGER NOT NULL,
    group_id         TEXT,
    group_slug       TEXT,
    schedule         TEXT,
    catch_up         TEXT NOT NULL,
    agent            TEXT,
    thinking         TEXT,
    claim_machine    TEXT,
    claim_session    TEXT,
    claim_at         TEXT,
    branch           TEXT,
    delivered_commit TEXT,
    delivered_at     TEXT,
    delivered_by     TEXT,
    removed_at       TEXT,
    removed_from     TEXT,
    removal          TEXT,
    sequence         INTEGER NOT NULL
);
CREATE UNIQUE INDEX todos_live_slug ON todos (project, slug)
    WHERE status IN ('active', 'iceboxed', 'structural_holder');
CREATE INDEX todos_project ON todos (project);

CREATE TABLE prerequisites (
    todo_id         TEXT NOT NULL REFERENCES todos (id) ON DELETE CASCADE,
    prerequisite_id TEXT NOT NULL,
    slug            TEXT NOT NULL,
    PRIMARY KEY (todo_id, prerequisite_id)
);

CREATE TABLE artifacts (
    todo_id TEXT NOT NULL REFERENCES todos (id) ON DELETE CASCADE,
    name    TEXT NOT NULL,
    address INTEGER,
    PRIMARY KEY (todo_id, name)
);

CREATE TABLE artifact_versions (
    todo_id      TEXT NOT NULL,
    name         TEXT NOT NULL,
    sequence     INTEGER NOT NULL,
    digest       TEXT NOT NULL,
    writer       TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    content      TEXT,
    derived_from TEXT NOT NULL,
    grounding    TEXT NOT NULL,
    remote       INTEGER,
    PRIMARY KEY (todo_id, sequence),
    FOREIGN KEY (todo_id, name) REFERENCES artifacts (todo_id, name) ON DELETE CASCADE
);

CREATE TABLE stamps (
    todo_id          TEXT NOT NULL REFERENCES todos (id) ON DELETE CASCADE,
    sequence         INTEGER NOT NULL,
    fact             TEXT NOT NULL,
    at               TEXT NOT NULL,
    actor            TEXT NOT NULL,
    role             TEXT,
    model            TEXT,
    model_version    TEXT,
    model_provenance TEXT NOT NULL,
    eval_fingerprint TEXT,
    human_involved   INTEGER,
    evidence         TEXT NOT NULL,
    PRIMARY KEY (todo_id, sequence)
);

CREATE TABLE holds (
    todo_id     TEXT NOT NULL REFERENCES todos (id) ON DELETE CASCADE,
    source      TEXT NOT NULL,
    reason      TEXT NOT NULL,
    set_by      TEXT NOT NULL,
    set_at      TEXT NOT NULL,
    released_at TEXT,
    released_by TEXT,
    PRIMARY KEY (todo_id, set_at)
);
CREATE UNIQUE INDEX holds_current ON holds (todo_id) WHERE released_at IS NULL;

CREATE TABLE session_bindings (
    todo_id  TEXT NOT NULL REFERENCES todos (id) ON DELETE CASCADE,
    role     TEXT NOT NULL,
    session  TEXT NOT NULL,
    worktree TEXT,
    PRIMARY KEY (todo_id, role)
);

CREATE TABLE reflections (
    todo_id           TEXT PRIMARY KEY NOT NULL REFERENCES todos (id) ON DELETE CASCADE,
    issue_number      INTEGER,
    remote_revision   INTEGER,
    pending           INTEGER NOT NULL,
    conflicted        INTEGER NOT NULL,
    failure           TEXT,
    pull_number       INTEGER,
    issue_id          INTEGER,
    record_comment    INTEGER,
    etag              TEXT,
    pull_outstanding  TEXT,
    reviews_reflected INTEGER NOT NULL,
    pull_closed       INTEGER NOT NULL,
    board             TEXT,
    board_item        TEXT,
    board_status      TEXT,
    board_failure     TEXT
);

CREATE TABLE inbound_commands (
    todo_id    TEXT NOT NULL REFERENCES todos (id) ON DELETE CASCADE,
    comment_id INTEGER NOT NULL,
    PRIMARY KEY (todo_id, comment_id)
);

CREATE TABLE provider_deferrals (
    project TEXT PRIMARY KEY NOT NULL,
    until   TEXT NOT NULL
);

CREATE TABLE listing_copies (
    project         TEXT NOT NULL,
    issue_number    INTEGER NOT NULL,
    slug            TEXT NOT NULL,
    kind            TEXT NOT NULL,
    status          TEXT NOT NULL,
    remote_revision INTEGER NOT NULL,
    claim_machine   TEXT,
    record          TEXT NOT NULL,
    copied_at       TEXT NOT NULL,
    PRIMARY KEY (project, issue_number)
);

-- Runner tables, owned and written by the runner.

CREATE TABLE events (
    seq             INTEGER PRIMARY KEY NOT NULL,
    id              TEXT NOT NULL UNIQUE,
    name            TEXT NOT NULL,
    project         TEXT,
    payload         TEXT NOT NULL,
    envelope        TEXT NOT NULL,
    idempotency_key TEXT UNIQUE,
    logged_at       TEXT NOT NULL,
    publication_scope TEXT CHECK (publication_scope IN ('local', 'cluster')),
    publication_state TEXT CHECK (publication_state IN ('pending', 'confirmed')),
    publication_failure TEXT
);

CREATE INDEX events_pending_publications ON events (publication_scope, seq)
WHERE publication_state = 'pending';

CREATE TABLE redis_reachability (
    source TEXT PRIMARY KEY NOT NULL,
    reachable INTEGER NOT NULL
);

CREATE TABLE event_coordinates (
    source   TEXT NOT NULL,
    stream   TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    seq      INTEGER NOT NULL REFERENCES events (seq),
    PRIMARY KEY (source, stream, entry_id)
);

CREATE TABLE consumer_cursors (
    consumer TEXT NOT NULL,
    source   TEXT NOT NULL,
    stream   TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    seq      INTEGER REFERENCES events (seq),
    PRIMARY KEY (consumer, source, stream)
);

CREATE TABLE handled_events (
    consumer   TEXT NOT NULL,
    event_id   TEXT NOT NULL,
    seq        INTEGER REFERENCES events (seq),
    handled_at TEXT NOT NULL,
    PRIMARY KEY (consumer, event_id)
);

CREATE TABLE consumer_outcomes (
    consumer TEXT NOT NULL,
    source   TEXT NOT NULL,
    stream   TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    outcome  TEXT NOT NULL,
    settled_at TEXT NOT NULL,
    PRIMARY KEY (consumer, source, stream, entry_id)
);

-- Reliability journal; the dead letter itself belongs to the source Redis.
CREATE TABLE event_attempts (
    consumer TEXT NOT NULL,
    source   TEXT NOT NULL,
    stream   TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    attempts INTEGER NOT NULL,
    first_attempt_at TEXT NOT NULL,
    last_attempt_at TEXT NOT NULL,
    failure TEXT NOT NULL,
    PRIMARY KEY (consumer, source, stream, entry_id)
);

CREATE TABLE consumer_sources (
    consumer TEXT NOT NULL,
    source TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('active', 'retiring', 'retired')),
    prepared INTEGER NOT NULL,
    PRIMARY KEY (consumer, source)
);

CREATE TABLE event_parking (
    consumer TEXT NOT NULL,
    source TEXT NOT NULL,
    stream TEXT NOT NULL,
    entry_id TEXT NOT NULL,
    seq INTEGER REFERENCES events (seq),
    outcome TEXT NOT NULL,
    PRIMARY KEY (consumer, source, stream, entry_id)
);

CREATE TABLE procedure_activations (
    event_id TEXT NOT NULL REFERENCES events (id),
    procedure TEXT NOT NULL,
    exact_home TEXT NOT NULL,
    envelope TEXT NOT NULL,
    declaration TEXT NOT NULL,
    accepted_at TEXT NOT NULL,
    state TEXT NOT NULL,
    failure TEXT,
    PRIMARY KEY (event_id, procedure, exact_home)
);

CREATE TABLE request_validators (
    project TEXT NOT NULL,
    request TEXT NOT NULL,
    etag    TEXT NOT NULL,
    body    TEXT NOT NULL,
    PRIMARY KEY (project, request)
);

CREATE TABLE poll_deferrals (
    project TEXT PRIMARY KEY NOT NULL,
    until   TEXT NOT NULL
);

CREATE TABLE watched_refs (
    project     TEXT NOT NULL,
    remote      TEXT NOT NULL,
    ref         TEXT NOT NULL,
    revision    TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    PRIMARY KEY (project, remote, ref)
);

CREATE TABLE executions (
    project        TEXT NOT NULL,
    todo_id        TEXT NOT NULL,
    role           TEXT NOT NULL,
    session        TEXT NOT NULL,
    workflow_state TEXT NOT NULL,
    recovery_sent  INTEGER NOT NULL,
    anomaly_raised INTEGER NOT NULL,
    PRIMARY KEY (project, todo_id, role)
);

CREATE TABLE standing_bindings (
    procedure TEXT NOT NULL,
    project   TEXT NOT NULL,
    session   TEXT NOT NULL,
    PRIMARY KEY (procedure, project)
);

CREATE TABLE standing_starts (
    procedure TEXT NOT NULL,
    project   TEXT NOT NULL,
    begun_at  TEXT NOT NULL,
    reported  INTEGER NOT NULL,
    PRIMARY KEY (procedure, project)
);

CREATE TABLE task_failures (
    project    TEXT NOT NULL,
    todo_id    TEXT NOT NULL,
    occurrence TEXT NOT NULL,
    session    TEXT,
    notified   INTEGER NOT NULL,
    PRIMARY KEY (project, todo_id)
);

CREATE TABLE wakeups (
    last_at TEXT NOT NULL
);
"#;

/// Version 2: all-start identity, durable recovery and per-key ownership.
const V2: &str = r#"
ALTER TABLE session_bindings RENAME TO session_bindings_v1;
CREATE TABLE session_bindings (
 todo_id TEXT NOT NULL REFERENCES todos(id) ON DELETE CASCADE,
 role TEXT NOT NULL, attempt INTEGER NOT NULL, runtime TEXT, semantic_key TEXT NOT NULL, display_title TEXT,
 session TEXT, worktree TEXT, branch TEXT, base_head TEXT,
 completion_sequence INTEGER, completed_head TEXT,
 failure_reason TEXT, failure_head TEXT, failure_workspace TEXT, failed_at TEXT,
 released_at TEXT,
 evidence TEXT NOT NULL,
 PRIMARY KEY(todo_id,role,attempt), UNIQUE(runtime,semantic_key)
);
INSERT INTO session_bindings (todo_id,role,attempt,runtime,semantic_key,session,worktree,evidence)
SELECT todo_id,role,1,substr(session,1,instr(session,':')-1),
       'legacy:' || todo_id || ':' || role,session,worktree,
       json_object('role',role,'session',session,'worktree',worktree,'assignment','unspecified',
                   'attempt',1,'semantic_key','legacy:' || todo_id || ':' || role)
FROM session_bindings_v1;
DROP TABLE session_bindings_v1;
ALTER TABLE todos ADD COLUMN refreshed_at TEXT;
CREATE TABLE runtime_key_locks (
 runtime TEXT NOT NULL, semantic_key TEXT NOT NULL, project TEXT,
 owner TEXT, acquired_at TEXT NOT NULL, PRIMARY KEY(runtime,semantic_key)
);
CREATE TABLE runtime_starts (
 runtime TEXT NOT NULL, semantic_key TEXT NOT NULL, project TEXT, kind TEXT NOT NULL,
 subject TEXT NOT NULL, checkout TEXT NOT NULL, brief TEXT NOT NULL, request TEXT NOT NULL,
 begun_at TEXT NOT NULL, reported INTEGER NOT NULL, resolved_at TEXT, resolved_by TEXT,
 resolution TEXT, state TEXT NOT NULL, PRIMARY KEY(runtime,semantic_key)
);
CREATE TABLE runtime_bindings (
 runtime TEXT NOT NULL, semantic_key TEXT NOT NULL, kind TEXT NOT NULL, project TEXT,
 subject TEXT NOT NULL, session TEXT NOT NULL, display_title TEXT, project_id TEXT, checkout TEXT NOT NULL,
 worktree TEXT, branch TEXT, base_head TEXT, accepted INTEGER NOT NULL, request TEXT,
 PRIMARY KEY(runtime,semantic_key)
);
"#;

/// Version 3: durable session-wide input and standing/parent attention.
const V3: &str = r#"
ALTER TABLE stamps ADD COLUMN source_comment INTEGER;
CREATE UNIQUE INDEX stamps_decision_source ON stamps(todo_id,source_comment) WHERE source_comment IS NOT NULL;
ALTER TABLE inbound_commands ADD COLUMN reply TEXT;
ALTER TABLE inbound_commands ADD COLUMN reply_pending INTEGER NOT NULL DEFAULT 0;
ALTER TABLE procedure_activations ADD COLUMN delivery_kind TEXT NOT NULL DEFAULT 'occurrence';
ALTER TABLE procedure_activations ADD COLUMN subject TEXT;
ALTER TABLE procedure_activations ADD COLUMN event_type TEXT;
DROP INDEX holds_current;
ALTER TABLE holds RENAME TO holds_old;
CREATE TABLE holds (todo_id TEXT NOT NULL REFERENCES todos(id) ON DELETE CASCADE, source TEXT NOT NULL, reason TEXT NOT NULL, set_by TEXT NOT NULL, set_at TEXT NOT NULL, released_at TEXT, released_by TEXT);
INSERT INTO holds SELECT * FROM holds_old ORDER BY rowid;
DROP TABLE holds_old;
CREATE UNIQUE INDEX holds_current ON holds(todo_id) WHERE released_at IS NULL;
CREATE TABLE runtime_inputs (
 runtime TEXT NOT NULL, session TEXT NOT NULL, input_id TEXT NOT NULL,
 intent TEXT NOT NULL, content TEXT NOT NULL, state TEXT NOT NULL,
 kind TEXT NOT NULL, source_at TEXT, arrived_at TEXT NOT NULL, assignment TEXT,
 group_id TEXT, selected_content TEXT, waiting INTEGER NOT NULL DEFAULT 0, retry_blocked INTEGER NOT NULL DEFAULT 0,
 evidence_source TEXT, evidence_at TEXT, accepted_at TEXT, entered_run TEXT, replayed_run TEXT, watch_run INTEGER NOT NULL DEFAULT 1, failure TEXT,
 PRIMARY KEY(runtime,session,input_id)
);
CREATE UNIQUE INDEX runtime_input_waiting ON runtime_inputs(runtime,session) WHERE waiting=1;
CREATE TABLE procedure_responsibilities (
 responsibility TEXT PRIMARY KEY NOT NULL, procedure TEXT NOT NULL, exact_home TEXT NOT NULL,
 subject TEXT, generation INTEGER NOT NULL, semantic_key TEXT NOT NULL UNIQUE,
 checkout TEXT NOT NULL, declaration TEXT NOT NULL, request TEXT NOT NULL,
 session TEXT, terminal TEXT, seen_position INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE procedure_generations (responsibility TEXT NOT NULL,generation INTEGER NOT NULL,semantic_key TEXT NOT NULL,session TEXT NOT NULL,declaration TEXT NOT NULL,request TEXT NOT NULL,terminal TEXT NOT NULL,PRIMARY KEY(responsibility,generation));
CREATE TABLE standing_deliveries (
 event_id TEXT NOT NULL, procedure TEXT NOT NULL, exact_home TEXT NOT NULL,
 responsibility TEXT NOT NULL, position INTEGER PRIMARY KEY AUTOINCREMENT,
 envelope TEXT NOT NULL, declaration TEXT NOT NULL, state TEXT NOT NULL,
 target TEXT, input_id TEXT, failure TEXT,
 UNIQUE(event_id,procedure,exact_home)
);
CREATE TABLE manual_standing_reservations (
 responsibility TEXT PRIMARY KEY, token TEXT NOT NULL UNIQUE, caller TEXT NOT NULL,
 home TEXT NOT NULL, declaration TEXT NOT NULL, request TEXT NOT NULL, session TEXT,
 retry_allowed INTEGER NOT NULL DEFAULT 0, no_start_reason TEXT
);
CREATE TABLE approval_attention (
 todo_id TEXT NOT NULL, decision_sequence INTEGER NOT NULL, content TEXT NOT NULL,
 target TEXT, input_id TEXT, state TEXT NOT NULL, failure TEXT,
 PRIMARY KEY(todo_id,decision_sequence)
);
CREATE TABLE github_label_actions (
 project TEXT NOT NULL, issue_number INTEGER NOT NULL, action_id INTEGER NOT NULL,
 todo_id TEXT NOT NULL, action TEXT NOT NULL, login TEXT NOT NULL,
 at TEXT NOT NULL, outcome TEXT NOT NULL, PRIMARY KEY(project,issue_number,action_id)
);
"#;

/// Version 4: dispatch tiers and scoped, expiring availability.
const V4: &str = r#"
ALTER TABLE todos RENAME COLUMN thinking TO tier;
UPDATE todos SET tier=CASE tier WHEN 'fast' THEN 'light' WHEN 'med' THEN 'standard' WHEN 'slow' THEN 'strong' ELSE tier END;
CREATE TABLE todo_aliases (todo_id TEXT NOT NULL REFERENCES todos(id) ON DELETE CASCADE, slug TEXT NOT NULL, PRIMARY KEY(todo_id,slug));
CREATE TABLE agent_availability (
 agent TEXT NOT NULL, model TEXT NOT NULL DEFAULT '', status TEXT NOT NULL,
 status_until TEXT, reason TEXT, PRIMARY KEY(agent,model)
);
"#;

/// Version 5: profile-based selection evidence preserves interrupted starts.
const V5: &str = r#"
UPDATE runtime_starts SET request=json_remove(json_set(request,
 '$.selection.profile',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.kind',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.instance',json_extract(request,'$.selection.provider')),'$.selection.family','$.selection.provider')
 WHERE json_type(request,'$.selection.family') IS NOT NULL;
UPDATE runtime_starts SET request=json_set(request,'$.preferences',
 json((SELECT json_group_array(json(CASE WHEN json_extract(value,'$.agent')='gemini' THEN json_set(value,'$.agent','antigravity') ELSE value END)) FROM json_each(request,'$.preferences'))))
 WHERE json_type(request,'$.preferences')='array';
UPDATE runtime_bindings SET request=json_remove(json_set(request,
 '$.selection.profile',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.kind',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.instance',json_extract(request,'$.selection.provider')),'$.selection.family','$.selection.provider')
 WHERE json_type(request,'$.selection.family') IS NOT NULL;
UPDATE runtime_bindings SET request=json_set(request,'$.preferences',
 json((SELECT json_group_array(json(CASE WHEN json_extract(value,'$.agent')='gemini' THEN json_set(value,'$.agent','antigravity') ELSE value END)) FROM json_each(request,'$.preferences'))))
 WHERE json_type(request,'$.preferences')='array';
UPDATE procedure_responsibilities SET request=json_remove(json_set(request,
 '$.selection.profile',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.kind',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.instance',json_extract(request,'$.selection.provider')),'$.selection.family','$.selection.provider')
 WHERE json_type(request,'$.selection.family') IS NOT NULL;
UPDATE procedure_responsibilities SET request=json_set(request,'$.preferences',
 json((SELECT json_group_array(json(CASE WHEN json_extract(value,'$.agent')='gemini' THEN json_set(value,'$.agent','antigravity') ELSE value END)) FROM json_each(request,'$.preferences'))))
 WHERE json_type(request,'$.preferences')='array';
UPDATE procedure_generations SET request=json_remove(json_set(request,
 '$.selection.profile',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.kind',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.instance',json_extract(request,'$.selection.provider')),'$.selection.family','$.selection.provider')
 WHERE json_type(request,'$.selection.family') IS NOT NULL;
UPDATE procedure_generations SET request=json_set(request,'$.preferences',
 json((SELECT json_group_array(json(CASE WHEN json_extract(value,'$.agent')='gemini' THEN json_set(value,'$.agent','antigravity') ELSE value END)) FROM json_each(request,'$.preferences'))))
 WHERE json_type(request,'$.preferences')='array';
UPDATE manual_standing_reservations SET request=json_remove(json_set(request,
 '$.selection.profile',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.kind',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.instance',json_extract(request,'$.selection.provider')),'$.selection.family','$.selection.provider')
 WHERE json_type(request,'$.selection.family') IS NOT NULL;
UPDATE manual_standing_reservations SET request=json_set(request,'$.preferences',
 json((SELECT json_group_array(json(CASE WHEN json_extract(value,'$.agent')='gemini' THEN json_set(value,'$.agent','antigravity') ELSE value END)) FROM json_each(request,'$.preferences'))))
 WHERE json_type(request,'$.preferences')='array';
UPDATE procedure_responsibilities SET declaration=json_remove(declaration,'$.dispatch') WHERE json_type(declaration,'$.dispatch') IS NOT NULL;
UPDATE procedure_generations SET declaration=json_remove(declaration,'$.dispatch') WHERE json_type(declaration,'$.dispatch') IS NOT NULL;
UPDATE standing_deliveries SET declaration=json_remove(declaration,'$.dispatch') WHERE json_type(declaration,'$.dispatch') IS NOT NULL;
UPDATE manual_standing_reservations SET declaration=json_remove(declaration,'$.dispatch') WHERE json_type(declaration,'$.dispatch') IS NOT NULL;
UPDATE todos SET agent='antigravity' WHERE agent='gemini';
UPDATE agent_availability SET agent='antigravity' WHERE agent='gemini';
"#;

/// Version 6: observed quotas coexist with independent manual restrictions.
const V6: &str = r#"
CREATE TABLE agent_quota (
 agent TEXT NOT NULL, model TEXT NOT NULL, status TEXT NOT NULL,
 status_until TEXT NOT NULL, reason TEXT NOT NULL, PRIMARY KEY(agent,model)
);
"#;

/// Version 8: normalized source records and runtime-opaque positions.
const V8: &str = r#"
DROP TABLE history_pending;
CREATE TABLE history_projection_records (
 source TEXT NOT NULL, projection TEXT NOT NULL, native_id TEXT NOT NULL,
 thread_id TEXT NOT NULL, updated_at TEXT NOT NULL, payload_json TEXT NOT NULL,
 PRIMARY KEY(source,projection,native_id)
);
CREATE INDEX history_projection_threads ON history_projection_records(source,thread_id);
CREATE TABLE history_source_positions (source TEXT PRIMARY KEY,position_json TEXT NOT NULL);
INSERT INTO history_projection_records
SELECT source,'conversation',native_id,native_id,coalesce(json_extract(document,'$.thread.updatedAt'),''),
 json_object('kind','conversation','id',native_id,'conversation_id',native_id,'parent_id',json_extract(document,'$.thread.lineage.parentThreadId'),
 'title',json_extract(document,'$.thread.title'),'text',NULL,'run_id',NULL,'role',NULL,
 'created_at',json_extract(document,'$.thread.createdAt'),'updated_at',json_extract(document,'$.thread.updatedAt'),
 'completed',json('false'),'native',json_extract(document,'$.thread'))
FROM history_conversations WHERE source LIKE 't3:%' AND json_type(document,'$.thread')='object';
INSERT OR IGNORE INTO history_projection_records
SELECT c.source,'message',json_extract(v.value,'$.id'),coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.completedAt'),json_extract(v.value,'$.requestedAt'),''),
 json_object('kind','message','id',json_extract(v.value,'$.id'),'conversation_id',coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 'parent_id',NULL,'title',NULL,'run_id',json_extract(v.value,'$.runId'),'role',
 CASE WHEN json_extract(v.value,'$.role') IN ('user','assistant','system') THEN json_extract(v.value,'$.role') ELSE NULL END,
 'created_at',coalesce(json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.requestedAt')),
 'updated_at',coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt')),
 'completed',CASE WHEN json_extract(v.value,'$.streaming')=1 THEN json('false') ELSE json('true') END,
 'text',coalesce(json_extract(v.value,'$.text'),json_extract(v.value,'$.markdown'),json_extract(v.value,'$.input'),'') , 'native',json(v.value))
FROM history_conversations c,json_each(c.document,'$.messages') v
WHERE c.source LIKE 't3:%' AND json_type(v.value,'$.id')='text';
INSERT OR IGNORE INTO history_projection_records
SELECT c.source,'run',json_extract(v.value,'$.id'),coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.completedAt'),json_extract(v.value,'$.requestedAt'),''),
 json_object('kind','run','id',json_extract(v.value,'$.id'),'conversation_id',coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 'parent_id',NULL,'title',NULL,'run_id',json_extract(v.value,'$.runId'),'role',
 CASE WHEN json_extract(v.value,'$.role') IN ('user','assistant','system') THEN json_extract(v.value,'$.role') ELSE NULL END,
 'created_at',coalesce(json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.requestedAt')),
 'updated_at',coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt')),
 'completed',CASE WHEN json_extract(v.value,'$.completedAt') IS NOT NULL THEN json('true') ELSE json('false') END,
 'text',coalesce(json_extract(v.value,'$.text'),json_extract(v.value,'$.markdown'),json_extract(v.value,'$.input'),'') , 'native',json(v.value))
FROM history_conversations c,json_each(c.document,'$.runs') v
WHERE c.source LIKE 't3:%' AND json_type(v.value,'$.id')='text';
INSERT OR IGNORE INTO history_projection_records
SELECT c.source,'subagent',json_extract(v.value,'$.id'),coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.completedAt'),json_extract(v.value,'$.requestedAt'),''),
 json_object('kind','subagent','id',json_extract(v.value,'$.id'),'conversation_id',coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 'parent_id',NULL,'title',NULL,'run_id',json_extract(v.value,'$.runId'),'role',
 CASE WHEN json_extract(v.value,'$.role') IN ('user','assistant','system') THEN json_extract(v.value,'$.role') ELSE NULL END,
 'created_at',coalesce(json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.requestedAt')),
 'updated_at',coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt')),
 'completed',CASE WHEN json_extract(v.value,'$.completedAt') IS NOT NULL THEN json('true') ELSE json('false') END,
 'text',coalesce(json_extract(v.value,'$.text'),json_extract(v.value,'$.markdown'),json_extract(v.value,'$.input'),'') , 'native',json(v.value))
FROM history_conversations c,json_each(c.document,'$.subagents') v
WHERE c.source LIKE 't3:%' AND json_type(v.value,'$.id')='text';
INSERT OR IGNORE INTO history_projection_records
SELECT c.source,'turn_item',json_extract(v.value,'$.id'),coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.completedAt'),json_extract(v.value,'$.requestedAt'),''),
 json_object('kind','turn_item','id',json_extract(v.value,'$.id'),'conversation_id',coalesce(json_extract(v.value,'$.threadId'),c.native_id),
 'parent_id',NULL,'title',NULL,'run_id',json_extract(v.value,'$.runId'),'role',
 CASE WHEN json_extract(v.value,'$.role') IN ('user','assistant','system') THEN json_extract(v.value,'$.role') ELSE NULL END,
 'created_at',coalesce(json_extract(v.value,'$.createdAt'),json_extract(v.value,'$.requestedAt')),
 'updated_at',coalesce(json_extract(v.value,'$.updatedAt'),json_extract(v.value,'$.createdAt')),
 'completed',CASE WHEN json_extract(v.value,'$.completedAt') IS NOT NULL THEN json('true') ELSE json('false') END,
 'text',coalesce(json_extract(v.value,'$.text'),json_extract(v.value,'$.markdown'),json_extract(v.value,'$.input'),'') , 'native',json(v.value))
FROM history_conversations c,json_each(c.document,'$.turnItems') v
WHERE c.source LIKE 't3:%' AND json_type(v.value,'$.id')='text';
"#;

/// Version 9: repair the complete retained text contribution of prior normalized records.
const V9: &str = r#"
UPDATE history_projection_records
SET payload_json=json_set(payload_json,'$.text',
 coalesce(json_extract(payload_json,'$.text'),'') || char(10) || char(10) ||
 coalesce((SELECT group_concat(value,char(10)||char(10)) FROM (
  SELECT value FROM json_each(json_extract(history_projection_records.payload_json,'$.native'))
  WHERE type='text' AND key IN ('text','input','output','markdown','diffStr','result','progress')
  ORDER BY CASE key WHEN 'text' THEN 0 WHEN 'input' THEN 1 WHEN 'output' THEN 2 WHEN 'markdown' THEN 3 WHEN 'diffStr' THEN 4 WHEN 'result' THEN 5 ELSE 6 END
 )),''))
WHERE source LIKE 't3:%';
UPDATE history_conversations AS c
SET body=body || char(10) || char(10) || coalesce((
 SELECT group_concat(json_extract(r.payload_json,'$.text'),char(10)||char(10))
 FROM history_projection_records r
 WHERE r.source=c.source AND r.thread_id=c.native_id AND r.projection<>'conversation'
),'')
WHERE c.source LIKE 't3:%' AND EXISTS(
 SELECT 1 FROM history_projection_records r WHERE r.source=c.source AND r.thread_id=c.native_id
);

"#;

/// Version 10: retained start requests keep neutral effort separately from extra settings.
const V10: &str = r#"
UPDATE runtime_starts SET request=json_remove(request,'$.selection.options.reasoningEffort') WHERE json_extract(request,'$.selection.kind') IN ('codex','grok','kiro','muse');
UPDATE runtime_starts SET request=json_remove(request,'$.selection.options.thinking_level') WHERE json_extract(request,'$.selection.kind') IN ('antigravity');
UPDATE runtime_starts SET request=json_remove(request,'$.selection.options.thinking') WHERE json_extract(request,'$.selection.kind') IN ('pi');
UPDATE runtime_starts SET request=json_remove(request,'$.selection.options.variant') WHERE json_extract(request,'$.selection.kind') IN ('opencode');
UPDATE runtime_starts SET request=json_remove(request,'$.selection.options.effort') WHERE json_extract(request,'$.selection.kind') IN ('claude','fx','cursor');
UPDATE runtime_starts SET request=json_remove(request,'$.selection.options.reasoning') WHERE json_extract(request,'$.selection.kind')='cursor';
UPDATE runtime_bindings SET request=json_remove(request,'$.selection.options.reasoningEffort') WHERE json_extract(request,'$.selection.kind') IN ('codex','grok','kiro','muse');
UPDATE runtime_bindings SET request=json_remove(request,'$.selection.options.thinking_level') WHERE json_extract(request,'$.selection.kind') IN ('antigravity');
UPDATE runtime_bindings SET request=json_remove(request,'$.selection.options.thinking') WHERE json_extract(request,'$.selection.kind') IN ('pi');
UPDATE runtime_bindings SET request=json_remove(request,'$.selection.options.variant') WHERE json_extract(request,'$.selection.kind') IN ('opencode');
UPDATE runtime_bindings SET request=json_remove(request,'$.selection.options.effort') WHERE json_extract(request,'$.selection.kind') IN ('claude','fx','cursor');
UPDATE runtime_bindings SET request=json_remove(request,'$.selection.options.reasoning') WHERE json_extract(request,'$.selection.kind')='cursor';
UPDATE procedure_responsibilities SET request=json_remove(request,'$.selection.options.reasoningEffort') WHERE json_extract(request,'$.selection.kind') IN ('codex','grok','kiro','muse');
UPDATE procedure_responsibilities SET request=json_remove(request,'$.selection.options.thinking_level') WHERE json_extract(request,'$.selection.kind') IN ('antigravity');
UPDATE procedure_responsibilities SET request=json_remove(request,'$.selection.options.thinking') WHERE json_extract(request,'$.selection.kind') IN ('pi');
UPDATE procedure_responsibilities SET request=json_remove(request,'$.selection.options.variant') WHERE json_extract(request,'$.selection.kind') IN ('opencode');
UPDATE procedure_responsibilities SET request=json_remove(request,'$.selection.options.effort') WHERE json_extract(request,'$.selection.kind') IN ('claude','fx','cursor');
UPDATE procedure_responsibilities SET request=json_remove(request,'$.selection.options.reasoning') WHERE json_extract(request,'$.selection.kind')='cursor';
UPDATE procedure_generations SET request=json_remove(request,'$.selection.options.reasoningEffort') WHERE json_extract(request,'$.selection.kind') IN ('codex','grok','kiro','muse');
UPDATE procedure_generations SET request=json_remove(request,'$.selection.options.thinking_level') WHERE json_extract(request,'$.selection.kind') IN ('antigravity');
UPDATE procedure_generations SET request=json_remove(request,'$.selection.options.thinking') WHERE json_extract(request,'$.selection.kind') IN ('pi');
UPDATE procedure_generations SET request=json_remove(request,'$.selection.options.variant') WHERE json_extract(request,'$.selection.kind') IN ('opencode');
UPDATE procedure_generations SET request=json_remove(request,'$.selection.options.effort') WHERE json_extract(request,'$.selection.kind') IN ('claude','fx','cursor');
UPDATE procedure_generations SET request=json_remove(request,'$.selection.options.reasoning') WHERE json_extract(request,'$.selection.kind')='cursor';
UPDATE manual_standing_reservations SET request=json_remove(request,'$.selection.options.reasoningEffort') WHERE json_extract(request,'$.selection.kind') IN ('codex','grok','kiro','muse');
UPDATE manual_standing_reservations SET request=json_remove(request,'$.selection.options.thinking_level') WHERE json_extract(request,'$.selection.kind') IN ('antigravity');
UPDATE manual_standing_reservations SET request=json_remove(request,'$.selection.options.thinking') WHERE json_extract(request,'$.selection.kind') IN ('pi');
UPDATE manual_standing_reservations SET request=json_remove(request,'$.selection.options.variant') WHERE json_extract(request,'$.selection.kind') IN ('opencode');
UPDATE manual_standing_reservations SET request=json_remove(request,'$.selection.options.effort') WHERE json_extract(request,'$.selection.kind') IN ('claude','fx','cursor');
UPDATE manual_standing_reservations SET request=json_remove(request,'$.selection.options.reasoning') WHERE json_extract(request,'$.selection.kind')='cursor';
"#;

/// Version 11: runner-owned discussion links and independent delivery positions.
const V11: &str = r#"
CREATE TABLE conversation_links (
 id TEXT PRIMARY KEY, source_identity TEXT NOT NULL,
 a TEXT NOT NULL, b TEXT NOT NULL, active INTEGER NOT NULL,
 failure TEXT
);
CREATE UNIQUE INDEX conversation_link_pair ON conversation_links(a,b) WHERE active=1;
CREATE TABLE conversation_relay_positions (
 link TEXT NOT NULL REFERENCES conversation_links(id), source TEXT NOT NULL, target TEXT NOT NULL,
 position_json TEXT NOT NULL, seen_json TEXT NOT NULL, pending_json TEXT NOT NULL,
 delivered_turn TEXT, receipts_json TEXT NOT NULL DEFAULT '{}', failure TEXT,
 PRIMARY KEY(link,source)
);
"#;

/// Version 12: explicit native/opening/closing origins on retained link handovers.
const V12: &str = r#"
UPDATE conversation_relay_positions
SET pending_json=(SELECT json_group_array(json(
 CASE WHEN json_type(value,'$.origin') IS NULL
 THEN json_set(value,'$.origin',json_object('kind','turn')) ELSE value END
)) FROM json_each(conversation_relay_positions.pending_json));
"#;

/// Version 13: canonical unknown dispatcher evidence for retained native assignments.
const V13: &str = r#"
UPDATE session_bindings SET evidence=json_insert(evidence,'$.native_assignment.dispatcher',NULL)
WHERE json_type(evidence,'$.native_assignment')='object'
AND json_type(evidence,'$.native_assignment.dispatcher') IS NULL;
"#;

/// Version 14: retain the CI credential failure with its observation retry deadline.
const V14: &str = "ALTER TABLE poll_deferrals ADD COLUMN cause TEXT;";

/// Captured titles are normalized before runtime or work records are deserialized.
const V15: &str = r#"
UPDATE runtime_starts SET request=json_set(request,'$.title',json_extract(request,'$.key')) WHERE request IS NOT NULL AND (json_type(request,'$.title') IS NULL OR json_type(request,'$.title')='null');
UPDATE runtime_bindings SET request=json_set(request,'$.title',json_extract(request,'$.key')) WHERE request IS NOT NULL AND (json_type(request,'$.title') IS NULL OR json_type(request,'$.title')='null');
UPDATE procedure_responsibilities SET request=json_set(request,'$.title',json_extract(request,'$.key')) WHERE request IS NOT NULL AND (json_type(request,'$.title') IS NULL OR json_type(request,'$.title')='null');
UPDATE procedure_generations SET request=json_set(request,'$.title',json_extract(request,'$.key')) WHERE request IS NOT NULL AND (json_type(request,'$.title') IS NULL OR json_type(request,'$.title')='null');
UPDATE manual_standing_reservations SET request=json_set(request,'$.title',json_extract(request,'$.key')) WHERE request IS NOT NULL AND (json_type(request,'$.title') IS NULL OR json_type(request,'$.title')='null');
UPDATE session_bindings SET evidence=json_set(evidence,'$.native_placement.title',coalesce(
 display_title,
 (SELECT coalesce(json_extract(r.request,'$.title'),r.display_title)
  FROM runtime_bindings r
  WHERE r.semantic_key=json_extract(evidence,'$.native_placement.key') AND r.session=json_extract(evidence,'$.native_placement.session')),
 CASE

 WHEN length(json_extract(evidence,'$.native_placement.branch'))>8 AND substr(json_extract(evidence,'$.native_placement.branch'),-8)='/prepare'
 THEN json_extract(evidence,'$.role') || ' · ' || substr(json_extract(evidence,'$.native_placement.branch'),1,length(json_extract(evidence,'$.native_placement.branch'))-8) || ' · ' || json_extract(evidence,'$.native_placement.key')
 WHEN length(json_extract(evidence,'$.native_placement.branch'))>5 AND substr(json_extract(evidence,'$.native_placement.branch'),-5)='/work'
 THEN json_extract(evidence,'$.role') || ' · ' || substr(json_extract(evidence,'$.native_placement.branch'),1,length(json_extract(evidence,'$.native_placement.branch'))-5) || ' · ' || json_extract(evidence,'$.native_placement.key')
 END))
WHERE json_type(evidence,'$.native_placement')='object' AND (json_type(evidence,'$.native_placement.title') IS NULL OR json_type(evidence,'$.native_placement.title')='null');
UPDATE listing_copies SET record=json_set(record,'$.schema',3,'$.bindings',json((SELECT json_group_array(json(
 CASE WHEN json_type(value,'$.native_placement')='object' AND (json_type(value,'$.native_placement.title') IS NULL OR json_type(value,'$.native_placement.title')='null')
 THEN json_set(value,'$.native_placement.title',coalesce((SELECT coalesce(json_extract(r.request,'$.title'),r.display_title)
  FROM runtime_bindings r
  WHERE r.semantic_key=json_extract(value,'$.native_placement.key') AND r.session=json_extract(value,'$.native_placement.session')),
 CASE

 WHEN length(json_extract(value,'$.native_placement.branch'))>8 AND substr(json_extract(value,'$.native_placement.branch'),-8)='/prepare'
 THEN json_extract(value,'$.role') || ' · ' || substr(json_extract(value,'$.native_placement.branch'),1,length(json_extract(value,'$.native_placement.branch'))-8) || ' · ' || json_extract(value,'$.native_placement.key')
 WHEN length(json_extract(value,'$.native_placement.branch'))>5 AND substr(json_extract(value,'$.native_placement.branch'),-5)='/work'
 THEN json_extract(value,'$.role') || ' · ' || substr(json_extract(value,'$.native_placement.branch'),1,length(json_extract(value,'$.native_placement.branch'))-5) || ' · ' || json_extract(value,'$.native_placement.key')
 END)) ELSE value END
)) FROM json_each(listing_copies.record,'$.bindings')))) WHERE json_extract(record,'$.schema')<=2;
CREATE TEMP TABLE native_title_recovery_guard(title TEXT NOT NULL CHECK(length(title)>0));
INSERT INTO native_title_recovery_guard SELECT json_extract(evidence,'$.native_placement.title') FROM session_bindings WHERE json_type(evidence,'$.native_placement')='object';
INSERT INTO native_title_recovery_guard SELECT json_extract(value,'$.native_placement.title') FROM listing_copies,json_each(listing_copies.record,'$.bindings') WHERE json_type(value,'$.native_placement')='object';
DROP TABLE native_title_recovery_guard;
UPDATE session_bindings SET display_title=json_extract(evidence,'$.native_placement.title') WHERE json_type(evidence,'$.native_placement')='object';
"#;

/// Version 16: the board a project's reflection set up or found, recorded on
/// the machine rather than in the committed project layer. Applying it again
/// over a database that already holds the table changes nothing.
const V16: &str = r#"
CREATE TABLE IF NOT EXISTS project_boards (
    project TEXT PRIMARY KEY NOT NULL,
    board   TEXT NOT NULL
);
"#;

/// Version 17: conversations through connectors. Each conversation's bound
/// session, the start request captured for its first message and its own
/// answer-source position; each addressed message from receipt to its
/// answered turn, with its text and, from the time what accompanies it is
/// read, its captured context, boundary token and framed input, and when its
/// send to the runtime was begun; each
/// answer piece and when the connector accepted its post; each sender's
/// pending or sent refusal; whether the relay serving conversations runs; and
/// each connector's connection state. Applying it again over a database that
/// already holds the tables changes nothing.
const V17: &str = r#"
CREATE TABLE IF NOT EXISTS connected_conversations (
    company         TEXT NOT NULL,
    connector       TEXT NOT NULL,
    conversation    TEXT NOT NULL,
    semantic_key    TEXT NOT NULL,
    opening         TEXT,
    request         TEXT,
    session         TEXT,
    source_identity TEXT,
    position_json   TEXT,
    failure         TEXT,
    PRIMARY KEY (company, connector, conversation)
);
CREATE TABLE IF NOT EXISTS conversation_messages (
    company      TEXT NOT NULL,
    connector    TEXT NOT NULL,
    conversation TEXT NOT NULL,
    message      TEXT NOT NULL,
    member       TEXT NOT NULL,
    written_us   INTEGER NOT NULL,
    written_at   TEXT NOT NULL,
    arrived_at   TEXT NOT NULL,
    state        TEXT NOT NULL CHECK (state IN ('received', 'pending', 'sent', 'answered')),
    text         TEXT NOT NULL,
    context_json TEXT,
    boundary     TEXT,
    content      TEXT,
    attempted_at TEXT,
    turn         TEXT,
    failure      TEXT,
    PRIMARY KEY (company, connector, conversation, message)
);
CREATE TABLE IF NOT EXISTS conversation_answers (
    company      TEXT NOT NULL,
    connector    TEXT NOT NULL,
    conversation TEXT NOT NULL,
    turn         TEXT NOT NULL,
    piece        INTEGER NOT NULL,
    text         TEXT NOT NULL,
    posted_at    TEXT,
    failure      TEXT,
    PRIMARY KEY (company, connector, conversation, turn, piece)
);
CREATE TABLE IF NOT EXISTS conversation_refusals (
    company      TEXT NOT NULL,
    connector    TEXT NOT NULL,
    conversation TEXT NOT NULL,
    sender       TEXT NOT NULL,
    received_at  TEXT NOT NULL,
    replied_at   TEXT,
    failure      TEXT,
    PRIMARY KEY (company, connector, conversation, sender)
);
CREATE TABLE IF NOT EXISTS conversation_relay (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    failure    TEXT,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS connector_states (
    company    TEXT NOT NULL,
    connector  TEXT NOT NULL,
    state      TEXT NOT NULL CHECK (state IN ('connecting', 'connected', 'failed', 'stopped')),
    failure    TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (company, connector)
);
"#;
