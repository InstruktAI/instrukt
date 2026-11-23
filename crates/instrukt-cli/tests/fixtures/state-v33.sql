-- Empty v33 database produced by the shipped migrations through eef420c.
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
CREATE TABLE runtime_bindings (
 runtime TEXT NOT NULL, semantic_key TEXT NOT NULL, kind TEXT NOT NULL, project TEXT,
 subject TEXT NOT NULL, session TEXT NOT NULL, display_title TEXT, project_id TEXT, checkout TEXT NOT NULL,
 worktree TEXT, branch TEXT, base_head TEXT, accepted INTEGER NOT NULL, request TEXT, generation INTEGER NOT NULL DEFAULT 1, start_key TEXT, replaced_session TEXT, legacy_modes TEXT,
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
    failure         TEXT, display_title TEXT,
    PRIMARY KEY (company, connector, conversation)
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
    superseded_at TEXT, retry_at TEXT, failed_turns INTEGER NOT NULL DEFAULT 0, context_delivery_ids TEXT, send_unknown_at TEXT, dispatch_command_id TEXT, dispatch_write_at TEXT, sender_is_admin INTEGER NOT NULL DEFAULT 0, is_direct INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (company, connector, conversation, message)
);
CREATE UNIQUE INDEX conversation_admission_order
           ON conversation_messages(admission_position);
CREATE INDEX conversation_superseded_failures
           ON conversation_messages(superseded_at)
           WHERE state='failed' AND superseded_at IS NOT NULL;
CREATE TABLE conversation_context_receipts (
    company TEXT NOT NULL,
    connector TEXT NOT NULL,
    conversation TEXT NOT NULL,
    message TEXT NOT NULL,
    delivery TEXT NOT NULL,
    retired_at TEXT,
    PRIMARY KEY (company, connector, conversation, message)
);
CREATE INDEX conversation_context_receipt_delivery
    ON conversation_context_receipts(company, connector, conversation, delivery);
CREATE INDEX conversation_context_receipt_retired
    ON conversation_context_receipts(retired_at) WHERE retired_at IS NOT NULL;
CREATE TABLE connector_read_back_seed (
    company TEXT NOT NULL,
    connector TEXT NOT NULL,
    first_seen INTEGER NOT NULL,
    PRIMARY KEY(company, connector)
);
CREATE TABLE conversation_missed_identities (
    company TEXT NOT NULL, connector TEXT NOT NULL, conversation TEXT NOT NULL,
    message TEXT NOT NULL, written_us INTEGER NOT NULL,
    PRIMARY KEY(company, connector, conversation, message)
);
CREATE INDEX conversation_missed_identities_by_time
    ON conversation_missed_identities(written_us);
CREATE TABLE "conversation_refusals" (
    company TEXT NOT NULL, connector TEXT NOT NULL, conversation TEXT NOT NULL,
    sender TEXT NOT NULL, reason TEXT NOT NULL CHECK(reason IN ('not_own','unavailable')),
    received_at TEXT NOT NULL, replied_at TEXT, failure TEXT, attempted_at TEXT,
    PRIMARY KEY(company,connector,conversation,sender,reason)
);
CREATE TABLE "conversation_answers" (
        company TEXT NOT NULL, connector TEXT NOT NULL, conversation TEXT NOT NULL,
        turn TEXT NOT NULL, piece INTEGER NOT NULL, text TEXT NOT NULL,
        posted_at TEXT, failure TEXT, attempted_at TEXT, admission_position INTEGER,
        provider_post_id TEXT,
        PRIMARY KEY(company,connector,conversation,turn,piece)
    );
CREATE TABLE runtime_generations (
            runtime TEXT NOT NULL, semantic_key TEXT NOT NULL, generation INTEGER NOT NULL,
            session TEXT, request TEXT, project_id TEXT, checkout TEXT, worktree TEXT,
            branch TEXT, base_head TEXT, accepted INTEGER NOT NULL, display_title TEXT,
            start_key TEXT, replaced_session TEXT, legacy_evidence TEXT, legacy_modes TEXT,
            PRIMARY KEY(runtime,semantic_key,generation)
        );
CREATE TABLE mcp_registrations (
            return_id      TEXT PRIMARY KEY NOT NULL,
            company        TEXT,
            server         TEXT,
            host_client    TEXT,
            issuer         TEXT NOT NULL,
            declared       INTEGER NOT NULL,
            client_id      TEXT NOT NULL,
            client_secret  TEXT,
            secret_digest  TEXT,
            return_address TEXT NOT NULL,
            registered_at  TEXT NOT NULL,
            retired_at     TEXT
        );
CREATE UNIQUE INDEX mcp_registrations_host_client
            ON mcp_registrations(host_client)
            WHERE host_client IS NOT NULL AND retired_at IS NULL;
CREATE UNIQUE INDEX mcp_registrations_company_server
            ON mcp_registrations(company, server, declared)
            WHERE company IS NOT NULL AND server IS NOT NULL AND retired_at IS NULL;
CREATE TABLE company_issuers (
            company          TEXT PRIMARY KEY NOT NULL,
            issuer_id        TEXT NOT NULL UNIQUE,
            public_jwks_json TEXT NOT NULL,
            updated_at       TEXT NOT NULL
        );
CREATE TABLE mcp_generations (
            company    TEXT NOT NULL,
            person     TEXT NOT NULL,
            server     TEXT NOT NULL,
            generation INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (company, person, server)
        );
CREATE TABLE mcp_logins (
            company        TEXT NOT NULL,
            person         TEXT NOT NULL,
            server         TEXT NOT NULL,
            resource       TEXT NOT NULL,
            subject        TEXT,
            return_id      TEXT NOT NULL,
            access_token   TEXT NOT NULL,
            expires_at     TEXT,
            checked        INTEGER NOT NULL DEFAULT 1,
            refresh_token  TEXT,
            scopes         TEXT,
            signed_in_at   TEXT NOT NULL,
            renewed_at     TEXT,
            renewing_until TEXT,
            PRIMARY KEY (company, person, server)
        );
CREATE TABLE mcp_sign_ins (
            state_digest   TEXT PRIMARY KEY NOT NULL,
            company        TEXT NOT NULL,
            person         TEXT NOT NULL,
            server         TEXT NOT NULL,
            return_id      TEXT NOT NULL,
            resource       TEXT NOT NULL,
            scopes         TEXT,
            generation     INTEGER NOT NULL,
            connector      TEXT NOT NULL,
            conversation   TEXT NOT NULL,
            message        TEXT NOT NULL,
            recipient      TEXT NOT NULL,
            code_verifier  TEXT,
            nonce          TEXT,
            link           TEXT,
            created_at     TEXT NOT NULL,
            expires_at     TEXT NOT NULL,
            state          TEXT NOT NULL CHECK (state IN ('open','exchanging','completed','declined','refused','failed','undelivered','expired','cancelled')),
            link_sent_at   TEXT,
            told_at        TEXT,
            closed_at      TEXT,
            failure        TEXT
        );
CREATE TABLE mcp_sign_in_waiters (
            state_digest   TEXT NOT NULL,
            waiter         TEXT NOT NULL,
            waiting_until  TEXT NOT NULL,
            PRIMARY KEY (state_digest, waiter)
        );
PRAGMA user_version=33;
COMMIT;
