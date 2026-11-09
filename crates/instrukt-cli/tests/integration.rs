//! All CLI integration scenarios, linked once with shared boundary fixtures.
mod common;
mod fixtures;

#[path = "cases/agent-entrypoints.rs"]
mod agent_entrypoints;
#[path = "cases/agent-selection.rs"]
mod agent_selection;
#[path = "cases/anomaly-notification.rs"]
mod anomaly_notification;
#[path = "cases/approvals.rs"]
mod approvals;
#[path = "cases/attributed-commit.rs"]
mod attributed_commit;
#[path = "cases/binding-reservation.rs"]
mod binding_reservation;
#[path = "cases/blast-radius.rs"]
mod blast_radius;
#[path = "cases/configuration-editing.rs"]
mod configuration_editing;
#[path = "cases/conversation-links.rs"]
mod conversation_links;
#[path = "cases/delivery-cycle.rs"]
mod delivery_cycle;
#[path = "cases/dependencies-and-holds.rs"]
mod dependencies_and_holds;
#[path = "cases/docs-repositories.rs"]
mod docs_repositories;
#[path = "cases/editor-schemas.rs"]
mod editor_schemas;
#[path = "cases/eval-report.rs"]
mod eval_report;
#[path = "cases/event-activation.rs"]
mod event_activation;
#[path = "cases/event-consumers.rs"]
mod event_consumers;
#[path = "cases/event-discovery.rs"]
mod event_discovery;
#[path = "cases/event-transport.rs"]
mod event_transport;
#[path = "cases/github-apps.rs"]
mod github_apps;
#[path = "cases/github-record.rs"]
mod github_record;
#[path = "cases/github-webhook-normalization.rs"]
mod github_webhook_normalization;
#[path = "cases/graph-engine.rs"]
mod graph_engine;
#[path = "cases/graph-generation.rs"]
mod graph_generation;
#[path = "cases/graph-navigation.rs"]
mod graph_navigation;
#[path = "cases/harness.rs"]
mod harness;
#[path = "cases/history.rs"]
mod history;
#[path = "cases/hook-sections.rs"]
mod hook_sections;
#[path = "cases/inbound-webhook-endpoint-framework.rs"]
mod inbound_webhook_endpoint_framework;
#[path = "cases/inspection-status.rs"]
mod inspection_status;
#[path = "cases/integration-gate.rs"]
mod integration_gate;
#[path = "cases/interpolation.rs"]
mod interpolation;
#[path = "cases/known-issue-suppression.rs"]
mod known_issue_suppression;
#[path = "cases/layered-configuration.rs"]
mod layered_configuration;
#[path = "cases/logging.rs"]
mod logging;
#[path = "cases/module-installation.rs"]
mod module_installation;
#[path = "cases/owned-files.rs"]
mod owned_files;
#[path = "cases/pending-documentation.rs"]
mod pending_documentation;
#[path = "cases/preparation.rs"]
mod preparation;
#[path = "cases/procedure-event-delivery.rs"]
mod procedure_event_delivery;
#[path = "cases/procedure-subscription-routing.rs"]
mod procedure_subscription_routing;
#[path = "cases/progressive-retrieval.rs"]
mod progressive_retrieval;
#[path = "cases/project-initialization.rs"]
mod project_initialization;
#[path = "cases/project-publication.rs"]
mod project_publication;
#[path = "cases/release-installer.rs"]
mod release_installer;
#[path = "cases/roles-and-capabilities.rs"]
mod roles_and_capabilities;
#[path = "cases/session-continuity.rs"]
mod session_continuity;
#[path = "cases/single-ingress-webhook-termination.rs"]
mod single_ingress_webhook_termination;
#[path = "cases/snippet-schema.rs"]
mod snippet_schema;
#[path = "cases/snippet-validation.rs"]
mod snippet_validation;
#[path = "cases/state-database.rs"]
mod state_database;
#[path = "cases/task-timing.rs"]
mod task_timing;
#[path = "cases/todos.rs"]
mod todos;
#[path = "cases/use-case-coverage.rs"]
mod use_case_coverage;
#[path = "cases/use-case-trace.rs"]
mod use_case_trace;
#[path = "cases/work-dispatch.rs"]
mod work_dispatch;
#[path = "cases/workflow-definition.rs"]
mod workflow_definition;
