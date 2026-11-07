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

PRAGMA user_version=1;
