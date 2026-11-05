//! Verb-gated features (`session-continuity`, "Verb-gated features"): a
//! feature is available only when the runtime the developer chose fills
//! every verb it needs. No runtime is detected or chosen here; with none
//! chosen, every feature that needs one is unavailable.

use std::fmt;

use super::_runtime::{RuntimeEntry, VerbName};

/// A feature whose availability depends on the chosen runtime's verbs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    /// `modules.events: true`.
    StandingAgents,
    /// `modules.todos: true`.
    TodoDispatch,
    /// `modules.todos: true`.
    RecurringTasks,
    /// `runner.notify` unset: the runtime's own notification.
    AnomalyNotification,
    /// `instrukt sessions alive`.
    SessionStatus,
    /// `instrukt sessions link`.
    LinkedSessionInput,
    /// `instrukt sessions history search`.
    HistorySearch,
    /// `instrukt sessions history show`.
    HistoryShow,
}

impl Feature {
    /// The verbs the feature needs. Todo dispatch also needs worktree
    /// support from `start`, which the entry's text cannot show; a start
    /// that returns no worktree path reports that gap when it runs.
    pub fn verbs(self) -> &'static [VerbName] {
        use VerbName::*;
        match self {
            Feature::StandingAgents | Feature::TodoDispatch => &[Start, Send, Conversation, Alive],
            Feature::RecurringTasks => &[Start, Conversation, Alive],
            Feature::AnomalyNotification => &[Notify],
            Feature::SessionStatus => &[Alive],
            Feature::LinkedSessionInput => &[Link],
            Feature::HistorySearch => &[HistorySearch],
            Feature::HistoryShow => &[HistoryShow],
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Feature::StandingAgents => "standing agents",
            Feature::TodoDispatch => "todo dispatch",
            Feature::RecurringTasks => "recurring tasks",
            Feature::AnomalyNotification => "anomaly notification",
            Feature::SessionStatus => "session status",
            Feature::LinkedSessionInput => "linked-session input",
            Feature::HistorySearch => "history search",
            Feature::HistoryShow => "history show",
        }
    }

    /// The features a module switch turns on, for the switch's key under
    /// `modules`.
    pub(crate) fn switched_on_by(module: &str) -> &'static [Feature] {
        match module {
            "events" => &[Feature::StandingAgents],
            "todos" => &[Feature::TodoDispatch, Feature::RecurringTasks],
            _ => &[],
        }
    }
}

impl fmt::Display for Feature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Why features are unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    /// No runtime is chosen: neither `runner.runtime` nor a `runtimes`
    /// entry.
    NoRuntime,
    /// The chosen runtime leaves these verbs empty.
    MissingVerbs {
        runtime: String,
        verbs: Vec<VerbName>,
    },
}

impl Unavailable {
    /// The reason in words: what `features` need and why they lack it.
    pub fn describe(&self, features: &[Feature]) -> String {
        let names = features
            .iter()
            .map(|f| f.name())
            .collect::<Vec<_>>()
            .join(" and ");
        match self {
            Unavailable::NoRuntime => format!(
                "{names} need a runtime, and no runtime is chosen; choose one with `runner.runtime` or map one under `runtimes`"
            ),
            Unavailable::MissingVerbs { runtime, verbs } => {
                let verbs = verbs
                    .iter()
                    .map(|v| format!("`{v}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("{names} need {verbs}, which runtime `{runtime}` leaves empty")
            }
        }
    }
}

/// Whether `features` are available on `runtime` (the chosen runtime, if
/// any); `None` when they are.
pub fn availability(runtime: Option<&RuntimeEntry>, features: &[Feature]) -> Option<Unavailable> {
    let Some(entry) = runtime else {
        return Some(Unavailable::NoRuntime);
    };
    let mut missing: Vec<VerbName> = Vec::new();
    for verb in features.iter().flat_map(|f| f.verbs()) {
        if !entry.fills(*verb) && !missing.contains(verb) {
            missing.push(*verb);
        }
    }
    (!missing.is_empty()).then(|| Unavailable::MissingVerbs {
        runtime: entry.name.clone(),
        verbs: missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(text: &str) -> RuntimeEntry {
        RuntimeEntry::from_value(0, &serde_yaml_ng::from_str(text).unwrap()).unwrap()
    }

    #[test]
    fn a_runtime_filling_every_needed_verb_carries_the_feature() {
        let full =
            entry("{name: rt, session_id_env: R, start: s, send: s, alive: a, conversation: c}");
        assert_eq!(
            availability(
                Some(&full),
                &[Feature::TodoDispatch, Feature::RecurringTasks]
            ),
            None
        );
    }

    #[test]
    fn each_missing_verb_is_named_once_with_its_runtime() {
        let partial = entry("{name: rt, session_id_env: R, start: s}");
        assert_eq!(
            availability(
                Some(&partial),
                &[Feature::TodoDispatch, Feature::RecurringTasks]
            ),
            Some(Unavailable::MissingVerbs {
                runtime: "rt".into(),
                verbs: vec![VerbName::Send, VerbName::Conversation, VerbName::Alive],
            })
        );
    }

    #[test]
    fn without_a_chosen_runtime_nothing_is_available() {
        assert_eq!(
            availability(None, &[Feature::StandingAgents]),
            Some(Unavailable::NoRuntime)
        );
    }
}
