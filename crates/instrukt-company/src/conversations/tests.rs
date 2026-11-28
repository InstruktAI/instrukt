use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use super::_delivery::{
    FAILED_RETRY_MAX_ATTEMPTS, NativeDelivery, opening_procedure, opening_procedure_for_role,
    record_delivery_failure, send,
};
use super::_relay::settle_failed_messages;
use super::_store::{Conversation, Message, Stage};
use super::_turns::{STANDARD_ERROR_REPLY, UNOBSERVED_SENT_GRACE, reconcile_thread};
use super::{ConnectionState, ConversationError, SpeakerError, effective_since, split, unfinished};
use instrukt_core::config::{
    AgentSelection, Agents, COMPANY_AGENT_ROLE, Env, SHIPPED_AGENTS, SessionScope,
};
use instrukt_core::home::Home;
use instrukt_core::ids::{InputId, ProjectSlug, RoleName, RunId, SessionRef};
use instrukt_core::interfaces::{CallerIdentity, CallerIdentityLookup};
use instrukt_core::state_db::StateDb;
use instrukt_runner::runtime::{
    ActiveTurn, Capability, Conversation as RuntimeConversation, DeliveryRealization,
    DispatchError, ExecutionRootMode, ExecutionRootStatus, ObservedTurn, OpeningInput, Presence,
    ProviderSnapshot, RuntimeAdapter, RuntimeError, RuntimeStart, SelectionOperation, SendEvidence,
    SessionState, Thread, TurnState,
};
use instrukt_runner::state::RunnerState;
use jiff::{SignedDuration, Timestamp};

struct StubAdapter {
    thread: Mutex<Result<Thread, RuntimeError>>,
    reject_send: bool,
}

impl StubAdapter {
    fn with_thread(thread: Thread) -> Self {
        Self {
            thread: Mutex::new(Ok(thread)),
            reject_send: false,
        }
    }

    fn with_error(error: RuntimeError) -> Self {
        Self {
            thread: Mutex::new(Err(error)),
            reject_send: false,
        }
    }

    fn rejecting_send(thread: Thread) -> Self {
        Self {
            thread: Mutex::new(Ok(thread)),
            reject_send: true,
        }
    }
}

impl CallerIdentityLookup for StubAdapter {
    fn caller_variables(&self) -> &'static [&'static str] {
        &[]
    }
    fn lookup_caller(&self, _env: &Env) -> Option<CallerIdentity> {
        None
    }
}

