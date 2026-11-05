//! The existing published procedure matcher accepts keyed activations. Runtime
//! delivery remains a later boundary; the retained reader itself starts no session.

use std::sync::Arc;

use instrukt_core::config::{Configuration, Module};
use instrukt_core::home::Home;
use instrukt_core::index::GlobalIndex;
use instrukt_core::publication::PublishedRecord;
use instrukt_core::state_db::rusqlite::params;
use serde_json::json;

use super::{Consumer, Interest, ReactionError};
use crate::events::{EventHome, TypePattern};
use crate::state::{self, RunnerState, StateError};
use crate::transport::EventScope;

pub(crate) const ACTIVATION_CONSUMER: &str = "procedure-activation";

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
            let fail = |error: &dyn std::fmt::Display| ReactionError {
                stage: "activation-acceptance".into(),
                detail: error.to_string(),
            };
            // Every attempt obtains the current publication. An unreadable index
            // cannot leave a stale match set active.
            let index = GlobalIndex::load(&home)
                .map_err(|e| fail(&e))?
                .unwrap_or_default();
            let EventHome::Project(project) = &event.home else {
                return Ok(json!({"activations": 0}));
            };
            let published = PublishedRecord::load_all(&home).map_err(|e| fail(&e))?;
            let Some(record) = published.iter().find(|record| &record.project == project) else {
                return Ok(json!({"activations": 0}));
            };
            let config =
                Configuration::load(&home, Some(&record.checkout)).map_err(|e| fail(&e))?;
            if !config
                .module_enabled(Module::Events)
                .map_err(|e| fail(&e))?
            {
                return Ok(json!({"activations": 0}));
            }
            let mut selected = Vec::new();
            for procedure in crate::standing::subscribers(&index, project) {
                let patterns = procedure
                    .subscriptions
                    .iter()
                    .map(|text| TypePattern::parse(text))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| fail(&e))?;
                if patterns
                    .iter()
                    .any(|pattern| pattern.matches(&event.event_type))
                {
                    selected.push(procedure);
                }
            }
            let envelope = event.encode().map_err(|e| fail(&e))?;
            let exact_home = String::from(event.home.clone());
            let accepted: Result<usize, StateError> = state.write(|tx| {
                let path = state::db_path(tx);
                for procedure in &selected {
                    let declaration = serde_json::to_string(procedure).map_err(|e| state::invalid(&path, e))?;
                    tx.execute("INSERT OR IGNORE INTO procedure_activations (event_id, procedure, exact_home, envelope, declaration, accepted_at, state) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'accepted')", params![event.id, procedure.id.as_str(), exact_home, envelope, declaration, now.to_string()]).map_err(state::sql(&path))?;
                }
                Ok(selected.len())
            });
            Ok(json!({"activations": accepted.map_err(|e| fail(&e))?}))
        }),
    })
}
