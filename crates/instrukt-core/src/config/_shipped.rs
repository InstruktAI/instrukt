//! The runtime entries instrukt ships (`layered-configuration`, "Runtime
//! choice and shipped entries"). A shipped entry is configuration only: it
//! joins the effective mapping when the developer chooses it with
//! `runner.runtime`, and is then driven through the same verbs as a
//! declared entry. Nothing selects one for the developer.

use serde_yaml_ng::Value;

/// The shipped entries, by name, as YAML documents.
const SHIPPED: [(&str, &str); 1] = [("t3", include_str!("shipped/t3.yaml"))];

/// The names of the shipped runtime entries.
pub fn names() -> impl Iterator<Item = &'static str> {
    SHIPPED.iter().map(|(name, _)| *name)
}

/// The shipped entry named `name`, as stored. The text is part of the
/// binary and its tests read it; should it ever not parse, the entry is
/// reported and treated as not shipped, so choosing it is a finding.
pub(crate) fn entry(name: &str) -> Option<Value> {
    let (_, text) = SHIPPED.iter().find(|(shipped, _)| *shipped == name)?;
    match serde_yaml_ng::from_str(text) {
        Ok(value) => Some(value),
        Err(err) => {
            tracing::error!(event = "shipped_runtime_unreadable", runtime = name, error = %err);
            None
        }
    }
}

/// `value` with the chosen shipped entry joined to its `runtimes`, after
/// the declared entries, unless a declared entry carries its name.
pub(crate) fn join_chosen(value: &mut Value) {
    let Some(chosen) = value
        .get("runner")
        .and_then(|runner| runner.get("runtime"))
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    let Some(shipped) = entry(&chosen) else {
        return;
    };
    let Some(map) = value.as_mapping_mut() else {
        return;
    };
    let runtimes = map
        .entry(Value::String("runtimes".to_owned()))
        .or_insert_with(|| Value::Sequence(Vec::new()));
    let Some(seq) = runtimes.as_sequence_mut() else {
        return;
    };
    let declared = seq
        .iter()
        .any(|entry| entry.get("name").and_then(Value::as_str) == Some(chosen.as_str()));
    if !declared {
        seq.push(shipped);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::_runtime::{RuntimeEntry, VerbKind, VerbName};

    fn yaml(text: &str) -> Value {
        serde_yaml_ng::from_str(text).unwrap()
    }

    #[test]
    fn every_shipped_entry_reads_as_a_runtime_entry_of_its_own_name() {
        for name in names() {
            let entry = RuntimeEntry::from_value(0, &entry(name).unwrap()).unwrap();
            assert_eq!(entry.name, name);
        }
    }

    #[test]
    fn the_t3_entry_fills_its_verbs_with_the_kinds_the_spec_names() {
        let entry = RuntimeEntry::from_value(0, &entry("t3").unwrap()).unwrap();
        let kind = |verb| entry.verb(verb).map(|v| v.kind);
        for verb in [
            VerbName::Start,
            VerbName::Send,
            VerbName::Alive,
            VerbName::Conversation,
            VerbName::HistoryShow,
        ] {
            assert_eq!(kind(verb), Some(VerbKind::Command), "{verb}");
        }
        for verb in [VerbName::End, VerbName::EndSelf, VerbName::HistorySearch] {
            assert_eq!(kind(verb), Some(VerbKind::Agent), "{verb}");
        }
        for verb in [
            VerbName::Notify,
            VerbName::Link,
            VerbName::Available,
            VerbName::Model,
        ] {
            assert_eq!(kind(verb), None, "{verb}");
        }
    }

    #[test]
    fn a_chosen_shipped_entry_joins_after_the_declared_ones_unless_one_takes_its_name() {
        let mut chosen = yaml("runner: {runtime: t3}\nruntimes: [{name: a, session_id_env: A}]\n");
        join_chosen(&mut chosen);
        let names: Vec<&str> = chosen["runtimes"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["a", "t3"]);

        let declared =
            yaml("runner: {runtime: t3}\nruntimes: [{name: t3, session_id_env: MINE}]\n");
        let mut joined = declared.clone();
        join_chosen(&mut joined);
        assert_eq!(joined, declared);

        let unchosen = yaml("runtimes: [{name: a, session_id_env: A}]\n");
        let mut untouched = unchosen.clone();
        join_chosen(&mut untouched);
        assert_eq!(untouched, unchosen);
    }
}
