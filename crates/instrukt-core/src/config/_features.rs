//! Runtime capability requirements at module admission.
use super::Runtime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    StandingAgents,
    HistoryExtraction,
    TodoDispatch,
    RecurringTasks,
    AnomalyNotification,
    SessionStatus,
    LinkedSessionInput,
    WorktreeAttachment,
    /// Conversations through company connectors: a company home's declared
    /// connector gives them.
    Conversations,
}
impl Feature {
    pub fn name(self) -> &'static str {
        match self {
            Self::StandingAgents => "standing agents",
            Self::HistoryExtraction => "history extraction",
            Self::TodoDispatch => "todo dispatch",
            Self::RecurringTasks => "recurring tasks",
            Self::AnomalyNotification => "anomaly notification",
            Self::SessionStatus => "session status",
            Self::LinkedSessionInput => "linked-session input",
            Self::WorktreeAttachment => "worktree attachment",
            Self::Conversations => "conversations",
        }
    }
    pub(crate) fn switched_on_by(module: &str) -> &'static [Self] {
        match module {
            "events" => &[Self::StandingAgents],
            "todos" => &[Self::TodoDispatch, Self::RecurringTasks],
            _ => &[],
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unavailable {
    MissingCapabilities {
        runtime: String,
        capabilities: Vec<&'static str>,
    },
}
impl Unavailable {
    pub fn describe(&self, features: &[Feature]) -> String {
        let names = features
            .iter()
            .map(|f| f.name())
            .collect::<Vec<_>>()
            .join(" and ");
        match self {
            Self::MissingCapabilities {
                runtime,
                capabilities,
            } => format!(
                "runtime `{runtime}` lacks {} required for {names}",
                capabilities.join(", ")
            ),
        }
    }
}
pub fn availability(runtime: Runtime, features: &[Feature]) -> Option<Unavailable> {
    if features.is_empty() {
        return None;
    }
    let mut capabilities = Vec::new();
    for feature in features {
        let missing = match feature {
            Feature::StandingAgents => None,
            Feature::AnomalyNotification => Some("notification"),
            Feature::LinkedSessionInput => Some("link"),
            Feature::WorktreeAttachment => Some("worktree attachment"),
            _ => None,
        };
        if let Some(missing) = missing
            && !capabilities.contains(&missing)
        {
            capabilities.push(missing);
        }
    }
    (!capabilities.is_empty()).then(|| Unavailable::MissingCapabilities {
        runtime: runtime.name().into(),
        capabilities,
    })
}
