//! One generic matcher accepts durable responsibilities. Runtime delivery is separate.

use std::sync::Arc;

use instrukt_core::config::{Configuration, Module};
use instrukt_core::home::Home;
use instrukt_core::index::{GlobalIndex, IndexEntry, IndexScope};
use instrukt_core::publication::PublishedRecord;
use instrukt_core::state_db::rusqlite::params;
use instrukt_core::subscription::SubscriptionHome;
use serde_json::json;

use super::{Consumer, Interest, ReactionError};
use crate::events::{EventHome, TypePattern};
use crate::state::{self, RunnerState, StateError};
use crate::transport::EventScope;

pub(crate) const ACTIVATION_CONSUMER: &str = "procedure-activation";

fn failure(error: &dyn std::fmt::Display) -> ReactionError {
    ReactionError {
        stage: "activation-acceptance".into(),
        detail: error.to_string(),
    }
}

/// Reload per event rather than retaining a cache: an unreadable generation can
/// never leave a stale set in memory. Validate the whole active set before matching.
fn subscriptions(home: &Home) -> Result<Vec<(IndexEntry, Vec<TypePattern>)>, ReactionError> {
    let index = GlobalIndex::load(home).map_err(|error| {
        tracing::warn!(event = "subscriptions_cleared", error = %error, "publication index unreadable; active subscriptions cleared");
        failure(&error)
    })?;
    let index = match index {
        Some(index) => index,
        None if PublishedRecord::load_all(home)
            .map_err(|error| failure(&error))?
            .is_empty() =>
        {
            GlobalIndex::default()
        }
        None => {
            tracing::warn!(
                event = "subscriptions_cleared",
                "publication index absent; active subscriptions cleared"
            );
            return Err(failure(
                &"publication index absent; active subscriptions cleared",
            ));
        }
    };
    let mut selected = Vec::new();
    for entry in index
        .snippets
        .into_iter()
        .filter(|entry| !entry.subscriptions.is_empty() && entry.delivered_by.is_empty())
    {
        if entry.taxonomy != "procedure"
            || entry
                .role
                .as_deref()
                .is_none_or(|role| role.trim().is_empty())
            || entry.activation_trigger.is_some()
            || entry.subscription_home.is_none()
            || (entry.scope == IndexScope::Local
                && entry.subscription_home != Some(SubscriptionHome::Project))
        {
            tracing::warn!(event = "subscriptions_cleared", procedure = %entry.id, "invalid subscription index; active subscriptions cleared");
            return Err(failure(&format!(
                "invalid published subscription {}",
                entry.id
            )));
        }
        let patterns = entry.subscriptions.iter().map(|text| TypePattern::parse(text)).collect::<Result<Vec<_>, _>>().map_err(|error| {
            tracing::warn!(event = "subscriptions_cleared", error = %error, "invalid subscription index; active subscriptions cleared");
            failure(&error)
        })?;
        selected.push((entry, patterns));
    }
    Ok(selected)
}

fn exact_home(entry: &IndexEntry, origin: &EventHome) -> Option<String> {
    match (entry.subscription_home?, origin) {
        (SubscriptionHome::Platform, EventHome::None) => Some("platform".into()),
        (SubscriptionHome::Project, EventHome::Project(project))
            if entry.scope == IndexScope::Global || &entry.source_project == project =>
        {
            Some(format!("project:{project}"))
        }
        (SubscriptionHome::Company, EventHome::Company(company)) => {
            Some(format!("company:{company}"))
        }
        (SubscriptionHome::Person, EventHome::Person(person)) => Some(format!("person:{person}")),
        _ => None,
    }
}