impl RuntimeAdapter for StubAdapter {
    fn runtime_name(&self) -> &'static str {
        "t3"
    }
    fn provider_snapshot(&self, _home: &Home) -> Result<ProviderSnapshot, RuntimeError> {
        Err(RuntimeError::Refused("stub".into()))
    }
    fn selection_realization(&self, _operation: SelectionOperation) -> String {
        "stub".into()
    }
    fn native_selection(&self, selection: &AgentSelection) -> AgentSelection {
        selection.clone()
    }
    fn launch_scope(&self, _home: &Home) -> String {
        "stub".into()
    }
    fn resolve_launch(
        &self,
        request: &RuntimeStart,
        _existing: Option<&SessionRef>,
    ) -> Result<RuntimeStart, RuntimeError> {
        Ok(request.clone())
    }
    fn accepted_start(&self, _request: &RuntimeStart) -> Result<bool, RuntimeError> {
        Ok(true)
    }
    fn create_thread(&self, _request: &RuntimeStart) -> Result<SessionRef, RuntimeError> {
        Err(RuntimeError::Refused("stub".into()))
    }
    fn verify_start(&self, _request: &RuntimeStart, _thread: &Thread) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn verify_placement(
        &self,
        _request: &RuntimeStart,
        _thread: &Thread,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn verify_thread_project(
        &self,
        _checkout: &Path,
        _thread: &Thread,
        _key: Option<&str>,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn execution_root(
        &self,
        _root: &Path,
        _mode: ExecutionRootMode,
    ) -> Result<ExecutionRootStatus, RuntimeError> {
        Err(RuntimeError::Refused("stub".into()))
    }
    fn delivery_realization(&self) -> DeliveryRealization {
        DeliveryRealization::Queued
    }
    fn supports(&self, _capability: Capability) -> bool {
        false
    }
    fn observe_send(
        &self,
        _session: &SessionRef,
        _input: &str,
        _command_id: Option<&str>,
    ) -> Result<SendEvidence, RuntimeError> {
        Ok(SendEvidence::Unresolved)
    }
    fn list(&self, _checkout: &Path) -> Result<Vec<Thread>, RuntimeError> {
        Ok(Vec::new())
    }
    fn dispatch_send(
        &self,
        session: &SessionRef,
        command_id: &str,
        _message: &str,
        before_frame: &mut (dyn FnMut() -> Result<(), RuntimeError> + Send),
    ) -> Result<(), DispatchError> {
        before_frame().map_err(DispatchError::BeforeFrame)?;
        if self.reject_send {
            return Err(DispatchError::AfterFrame(
                RuntimeError::CommandPreviouslyRejected {
                    command_id: command_id.to_owned(),
                    detail: format!("rejected for {session}"),
                },
            ));
        }
        Ok(())
    }
    fn send(&self, _session: &SessionRef, _text: &str) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn read_state(&self, _session: &SessionRef) -> Result<Thread, RuntimeError> {
        self.thread
            .lock()
            .unwrap()
            .as_ref()
            .cloned()
            .map_err(|err| RuntimeError::Refused(err.to_string()))
    }
    fn read_state_within(
        &self,
        session: &SessionRef,
        _timeout: Duration,
    ) -> Result<Thread, RuntimeError> {
        self.read_state(session)
    }
    fn read_conversation(&self, session: &SessionRef) -> Result<RuntimeConversation, RuntimeError> {
        Ok(RuntimeConversation {
            session: session.clone(),
            messages: Vec::new(),
            limit: 0,
            bounded: false,
        })
    }
    fn send_certainly_rejected(&self, _session: &SessionRef, error: &RuntimeError) -> bool {
        matches!(error, RuntimeError::CommandPreviouslyRejected { .. })
    }
}

fn stub_thread(session: &SessionRef, state: SessionState) -> Thread {
    Thread {
        session: session.clone(),
        project_id: None,
        title: Some("Acme".into()),
        semantic_key: None,
        state,
        presence: Presence::Present,
        branch: None,
        worktree: None,
        provider: None,
        model: None,
        interrupted: false,
        settled: false,
        settled_at: None,
        native_status: state.as_str().to_owned(),
        run_id: None,
        active_turn: None,
        recent_turns: Vec::new(),
        activity: None,
        native_details: serde_json::Value::Null,
    }
}

#[test]
fn a_messages_interval_is_its_own_or_the_relays_failure_from_its_admission() {
    let at = |text: &str| Some(text.parse::<Timestamp>().unwrap());
    assert_eq!(
        effective_since(None, at("2026-11-19T10:00:00Z"), None),
        None
    );
    assert_eq!(
        effective_since(at("2026-11-19T10:05:00Z"), at("2026-11-19T10:00:00Z"), None),
        at("2026-11-19T10:05:00Z")
    );
    assert_eq!(
        effective_since(None, at("2026-11-19T10:00:00Z"), at("2026-11-19T09:00:00Z")),
        at("2026-11-19T10:00:00Z")
    );
    assert_eq!(
        effective_since(None, at("2026-11-19T10:00:00Z"), at("2026-11-19T10:02:00Z")),
        at("2026-11-19T10:02:00Z")
    );
    assert_eq!(
        effective_since(
            at("2026-11-19T10:01:00Z"),
            at("2026-11-19T10:00:00Z"),
            at("2026-11-19T10:02:00Z")
        ),
        at("2026-11-19T10:01:00Z")
    );
    assert_eq!(
        effective_since(None, None, at("2026-11-19T09:00:00Z")),
        at("2026-11-19T09:00:00Z")
    );
}

fn declared(pending: bool) -> instrukt_core::index::GlobalIndex {
    let delivered_by = if pending {
        "  delivered_by: [draft-it]\n"
    } else {
        ""
    };
    serde_yaml_ng::from_str(&format!(
        "schema_version: 3\nsnippets:\n- id: general/procedure/company-assistance\n  description: Assist.\n  type: procedure\n  scope: global\n  domain: general\n  path: general/procedure/company-assistance.md\n  source_project: demo\n  role: company-agent\n{delivered_by}"
    ))
    .unwrap()
}

#[test]
fn a_conversation_starts_only_from_a_delivered_procedure_declaring_its_role() {
    let delivered = declared(false);
    let found = opening_procedure(
        COMPANY_AGENT_ROLE,
        delivered
            .procedures_declaring(&RoleName::parse(COMPANY_AGENT_ROLE).unwrap())
            .into_iter()
            .next(),
    );
    assert_eq!(found.unwrap(), &delivered.snippets[0]);
    let pending = declared(true);
    let refused = opening_procedure(
        COMPANY_AGENT_ROLE,
        pending
            .procedures_declaring(&RoleName::parse(COMPANY_AGENT_ROLE).unwrap())
            .into_iter()
            .next(),
    );
    assert!(
        matches!(
            refused,
            Err(ConversationError::NoProcedure { ref role }) if role == "company-agent"
        ),
        "{refused:?}"
    );
    assert!(matches!(
        opening_procedure(COMPANY_AGENT_ROLE, None),
        Err(ConversationError::NoProcedure { ref role }) if role == "company-agent"
    ));
}

#[test]
fn a_technical_coordinator_connector_opens_from_its_own_procedure_not_company_assistance() {
    let project = ProjectSlug::parse("instrukt").unwrap();
    let corpus = instrukt_core::docs::Corpus::from_sources(
        Some(project.clone()),
        None,
        [
            instrukt_core::docs::SourceFile {
                path: "docs/general/procedure/company-assistance.md".to_owned(),
                text: include_str!("../../../../docs/general/procedure/company-assistance.md")
                    .to_owned(),
            },
            instrukt_core::docs::SourceFile {
                path: "docs/general/procedure/technical-coordinator.md".to_owned(),
                text: include_str!("../../../../docs/general/procedure/technical-coordinator.md")
                    .to_owned(),
            },
        ],
    );
    let snippets = corpus
        .snippets()
        .iter()
        .map(|snippet| {
            let current = snippet.current();
            let front = current.frontmatter();
            instrukt_core::index::IndexEntry {
                id: snippet.id().clone(),
                description: front.description.clone().unwrap_or_default(),
                taxonomy: snippet.taxonomy().dir().to_owned(),
                scope: instrukt_core::index::IndexScope::Global,
                domain: snippet.domain().to_string(),
                path: current.path().to_path_buf(),
                source_project: project.clone(),
                visibility: None,
                delivered_by: Vec::new(),
                superseded_by: Vec::new(),
                activation_trigger: front.entry.activation_trigger.clone(),
                role: front.entry.role.clone(),
                subscriptions: Vec::new(),
                subscription_home: None,
                subscription_identity: Default::default(),
                subscription_attendance: Default::default(),
                required_reads: Vec::new(),
                parameters: Vec::new(),
                interruptible: false,
                content_sha256: None,
                compressed_source_sha256: None,
                compressed_content_sha256: None,
            }
        })
        .collect();
    let index = instrukt_core::index::GlobalIndex {
        schema_version: 3,
        snippets,
        third_party: Vec::new(),
    };
    let agents = Agents::parse(SHIPPED_AGENTS).unwrap();
    let opened = opening_procedure_for_role(
        &index,
        &RoleName::parse("technical-coordinator").unwrap(),
        &agents,
    )
    .unwrap();
    let expected =
        instrukt_core::ids::SnippetId::parse("general/procedure/technical-coordinator").unwrap();
    assert_eq!(opened.id, expected);
}

fn cut(text: &str, limit: usize) -> (Vec<usize>, bool) {
    let parts = split(text, limit);
    let counts = parts.iter().map(|part| part.chars().count()).collect();
    (counts, parts.concat() == text)
}

#[test]
fn a_part_ends_after_the_last_line_break_in_its_second_half() {
    let text = format!("{}\n{}\n{}", "a".repeat(22), "b".repeat(5), "c".repeat(30));
    assert_eq!(cut(&text, 40), (vec![29, 30], true));
}

#[test]
fn a_line_break_in_the_first_half_does_not_end_a_part() {
    let text = format!("{}\n{}", "a".repeat(10), "b".repeat(50));
    assert_eq!(cut(&text, 40), (vec![40, 21], true));
}

#[test]
fn the_remainder_is_the_last_part_and_no_empty_part_follows() {
    assert_eq!(cut(&"a".repeat(41), 40), (vec![40, 1], true));
    assert_eq!(cut(&"a".repeat(80), 40), (vec![40, 40], true));
    assert_eq!(
        cut(&format!("{}\n  ", "a".repeat(39)), 40),
        (vec![40, 2], true)
    );
    assert_eq!(cut(&"a".repeat(5), 40), (vec![5], true));
    assert_eq!(cut("", 40), (vec![], true));
}

#[test]
fn the_limit_counts_characters_not_bytes() {
    assert_eq!(cut(&"é".repeat(41), 40), (vec![40, 1], true));
}

fn bound_before_start(session: &SessionRef) -> (tempfile::TempDir, instrukt_core::home::Home) {
    let root = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
    let home = instrukt_core::home::Home::at_user_home(root.path().to_path_buf()).for_writing();
    let mut db = StateDb::open(&home).unwrap();
    db.write::<_, instrukt_core::state_db::StateDbError>(|tx| {
        tx.execute(
            "INSERT INTO connected_conversations(company,connector,conversation,semantic_key,role,session_scope) VALUES ('acme','slack','C1','company:acme:slack:C1','company-agent','conversation')",
            [],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO runtime_bindings(runtime,semantic_key,kind,subject,session,checkout,accepted,generation) VALUES ('t3','company:acme:slack:C1','conversation','company:acme:slack:C1',?1,'/h/companies/acme',0,1)",
            [session.to_string()],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    drop(db);
    (root, home)
}

#[test]
fn a_conversation_session_is_recognised_from_its_binding_before_it_starts() {
    let session: SessionRef = "t3:thread-before-start".parse().unwrap();
    let (_root, home) = bound_before_start(&session);
    let acme = ProjectSlug::parse("acme").unwrap();
    let assignment = super::conversation_assignment(&home, &session)
        .unwrap()
        .expect("the bound session is the conversation's");
    assert_eq!(assignment.company, Some(acme.clone()));
    assert_eq!(assignment.role.as_str(), COMPANY_AGENT_ROLE);
    assert!(
        super::conversation_binding(&home, &session)
            .unwrap()
            .is_none()
    );
    // Pre-acceptance session fails closed with retryable SpeakerError::Unbound.
    let adapter = StubAdapter::with_thread(stub_thread(&session, SessionState::Running));
    let err = super::speaker(&home, &session, &adapter).unwrap_err();
    assert!(matches!(err, SpeakerError::Unbound { ref company } if company == acme.as_str()));
    assert!(err.is_retryable());
}

#[test]
fn a_session_no_binding_names_is_no_conversations() {
    let (_root, home) = bound_before_start(&"t3:thread-before-start".parse().unwrap());
    let other: SessionRef = "t3:someone-else".parse().unwrap();
    assert!(
        super::conversation_assignment(&home, &other)
            .unwrap()
            .is_none()
    );
    let adapter = StubAdapter::with_thread(stub_thread(&other, SessionState::Running));
    assert!(matches!(super::speaker(&home, &other, &adapter), Ok(None)));
}

#[test]
fn speaker_enforces_three_outcomes_and_retired_generation_refusal() {
    let active_session: SessionRef = "t3:thread-active".parse().unwrap();
    let retired_session: SessionRef = "t3:thread-retired".parse().unwrap();
    let root = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
    let home = instrukt_core::home::Home::at_user_home(root.path().to_path_buf()).for_writing();
    let mut db = StateDb::open(&home).unwrap();
    db.write::<_, instrukt_core::state_db::StateDbError>(|tx| {
        tx.execute(
            "INSERT INTO connected_conversations(company,connector,conversation,semantic_key,session,role,session_scope)
             VALUES ('acme','slack','C1','company:acme:slack:C1',?1,'company-agent','conversation')",
            [active_session.to_string()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO runtime_generations(runtime,semantic_key,generation,session,checkout,accepted)
             VALUES ('t3','company:acme:slack:C1',1,?1,'/h/companies/acme',1)",
            [retired_session.to_string()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO conversation_messages(company,connector,conversation,message,sender,sender_name,sender_email,sender_is_admin,written_us,written_at,arrived_at,state,text,is_direct,dispatch_command_id)
             VALUES ('acme','slack','C1','m1','U1','Ada','ada@acme.example',1,1000,'2026-11-20T10:00:00Z','2026-11-20T10:00:00Z','sent','hello',1,'cmd-1')",
            [],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    drop(db);

    let now: Timestamp = "2026-11-20T10:00:05Z".parse().unwrap();

    // Retired generation (generation > 0) fails closed deterministically.
    let retired_adapter =
        StubAdapter::with_thread(stub_thread(&retired_session, SessionState::Running));
    let retired_err = super::speaker(&home, &retired_session, &retired_adapter).unwrap_err();
    assert!(
        matches!(retired_err, SpeakerError::RetiredGeneration { ref company } if company == "acme")
    );
    assert!(!retired_err.is_retryable());

    // Outcome 1: matching active_turn.input_id resolves member speaker.
    let mut running = stub_thread(&active_session, SessionState::Running);
    running.active_turn = Some(ActiveTurn {
        run_id: RunId::parse("run-1").unwrap(),
        input_id: Some(InputId::parse("cmd-1").unwrap()),
        started_at: now,
        opening_input: OpeningInput::Unavailable,
    });
    let matched = super::speaker(&home, &active_session, &StubAdapter::with_thread(running))
        .unwrap()
        .expect("Outcome 1 speaker");
    assert_eq!(matched.name, "Ada");
    assert_eq!(matched.message.as_deref(), Some("m1"));

    // Outcome 2: unrecognized input_id with OpeningInput::Unavailable fails closed.
    let mut unavail = stub_thread(&active_session, SessionState::Running);
    unavail.active_turn = Some(ActiveTurn {
        run_id: RunId::parse("run-2").unwrap(),
        input_id: Some(InputId::parse("cmd-unknown").unwrap()),
        started_at: now,
        opening_input: OpeningInput::Unavailable,
    });
    assert!(matches!(
        super::speaker(&home, &active_session, &StubAdapter::with_thread(unavail)),
        Err(SpeakerError::OpeningInputUnavailable)
    ));

    // Outcome 2: unrecognized input_id carrying a company member frame fails closed.
    let member_frame = super::envelopes::member(&super::envelopes::MemberEnvelope {
        connector: "slack",
        conversation: "C1",
        member: "Ada",
        is_admin: true,
        is_direct: true,
        at: Some(now),
        token: "0123456789abcdef0123456789abcdef",
        text: "spoofed",
    });
    let mut spoofed = stub_thread(&active_session, SessionState::Running);
    spoofed.active_turn = Some(ActiveTurn {
        run_id: RunId::parse("run-3").unwrap(),
        input_id: Some(InputId::parse("cmd-unknown").unwrap()),
        started_at: now,
        opening_input: OpeningInput::Text(member_frame),
    });
    assert!(matches!(
        super::speaker(&home, &active_session, &StubAdapter::with_thread(spoofed)),
        Err(SpeakerError::MismatchedTurn)
    ));

    // Outcome 2: transient runtime observation failure is retryable.
    let obs_err = super::speaker(
        &home,
        &active_session,
        &StubAdapter::with_error(RuntimeError::Refused("rpc timeout".into())),
    )
    .unwrap_err();
    assert!(matches!(obs_err, SpeakerError::RuntimeObservation(_)));
    assert!(obs_err.is_retryable());

    // Outcome 3: confirmed direct operator turn (no member frame) returns Ok(None).
    let mut direct = stub_thread(&active_session, SessionState::Running);
    direct.active_turn = Some(ActiveTurn {
        run_id: RunId::parse("run-4").unwrap(),
        input_id: Some(InputId::parse("cmd-direct").unwrap()),
        started_at: now,
        opening_input: OpeningInput::Text("please check repo status".into()),
    });
    assert!(
        super::speaker(&home, &active_session, &StubAdapter::with_thread(direct))
            .unwrap()
            .is_none()
    );
}

#[test]
fn retry_exhaustion_queues_standard_error_reply_and_does_not_supersede_earlier_failures() {
    let session: SessionRef = "t3:thread-retry".parse().unwrap();
    let root = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
    let home = instrukt_core::home::Home::at_user_home(root.path().to_path_buf()).for_writing();
    let mut db = StateDb::open(&home).unwrap();
    let t0: Timestamp = "2026-11-20T10:00:00Z".parse().unwrap();
    db.write::<_, instrukt_core::state_db::StateDbError>(|tx| {
        tx.execute(
            "INSERT INTO connected_conversations(company,connector,conversation,semantic_key,session,role,session_scope)
             VALUES ('acme','slack','C1','company:acme:slack:C1',?1,'company-agent','conversation')",
            [session.to_string()],
        )
        .unwrap();
        // Earlier failed message from U1 (admission_position = 1).
        tx.execute(
            "INSERT INTO conversation_messages(company,connector,conversation,message,sender,sender_name,written_us,written_at,arrived_at,admission_position,state,text,is_direct,failed_turns)
             VALUES ('acme','slack','C1','m-earlier','U1','Ada',1000,'2026-11-20T09:50:00Z','2026-11-20T09:50:00Z',1,'failed','first',1,1)",
            [],
        )
        .unwrap();
        // Later message from U1 already at FAILED_RETRY_MAX_ATTEMPTS = 3 retries (4th attempt failing now).
        tx.execute(
            "INSERT INTO conversation_messages(company,connector,conversation,message,sender,sender_name,written_us,written_at,arrived_at,admission_position,state,text,is_direct,dispatch_command_id,failed_turns)
             VALUES ('acme','slack','C1','m-exhausted','U1','Ada',2000,'2026-11-20T10:00:00Z','2026-11-20T10:00:00Z',2,'sent','second',1,'cmd-exhaust',?1)",
            [FAILED_RETRY_MAX_ATTEMPTS],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    drop(db);

    let state = RunnerState::new(&home);
    let c = Conversation {
        company: ProjectSlug::parse("acme").unwrap(),
        connector: "slack".into(),
        conversation: "C1".into(),
        semantic_key: "company:acme:slack:C1".into(),
        opening: None,
        opening_conversation: None,
        request: None,
        session: Some(session.clone()),
        role: RoleName::parse(COMPANY_AGENT_ROLE).unwrap(),
        session_scope: SessionScope::Conversation,
        needs_explicit_rollover: false,
    };
    let mut thread = stub_thread(&session, SessionState::EndedFailed);
    thread.recent_turns.push(ObservedTurn {
        run_id: RunId::parse("run-fail").unwrap(),
        input_id: Some(InputId::parse("cmd-exhaust").unwrap()),
        state: TurnState::Failed {
            started_at: Some(t0),
            completed_at: t0 + SignedDuration::from_secs(10),
        },
    });
    let outcome = reconcile_thread(
        &state,
        &c,
        &thread,
        4000,
        t0 + SignedDuration::from_secs(15),
    )
    .unwrap();
    assert_eq!(
        outcome.terminal_errors,
        vec![("C1".into(), "m-exhausted".into())]
    );

    // Standard error reply queued with source_message and turn='error:m-exhausted'.
    let (answer_turn, answer_text, source_msg): (String, String, Option<String>) = state
        .read(|conn, path| {
            conn.query_row(
                "SELECT turn,text,source_message FROM conversation_answers WHERE company='acme' AND connector='slack' AND conversation='C1'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(instrukt_runner::state::sql(path))
        })
        .unwrap()
        .unwrap();
    assert_eq!(answer_turn, "error:m-exhausted");
    assert_eq!(answer_text, STANDARD_ERROR_REPLY);
    assert_eq!(source_msg.as_deref(), Some("m-exhausted"));

    // Even after posting the terminal error reply, settle_failed_messages must NOT supersede m-earlier.
    state
        .write::<_, instrukt_runner::state::StateError>(|tx| {
            tx.execute(
                "UPDATE conversation_answers SET posted_at='2026-11-20T10:01:00Z' WHERE turn='error:m-exhausted'",
                [],
            )
            .unwrap();
            settle_failed_messages(tx, "acme", "slack", "C1", t0 + SignedDuration::from_secs(60))?;
            Ok(())
        })
        .unwrap();
    let earlier_superseded: Option<String> = state
        .read(|conn, path| {
            conn.query_row(
                "SELECT superseded_at FROM conversation_messages WHERE message='m-earlier'",
                [],
                |r| r.get(0),
            )
            .map_err(instrukt_runner::state::sql(path))
        })
        .unwrap()
        .flatten();
    assert!(earlier_superseded.is_none());
}

#[test]
fn lead1_window_eviction_and_unobserved_grace_escalate_without_deadlocking() {
    let session: SessionRef = "t3:thread-evict".parse().unwrap();
    let root = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
    let home = instrukt_core::home::Home::at_user_home(root.path().to_path_buf()).for_writing();
    let mut db = StateDb::open(&home).unwrap();
    let t0: Timestamp = "2026-11-20T10:00:00Z".parse().unwrap();
    db.write::<_, instrukt_core::state_db::StateDbError>(|tx| {
        tx.execute(
            "INSERT INTO connected_conversations(company,connector,conversation,semantic_key,session,role,session_scope)
             VALUES ('acme','slack','C1','company:acme:slack:C1',?1,'company-agent','conversation')",
            [session.to_string()],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO conversation_messages(company,connector,conversation,message,sender,sender_name,written_us,written_at,arrived_at,attempted_at,admission_position,state,text,is_direct,dispatch_command_id)
             VALUES ('acme','slack','C1','m-evict','U1','Ada',1000,'2026-11-20T10:00:00Z','2026-11-20T10:00:00Z',?1,1,'sent','hi',1,'cmd-evict')",
            [t0.to_string()],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    drop(db);

    let state = RunnerState::new(&home);
    let c = Conversation {
        company: ProjectSlug::parse("acme").unwrap(),
        connector: "slack".into(),
        conversation: "C1".into(),
        semantic_key: "company:acme:slack:C1".into(),
        opening: None,
        opening_conversation: None,
        request: None,
        session: Some(session.clone()),
        role: RoleName::parse(COMPANY_AGENT_ROLE).unwrap(),
        session_scope: SessionScope::Conversation,
        needs_explicit_rollover: false,
    };

    // 10 newer turns in recent_turns all started after attempted_at -> immediate window-eviction escalation.
    let mut thread = stub_thread(&session, SessionState::InputReady);
    for i in 1..=10 {
        let ts = t0 + SignedDuration::from_secs(i * 5);
        thread.recent_turns.push(ObservedTurn {
            run_id: RunId::parse(&format!("run-newer-{i}")).unwrap(),
            input_id: Some(InputId::parse(&format!("other-{i}")).unwrap()),
            state: TurnState::Completed {
                started_at: ts,
                completed_at: ts + SignedDuration::from_secs(1),
            },
        });
    }
    let outcome = reconcile_thread(
        &state,
        &c,
        &thread,
        4000,
        t0 + SignedDuration::from_secs(60),
    )
    .unwrap();
    assert_eq!(
        outcome.terminal_errors,
        vec![("C1".into(), "m-evict".into())]
    );

    // Unobserved Sent message with empty recent_turns stays in flight before UNOBSERVED_SENT_GRACE
    // and escalates once the grace deadline passes while InputReady.
    state
        .write::<_, instrukt_runner::state::StateError>(|tx| {
            tx.execute(
                "INSERT INTO conversation_messages(company,connector,conversation,message,sender,sender_name,written_us,written_at,arrived_at,attempted_at,admission_position,state,text,is_direct,dispatch_command_id)
                 VALUES ('acme','slack','C1','m-grace','U1','Ada',2000,'2026-11-20T10:05:00Z','2026-11-20T10:05:00Z',?1,2,'sent','hi2',1,'cmd-grace')",
                [(t0 + SignedDuration::from_mins(5)).to_string()],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
    let empty_idle = stub_thread(&session, SessionState::InputReady);
    let before = reconcile_thread(
        &state,
        &c,
        &empty_idle,
        4000,
        t0 + SignedDuration::from_mins(5) + SignedDuration::from_secs(30),
    )
    .unwrap();
    assert!(before.terminal_errors.is_empty());

    let after = reconcile_thread(
        &state,
        &c,
        &empty_idle,
        4000,
        t0 + SignedDuration::from_mins(5) + UNOBSERVED_SENT_GRACE,
    )
    .unwrap();
    assert_eq!(after.terminal_errors, vec![("C1".into(), "m-grace".into())]);
}

#[test]
fn lead5_multi_reply_source_message_joins_and_replay_clear_rejected_send() {
    let session: SessionRef = "t3:thread-lead5".parse().unwrap();
    let root = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
    let home = instrukt_core::home::Home::at_user_home(root.path().to_path_buf()).for_writing();
    let t0: Timestamp = "2026-11-20T10:00:00Z".parse().unwrap();
    let mut db = StateDb::open(&home).unwrap();
    db.write::<_, instrukt_core::state_db::StateDbError>(|tx| {
        tx.execute(
            "INSERT INTO connected_conversations(company,connector,conversation,semantic_key,session,role,session_scope)
             VALUES ('acme','slack','C1','company:acme:slack:C1',?1,'company-agent','conversation')",
            [session.to_string()],
        )
        .unwrap();
        // Answered message m1 with two distinct reply_ids both pointing at source_message='m1':
        // reply:first is unposted while reply:second (m1.turn) is already posted.
        tx.execute(
            "INSERT INTO conversation_messages(company,connector,conversation,message,sender,sender_name,written_us,written_at,arrived_at,state,text,is_direct,turn)
             VALUES ('acme','slack','C1','m1','U1','Ada',1000,'2026-11-20T10:00:00Z','2026-11-20T10:00:00Z','answered','hi',1,'reply:second')",
            [],
        )
        .unwrap();
        tx.execute(
            "INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,admission_position,source_message,posted_at)
             VALUES ('acme','slack','C1','reply:first',0,'part 1',1,'m1',NULL),
                    ('acme','slack','C1','reply:second',0,'part 2',1,'m1','2026-11-20T10:01:00Z')",
            [],
        )
        .unwrap();
        // Pending message m2 in replay branch whose prior dispatch_write_at is older than SEND_RECONCILE_BOUND.
        tx.execute(
            "INSERT INTO conversation_messages(company,connector,conversation,message,sender,sender_name,written_us,written_at,arrived_at,attempted_at,dispatch_command_id,dispatch_write_at,boundary,content,state,text,is_direct)
             VALUES ('acme','slack','C1','m2','U1','Ada',2000,'2026-11-20T10:02:00Z','2026-11-20T10:02:00Z',?1,'cmd-replay',?1,'bnd-2','framed','pending','hi2',1)",
            [t0.to_string()],
        )
        .unwrap();
        Ok(())
    })
    .unwrap();
    drop(db);

    // Lead 5: unfinished() must still include m1 because reply:first (linked via source_message='m1') is unposted.
    let acme = ProjectSlug::parse("acme").unwrap();
    let pending = unfinished(&home, &acme, "slack").unwrap();
    assert!(pending.iter().any(|u| u.message == "m1"));

    // Lead 5: replay branch encountering CommandPreviouslyRejected calls clear_rejected_send and
    // clears in-memory attempted/dispatch_write_at so record_delivery_failure does NOT mark send_unknown.
    let state = RunnerState::new(&home);
    let c = Conversation {
        company: acme,
        connector: "slack".into(),
        conversation: "C1".into(),
        semantic_key: "company:acme:slack:C1".into(),
        opening: None,
        opening_conversation: None,
        request: None,
        session: Some(session.clone()),
        role: RoleName::parse(COMPANY_AGENT_ROLE).unwrap(),
        session_scope: SessionScope::Conversation,
        needs_explicit_rollover: false,
    };
    let mut msg = Message {
        conversation: "C1".into(),
        message: "m2".into(),
        stage: Stage::Pending,
        boundary: Some("bnd-2".into()),
        content: Some("framed".into()),
        attempted: Some(t0),
        failure: None,
        retry_at: None,
        failed_turns: 0,
        send_unknown: false,
        dispatch_id: Some("cmd-replay".into()),
        dispatch_write_at: Some(t0),
        observed_run_id: None,
        terminal_error_queued_at: None,
    };
    let late = t0 + SignedDuration::from_mins(10);
    let clock = || late;
    let delivery = NativeDelivery {
        home: &home,
        clock: &clock,
    };
    let adapter = StubAdapter::rejecting_send(stub_thread(&session, SessionState::InputReady));
    let err = send(&adapter, &state, &c, &session, &mut msg, &delivery);
    assert!(err.is_err());
    assert!(msg.dispatch_write_at.is_none());
    assert!(msg.attempted.is_none());
    record_delivery_failure(&adapter, &state, &c, &msg, &clock, &err).unwrap();

    let (send_unknown_at, dispatch_id): (Option<String>, Option<String>) = state
        .read(|conn, path| {
            conn.query_row(
                "SELECT send_unknown_at, dispatch_command_id FROM conversation_messages WHERE message='m2'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(instrukt_runner::state::sql(path))
        })
        .unwrap()
        .unwrap();
    assert!(send_unknown_at.is_none());
    assert!(dispatch_id.is_none());
}

#[test]
fn stored_states_read_back_as_their_variants() {
    for state in [
        ConnectionState::Connecting,
        ConnectionState::Connected,
        ConnectionState::Failed,
        ConnectionState::Stopped,
    ] {
        assert_eq!(ConnectionState::parse(state.as_str()), Some(state));
    }
    for stage in [
        Stage::Received,
        Stage::Refused,
        Stage::Pending,
        Stage::Sent,
        Stage::Failed,
        Stage::Answered,
    ] {
        assert_eq!(Stage::parse(stage.as_str()), Some(stage));
    }
}
