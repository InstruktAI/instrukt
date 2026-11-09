//! The state database's migrations, oldest first. instrukt owns every one;
//! each moves the schema forward by one version and is never edited once
//! shipped. Tables and columns are the `state-database` spec's.

/// Every migration this build ships; the schema version is their count.
pub(super) const MIGRATIONS: &[&str] = &[V1, V2, V3, V4, V5, V6, V7, V8, V9, V10, V11, V12, V13];

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
 (SELECT json_group_array(json(CASE WHEN json_extract(value,'$.agent')='gemini' THEN json_set(value,'$.agent','antigravity') ELSE value END)) FROM json_each(request,'$.preferences')))
 WHERE json_type(request,'$.preferences')='array';
UPDATE runtime_bindings SET request=json_remove(json_set(request,
 '$.selection.profile',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.kind',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.instance',json_extract(request,'$.selection.provider')),'$.selection.family','$.selection.provider')
 WHERE json_type(request,'$.selection.family') IS NOT NULL;
UPDATE runtime_bindings SET request=json_set(request,'$.preferences',
 (SELECT json_group_array(json(CASE WHEN json_extract(value,'$.agent')='gemini' THEN json_set(value,'$.agent','antigravity') ELSE value END)) FROM json_each(request,'$.preferences')))
 WHERE json_type(request,'$.preferences')='array';
UPDATE procedure_responsibilities SET request=json_remove(json_set(request,
 '$.selection.profile',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.kind',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.instance',json_extract(request,'$.selection.provider')),'$.selection.family','$.selection.provider')
 WHERE json_type(request,'$.selection.family') IS NOT NULL;
UPDATE procedure_responsibilities SET request=json_set(request,'$.preferences',
 (SELECT json_group_array(json(CASE WHEN json_extract(value,'$.agent')='gemini' THEN json_set(value,'$.agent','antigravity') ELSE value END)) FROM json_each(request,'$.preferences')))
 WHERE json_type(request,'$.preferences')='array';
UPDATE procedure_generations SET request=json_remove(json_set(request,
 '$.selection.profile',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.kind',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.instance',json_extract(request,'$.selection.provider')),'$.selection.family','$.selection.provider')
 WHERE json_type(request,'$.selection.family') IS NOT NULL;
UPDATE procedure_generations SET request=json_set(request,'$.preferences',
 (SELECT json_group_array(json(CASE WHEN json_extract(value,'$.agent')='gemini' THEN json_set(value,'$.agent','antigravity') ELSE value END)) FROM json_each(request,'$.preferences')))
 WHERE json_type(request,'$.preferences')='array';
UPDATE manual_standing_reservations SET request=json_remove(json_set(request,
 '$.selection.profile',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.kind',CASE json_extract(request,'$.selection.family') WHEN 'gemini' THEN 'antigravity' ELSE json_extract(request,'$.selection.family') END,
 '$.selection.instance',json_extract(request,'$.selection.provider')),'$.selection.family','$.selection.provider')
 WHERE json_type(request,'$.selection.family') IS NOT NULL;
UPDATE manual_standing_reservations SET request=json_set(request,'$.preferences',
 (SELECT json_group_array(json(CASE WHEN json_extract(value,'$.agent')='gemini' THEN json_set(value,'$.agent','antigravity') ELSE value END)) FROM json_each(request,'$.preferences')))
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