pub(crate) fn consumer(
    home: &Home,
    state: &RunnerState,
    wakeup: std::sync::mpsc::SyncSender<()>,
) -> Result<Consumer, crate::events::TypePatternError> {
    let home = home.clone();
    let state = state.clone();
    Ok(Consumer {
        name: ACTIVATION_CONSUMER.into(),
        interest: Interest {
            patterns: vec![TypePattern::parse("*")?],
            own_computer_only: true,
            subject_kinds: Vec::new(),
            home_kinds: Vec::new(),
        },
        scopes: vec![EventScope::Local, EventScope::Cluster],
        reaction: Arc::new(move |event, now| {
            if event.home == EventHome::Unknown {
                return Ok(json!({"activations": 0}));
            }
            let subscriptions = subscriptions(&home)?;
            let root = if let EventHome::Project(project) = &event.home {
                PublishedRecord::load(&home, project)
                    .map_err(|error| failure(&error))?
                    .map(|record| record.checkout)
            } else {
                None
            };
            let config =
                Configuration::load(&home, root.as_deref()).map_err(|error| failure(&error))?;
            if !config
                .module_enabled(Module::Events)
                .map_err(|error| failure(&error))?
            {
                return Ok(json!({"activations": 0}));
            }
            let envelope = event.encode().map_err(|error| failure(&error))?;
            let mut accepted = 0;
            let mut changed = false;
            let mut failed = None;
            for (procedure, patterns) in &subscriptions {
                let Some(exact_home) = exact_home(procedure, &event.home) else {
                    continue;
                };
                if !patterns
                    .iter()
                    .any(|pattern| pattern.matches(&event.event_type))
                {
                    continue;
                }
                // Each keyed acceptance is independently durable. One failure must
                // not suppress another responsibility, and retries reuse prior keys.
                let result: Result<bool, StateError> = state.write(|tx| {
                    let path = state::db_path(tx);
                    let declaration = serde_json::to_string(procedure).map_err(|error| state::invalid(&path, error))?;
                    let catalog=crate::events::EventCatalog::builtin().map_err(|e|state::invalid(&path,e))?;
                    let kind=catalog.get(&event.event_type).map(|d|d.delivery_kind).unwrap_or_else(||if event.extensions.get("inbound_delivery").and_then(serde_json::Value::as_str)==Some("state"){instrukt_core::event_type::DeliveryKind::State}else{instrukt_core::event_type::DeliveryKind::Occurrence});
                    let inserted=tx.execute("INSERT INTO procedure_activations (event_id, procedure, exact_home, envelope, declaration, accepted_at, state,delivery_kind,subject,event_type) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'accepted',?7,?8,?9) ON CONFLICT(event_id, procedure, exact_home) DO NOTHING", params![event.id, procedure.id.as_str(), exact_home, envelope, declaration, now.to_string(),if kind==instrukt_core::event_type::DeliveryKind::State{"state"}else{"occurrence"},event.subject.to_string(),event.event_type]).map_err(state::sql(&path))?;
                    if kind==instrukt_core::event_type::DeliveryKind::State {supersede(tx,event,procedure.id.as_str(),&exact_home)?;}

                    Ok(inserted>0)
                });
                match result {
                    Ok(created) => {
                        accepted += 1;
                        changed |= created;
                    }
                    Err(error) => {
                        tracing::warn!(event = "activation_acceptance_failed", procedure = %procedure.id, home = %exact_home, error = %error);
                        failed = Some(failure(&error));
                    }
                }
            }
            if changed {
                match wakeup.try_send(()) {
                    Ok(()) | Err(std::sync::mpsc::TrySendError::Full(())) => {} // One wake already accounts for all durable pending input.
                    Err(std::sync::mpsc::TrySendError::Disconnected(())) => tracing::debug!(
                        event = "activation_coordinator_stopped",
                        "accepted activations remain durable for the next invocation"
                    ),
                }
            }
            match failed {
                Some(error) => Err(error),
                None => Ok(json!({"activations": accepted})),
            }
        }),
    })
}

/// Only unselected input is replaceable. Selected/accepted runtime messages are immutable.
fn supersede(
    tx: &instrukt_core::state_db::rusqlite::Transaction<'_>,
    event: &crate::events::EventEnvelope,
    procedure: &str,
    home: &str,
) -> Result<(), StateError> {
    let path = state::db_path(tx);
    let mut q=tx.prepare("SELECT a.event_id,a.envelope FROM procedure_activations a WHERE a.procedure=?1 AND a.exact_home=?2 AND a.subject=?3 AND a.event_type=?4 AND a.delivery_kind='state' AND a.state IN ('accepted','pending') AND NOT EXISTS (SELECT 1 FROM standing_deliveries d JOIN runtime_inputs i ON i.input_id=d.input_id WHERE d.event_id=a.event_id AND d.procedure=a.procedure AND d.exact_home=a.exact_home AND i.state IN ('selected','accepted','entered')) AND NOT EXISTS (SELECT 1 FROM standing_deliveries d WHERE d.event_id=a.event_id AND d.procedure=a.procedure AND d.exact_home=a.exact_home AND d.state='selected' AND d.target IS NULL) ORDER BY a.rowid").map_err(state::sql(&path))?;
    let rows = q
        .query_map(
            params![procedure, home, event.subject.to_string(), event.event_type],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
        )
        .map_err(state::sql(&path))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(state::sql(&path))?;
    let mut newest = None;
    for (id, envelope) in &rows {
        let parsed =
            crate::events::EventEnvelope::decode(envelope).map_err(|e| state::invalid(&path, e))?;
        if newest
            .as_ref()
            .is_none_or(|(_, time)| parsed.time.timestamp() >= *time)
        {
            newest = Some((id.clone(), parsed.time.timestamp()));
        }
    }
    for (id, _) in rows {
        if newest.as_ref().is_some_and(|(latest, _)| latest != &id) {
            tx.execute("UPDATE procedure_activations SET state='superseded',failure=?4 WHERE event_id=?1 AND procedure=?2 AND exact_home=?3",params![id,procedure,home,format!("superseded before delivery by {}",newest.as_ref().map(|r|r.0.as_str()).unwrap_or("unknown"))]).map_err(state::sql(&path))?;
            tx.execute("UPDATE runtime_inputs SET state='superseded' WHERE state='pending' AND input_id IN (SELECT input_id FROM standing_deliveries WHERE event_id=?1 AND procedure=?2 AND exact_home=?3)",params![id,procedure,home]).map_err(state::sql(&path))?;
            tx.execute("UPDATE standing_deliveries SET state='superseded' WHERE event_id=?1 AND procedure=?2 AND exact_home=?3",params![id,procedure,home]).map_err(state::sql(&path))?;
        }
    }
    Ok(())
}
