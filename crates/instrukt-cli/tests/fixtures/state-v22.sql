-- Empty v22 database produced by the shipped migrations through ec32cfa.
BEGIN;
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
    tier         TEXT,
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
, refreshed_at TEXT);
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
    evidence         TEXT NOT NULL, source_comment INTEGER,
    PRIMARY KEY (todo_id, sequence)
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
    comment_id INTEGER NOT NULL, reply TEXT, reply_pending INTEGER NOT NULL DEFAULT 0,
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
    failure TEXT, delivery_kind TEXT NOT NULL DEFAULT 'occurrence', subject TEXT, event_type TEXT,
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
, cause TEXT);
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
CREATE UNIQUE INDEX stamps_decision_source ON stamps(todo_id,source_comment) WHERE source_comment IS NOT NULL;
CREATE TABLE holds (todo_id TEXT NOT NULL REFERENCES todos(id) ON DELETE CASCADE, source TEXT NOT NULL, reason TEXT NOT NULL, set_by TEXT NOT NULL, set_at TEXT NOT NULL, released_at TEXT, released_by TEXT);
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
CREATE TABLE todo_aliases (todo_id TEXT NOT NULL REFERENCES todos(id) ON DELETE CASCADE, slug TEXT NOT NULL, PRIMARY KEY(todo_id,slug));
CREATE TABLE agent_availability (
 agent TEXT NOT NULL, model TEXT NOT NULL DEFAULT '', status TEXT NOT NULL,
 status_until TEXT, reason TEXT, PRIMARY KEY(agent,model)
);
CREATE TABLE agent_quota (
 agent TEXT NOT NULL, model TEXT NOT NULL, status TEXT NOT NULL,
 status_until TEXT NOT NULL, reason TEXT NOT NULL, PRIMARY KEY(agent,model)
);
CREATE TABLE history_sources (
 source TEXT PRIMARY KEY, position INTEGER NOT NULL DEFAULT 0, imported INTEGER NOT NULL DEFAULT 0
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
CREATE TABLE history_projection_records (
 source TEXT NOT NULL, projection TEXT NOT NULL, native_id TEXT NOT NULL,
 thread_id TEXT NOT NULL, updated_at TEXT NOT NULL, payload_json TEXT NOT NULL,
 PRIMARY KEY(source,projection,native_id)
);
CREATE INDEX history_projection_threads ON history_projection_records(source,thread_id);
CREATE TABLE history_source_positions (source TEXT PRIMARY KEY,position_json TEXT NOT NULL);
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
CREATE TABLE project_boards (
    project TEXT PRIMARY KEY NOT NULL,
    board   TEXT NOT NULL
);
CREATE TABLE connected_conversations (
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
CREATE TABLE conversation_answers (
    company      TEXT NOT NULL,
    connector    TEXT NOT NULL,
    conversation TEXT NOT NULL,
    turn         TEXT NOT NULL,
    piece        INTEGER NOT NULL,
    text         TEXT NOT NULL,
    posted_at    TEXT,
    failure      TEXT, attempted_at TEXT,
    PRIMARY KEY (company, connector, conversation, turn, piece)
);
CREATE TABLE conversation_refusals (
    company      TEXT NOT NULL,
    connector    TEXT NOT NULL,
    conversation TEXT NOT NULL,
    sender       TEXT NOT NULL,
    received_at  TEXT NOT NULL,
    replied_at   TEXT,
    failure      TEXT,
    PRIMARY KEY (company, connector, conversation, sender)
);
CREATE TABLE conversation_relay (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    failure    TEXT,
    updated_at TEXT NOT NULL
);
CREATE TABLE connector_states (
    company    TEXT NOT NULL,
    connector  TEXT NOT NULL,
    state      TEXT NOT NULL CHECK (state IN ('connecting', 'connected', 'failed', 'stopped')),
    failure    TEXT,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (company, connector)
);
CREATE TABLE "conversation_messages" (
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
    failure      TEXT, admitted_at TEXT, unavailable_since TEXT, turn_update TEXT, turn_ended_at TEXT,
    PRIMARY KEY (company, connector, conversation, message)
);
CREATE TABLE runner_workers (
    worker  TEXT PRIMARY KEY,
    failure TEXT NOT NULL,
    since   TEXT NOT NULL
);
CREATE TABLE runner_skipped_wake (
    id    INTEGER PRIMARY KEY CHECK (id = 1),
    at    TEXT NOT NULL,
    cause TEXT NOT NULL
);
CREATE TABLE runner_admission (
    id      INTEGER PRIMARY KEY CHECK (id = 1),
    at      TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('replaced', 'kept'))
);
CREATE TABLE "consumer_sources" (
    consumer TEXT NOT NULL,
    source TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('active', 'retiring', 'retired', 'abandoned')),
    prepared INTEGER NOT NULL,
    PRIMARY KEY (consumer, source)
);
CREATE INDEX consumer_open_announcements ON consumer_outcomes (
    json_extract(outcome, '$.parked.announced')
) WHERE json_extract(outcome, '$.parked.announcement_receipt') IS NOT NULL
      AND json_extract(outcome, '$.parked.announced') = 0;
PRAGMA user_version=22;
COMMIT;
