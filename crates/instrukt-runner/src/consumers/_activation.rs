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
            || !matches!(
                entry.event_delivery.as_deref(),
                Some("sequential" | "batched")
            )
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
                let result: Result<(), StateError> = state.write(|tx| {
                    let path = state::db_path(tx);
                    let declaration = serde_json::to_string(procedure).map_err(|error| state::invalid(&path, error))?;
                    tx.execute("INSERT INTO procedure_activations (event_id, procedure, exact_home, envelope, declaration, accepted_at, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'accepted') ON CONFLICT(event_id, procedure, exact_home) DO NOTHING", params![event.id, procedure.id.as_str(), exact_home, envelope, declaration, now.to_string()]).map_err(state::sql(&path))?;
                    Ok(())
                });
                match result {
                    Ok(()) => accepted += 1,
                    Err(error) => {
                        tracing::warn!(event = "activation_acceptance_failed", procedure = %procedure.id, home = %exact_home, error = %error);
                        failed = Some(failure(&error));
                    }
                }
            }
            match failed {
                Some(error) => Err(error),
                None => Ok(json!({"activations": accepted})),
            }
        }),
    })
}
