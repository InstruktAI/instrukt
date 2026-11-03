//! The runtime mapping and the active-runtime selection rule.

use std::collections::BTreeMap;

use serde_yaml_ng::Value;

/// The fields a runtime-mapping entry may carry.
pub(crate) const REQUIRED_FIELDS: [&str; 6] = [
    "name",
    "session_id_env",
    "start",
    "send",
    "alive",
    "conversation",
];
pub(crate) const OPTIONAL_FIELDS: [&str; 5] = ["notify", "link", "end", "available", "agent"];

/// The derived field `runtime.session_id`.
pub(crate) const SESSION_ID: &str = "session_id";

/// A snapshot of the caller's environment variables, against which
/// `${VAR}`, `runtime.session_id` and the active runtime resolve.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Env {
    vars: BTreeMap<String, String>,
}

impl Env {
    /// The environment of the running process. Variables whose name or
    /// value is not UTF-8 are left out.
    pub fn from_process() -> Self {
        Self {
            vars: std::env::vars_os()
                .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
                .collect(),
        }
    }

    /// An explicit environment.
    pub fn from_pairs<I, K, V>(pairs: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        Self {
            vars: pairs
                .into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(String::as_str)
    }
}

/// One entry of the runtime mapping: how a runtime identifies its sessions
/// and the command templates it offers. Templates are unexpanded; expand
/// them with [`super::Configuration::expand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEntry {
    /// Position in the mapping (`runtimes.<index>`).
    pub index: usize,
    pub name: String,
    pub session_id_env: String,
    /// Set when the entry is an agent CLI: the agent's name.
    pub agent: Option<String>,
    fields: BTreeMap<String, String>,
}

impl RuntimeEntry {
    /// The unexpanded template (or other string field) named `field`:
    /// `start`, `send`, `alive`, `conversation`, `notify`, `link`, `end`,
    /// `available`.
    pub fn field(&self, field: &str) -> Option<&str> {
        self.fields.get(field).map(String::as_str)
    }

    /// Reads entry `index` of the mapping; `None` when it lacks a `name` or
    /// `session_id_env` (a validation finding).
    pub(crate) fn from_value(index: usize, value: &Value) -> Option<Self> {
        let map = value.as_mapping()?;
        let fields: BTreeMap<String, String> = map
            .iter()
            .filter_map(|(k, v)| Some((k.as_str()?.to_owned(), v.as_str()?.to_owned())))
            .collect();
        Some(Self {
            index,
            name: fields.get("name")?.clone(),
            session_id_env: fields.get("session_id_env")?.clone(),
            agent: fields.get("agent").cloned(),
            fields,
        })
    }
}

/// Every well-formed entry of a `runtimes` value, in order.
pub(crate) fn entries(runtimes: Option<&Value>) -> Vec<RuntimeEntry> {
    runtimes
        .and_then(Value::as_sequence)
        .map(|seq| {
            seq.iter()
                .enumerate()
                .filter_map(|(i, v)| RuntimeEntry::from_value(i, v))
                .collect()
        })
        .unwrap_or_default()
}

/// The active runtime: the first entry whose `session_id_env` is set in
/// `env`.
pub(crate) fn active<'a>(entries: &'a [RuntimeEntry], env: &Env) -> Option<&'a RuntimeEntry> {
    entries
        .iter()
        .find(|entry| env.get(&entry.session_id_env).is_some())
}

/// The active agent: the first agent-CLI entry whose `session_id_env` is
/// set in `env` — the same rule applied to agent-CLI entries.
pub(crate) fn active_agent<'a>(entries: &'a [RuntimeEntry], env: &Env) -> Option<&'a RuntimeEntry> {
    entries
        .iter()
        .filter(|entry| entry.agent.is_some())
        .find(|entry| env.get(&entry.session_id_env).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping() -> Vec<RuntimeEntry> {
        let value: Value = serde_yaml_ng::from_str(
            "- {name: host, session_id_env: HOST_ID, conversation: c}\n\
             - {name: cli, session_id_env: CLI_ID, agent: cli, conversation: c}\n\
             - {name: other, session_id_env: OTHER_ID, agent: other}\n",
        )
        .unwrap();
        entries(Some(&value))
    }

    #[test]
    fn the_first_entry_whose_variable_is_set_is_active() {
        let entries = mapping();
        let both = Env::from_pairs([("HOST_ID", "h"), ("CLI_ID", "c")]);
        assert_eq!(active(&entries, &both).unwrap().index, 0);
        let cli = Env::from_pairs([("CLI_ID", "c"), ("OTHER_ID", "o")]);
        assert_eq!(active(&entries, &cli).unwrap().index, 1);
        assert!(active(&entries, &Env::default()).is_none());
    }

    #[test]
    fn the_active_agent_skips_entries_that_are_not_agent_clis() {
        let entries = mapping();
        let both = Env::from_pairs([("HOST_ID", "h"), ("OTHER_ID", "o")]);
        assert_eq!(active_agent(&entries, &both).unwrap().index, 2);
        assert!(active_agent(&entries, &Env::from_pairs([("HOST_ID", "h")])).is_none());
    }
}
