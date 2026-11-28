use super::_delivery::{opening_procedure, opening_procedure_for_role};
use super::_store::Stage;
use super::{ConnectionState, ConversationError, effective_since, split};
use instrukt_core::config::SHIPPED_AGENTS;
use instrukt_core::config::{Agents, COMPANY_AGENT_ROLE};
use instrukt_core::ids::RoleName;

#[test]
fn a_messages_interval_is_its_own_or_the_relays_failure_from_its_admission() {
    let at = |text: &str| Some(text.parse::<jiff::Timestamp>().unwrap());
    // Nothing open: no interval.
    assert_eq!(
        effective_since(None, at("2026-11-19T10:00:00Z"), None),
        None
    );
    // Its own interval alone.
    assert_eq!(
        effective_since(at("2026-11-19T10:05:00Z"), at("2026-11-19T10:00:00Z"), None),
        at("2026-11-19T10:05:00Z")
    );
    // A relay failure that began before the message was admitted counts
    // from its admission.
    assert_eq!(
        effective_since(None, at("2026-11-19T10:00:00Z"), at("2026-11-19T09:00:00Z")),
        at("2026-11-19T10:00:00Z")
    );
    // One that began after it counts from its start.
    assert_eq!(
        effective_since(None, at("2026-11-19T10:00:00Z"), at("2026-11-19T10:02:00Z")),
        at("2026-11-19T10:02:00Z")
    );
    // Both open: the earlier one.
    assert_eq!(
        effective_since(
            at("2026-11-19T10:01:00Z"),
            at("2026-11-19T10:00:00Z"),
            at("2026-11-19T10:02:00Z")
        ),
        at("2026-11-19T10:01:00Z")
    );
    // A message admitted before its admission was recorded counts from
    // the relay failure's start.
    assert_eq!(
        effective_since(None, None, at("2026-11-19T09:00:00Z")),
        at("2026-11-19T09:00:00Z")
    );
}

/// An index holding the procedure declaring the conversation role,
/// pending its delivering todo when `pending`.
fn declared(pending: bool) -> instrukt_core::index::GlobalIndex {
    let delivered_by = if pending {
        "  delivered_by: [draft-it]\n"
    } else {
        ""
    };
    serde_yaml_ng::from_str(&format!(
                "schema_version: 3\nsnippets:\n- id: general/procedure/company-assistance\n  description: Assist.\n  type: procedure\n  scope: global\n  domain: general\n  path: general/procedure/company-assistance.md\n  source_project: demo\n  role: company-agent\n{delivered_by}"
            ),
        )
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
    let project = instrukt_core::ids::ProjectSlug::parse("instrukt").unwrap();
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

/// The character count of each part, and whether the parts together
/// are the text unchanged.
fn cut(text: &str, limit: usize) -> (Vec<usize>, bool) {
    let parts = split(text, limit);
    let counts = parts.iter().map(|part| part.chars().count()).collect();
    (counts, parts.concat() == text)
}

#[test]
fn a_part_ends_after_the_last_line_break_in_its_second_half() {
    // Line breaks at characters 23 and 29 both lie in the second half.
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

/// A home whose runner bound `session` to Acme's Slack conversation,
/// with the fixed native thread, before the native start: the
/// conversation records no session and no generation yet.
fn bound_before_start(
    session: &instrukt_core::ids::SessionRef,
) -> (tempfile::TempDir, instrukt_core::home::Home) {
    let root = instrukt_core::fsx::temp_dir(&tempfile::Builder::new()).unwrap();
    let home = instrukt_core::home::Home::at_user_home(root.path().to_path_buf()).for_writing();
    let mut db = instrukt_core::state_db::StateDb::open(&home).unwrap();
    db.write::<_, instrukt_core::state_db::StateDbError>(|tx| {
            tx.execute(
                "INSERT INTO connected_conversations(company,connector,conversation,semantic_key,role,session_scope,reply_mode) VALUES ('acme','slack','C1','company:acme:slack:C1','company-agent','conversation','thread')",
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
    let session: instrukt_core::ids::SessionRef = "t3:thread-before-start".parse().unwrap();
    let (_root, home) = bound_before_start(&session);
    let acme = instrukt_core::ids::ProjectSlug::parse("acme").unwrap();
    let assignment = super::conversation_assignment(&home, &session)
        .unwrap()
        .expect("the bound session is the conversation's");
    assert_eq!(assignment.company, Some(acme.clone()));
    assert_eq!(assignment.role.as_str(), COMPANY_AGENT_ROLE);
    // Until the runner records it as the conversation's, the session has
    // no speaker to act for: it is refused, never served as a session
    // nobody is asking in.
    let adapter = instrukt_runner::runtime::adapter(instrukt_core::config::Runtime::T3);
    assert!(matches!(
        super::speaker(&home, &session, adapter.as_ref()),
        Err(instrukt_runner::mcp::McpError::Unbound { company }) if company == acme.as_str()
    ));
}

#[test]
fn a_session_no_binding_names_is_no_conversations() {
    let (_root, home) = bound_before_start(&"t3:thread-before-start".parse().unwrap());
    let other: instrukt_core::ids::SessionRef = "t3:someone-else".parse().unwrap();
    assert!(
        super::conversation_assignment(&home, &other)
            .unwrap()
            .is_none()
    );
    let adapter = instrukt_runner::runtime::adapter(instrukt_core::config::Runtime::T3);
    assert!(matches!(
        super::speaker(&home, &other, adapter.as_ref()),
        Ok(None)
    ));
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
