//! Resolved subscription facts shared by docs, publication and runner.
use serde::{Deserialize, Serialize};

use crate::event_type::TypePattern;
use crate::index::{IndexEntry, IndexScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionHome {
    Project,
    Platform,
    Company,
    Person,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionIdentity {
    #[default]
    Home,
    Subject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SubscriptionAttendance {
    #[default]
    Attended,
    Unattended,
}

/// Whether a published index entry is an active subscriber: its type
/// patterns when it is, why it is dropped when it is not. Only an entry that
/// declares subscriptions and is not pending is asked.
pub fn subscriber_patterns(entry: &IndexEntry) -> Result<Vec<TypePattern>, String> {
    if entry.taxonomy != "procedure" {
        return Err("only a procedure may declare subscriptions".to_owned());
    }
    if entry
        .role
        .as_deref()
        .is_none_or(|role| role.trim().is_empty())
    {
        return Err("a subscribing procedure must declare a role".to_owned());
    }
    if entry.activation_trigger.is_some() {
        return Err("a subscribing procedure must not declare an activation trigger".to_owned());
    }
    if entry.subscription_home.is_none() {
        return Err("a subscribing procedure must declare its subscription home".to_owned());
    }
    if entry.scope == IndexScope::Local
        && entry.subscription_home != Some(SubscriptionHome::Project)
    {
        return Err("a project's own procedure must subscribe in its project".to_owned());
    }
    entry
        .subscriptions
        .iter()
        .map(|text| TypePattern::parse(text))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())
}
