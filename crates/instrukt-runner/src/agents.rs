//! The dispatch agent order (`work-dispatch`, "Agent selection and intent
//! checks"; `layered-configuration`, "Agent preferences and
//! availability"): a task's own preference first, then the procedure's
//! `dispatch`, or else the global `agents.order` with
//! `agents.thinking_mode`.

use instrukt_core::config::{Configuration, ThinkingMode};
use instrukt_core::index::IndexDispatch;
use serde::{Deserialize, Serialize};

use crate::interfaces::{AgentPreference, Thinking};

/// One agent to try, with the thinking mode it starts in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentChoice {
    pub agent: String,
    pub thinking: Option<Thinking>,
}

impl Thinking {
    /// `slow`, `med` or `fast`.
    pub fn as_str(self) -> &'static str {
        match self {
            Thinking::Slow => "slow",
            Thinking::Med => "med",
            Thinking::Fast => "fast",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        [Thinking::Slow, Thinking::Med, Thinking::Fast]
            .into_iter()
            .find(|mode| mode.as_str() == text)
    }
}

impl From<ThinkingMode> for Thinking {
    fn from(mode: ThinkingMode) -> Self {
        match mode {
            ThinkingMode::Slow => Thinking::Slow,
            ThinkingMode::Med => Thinking::Med,
            ThinkingMode::Fast => Thinking::Fast,
        }
    }
}

/// The global `agents.order` and its thinking mode; a malformed value
/// (a validation finding) is logged and read as unset.
pub fn global_order(config: &Configuration) -> (Vec<&str>, Option<ThinkingMode>) {
    let order = config.agents_order().unwrap_or_else(|err| {
        tracing::warn!(event = "agents_order_unreadable", error = %err);
        Vec::new()
    });
    let thinking = config.agents_thinking_mode().unwrap_or_else(|err| {
        tracing::warn!(event = "agents_thinking_mode_unreadable", error = %err);
        None
    });
    (order, thinking)
}

/// The ordered agents to try; an agent appears once, at its first place.
pub fn agent_order(
    task: Option<&AgentPreference>,
    dispatch: Option<&IndexDispatch>,
    global_order: &[&str],
    global_thinking: Option<ThinkingMode>,
) -> Vec<AgentChoice> {
    let mut order: Vec<AgentChoice> = Vec::new();
    let mut push = |agent: &str, thinking: Option<Thinking>| {
        if !order.iter().any(|choice| choice.agent == agent) {
            order.push(AgentChoice {
                agent: agent.to_owned(),
                thinking,
            });
        }
    };
    if let Some(task) = task {
        push(&task.agent, task.thinking);
    }
    match dispatch {
        Some(dispatch) => {
            let thinking = Thinking::parse(&dispatch.thinking_mode);
            for agent in &dispatch.agent {
                push(agent, thinking);
            }
        }
        None => {
            for agent in global_order {
                push(agent, global_thinking.map(Thinking::from));
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(agent: &str, thinking: Thinking) -> AgentChoice {
        AgentChoice {
            agent: agent.into(),
            thinking: Some(thinking),
        }
    }

    #[test]
    fn the_task_preference_precedes_the_procedures_dispatch_which_replaces_the_global_order() {
        let task = AgentPreference {
            agent: "b".into(),
            thinking: Some(Thinking::Fast),
        };
        let dispatch = IndexDispatch {
            agent: vec!["a".into(), "b".into()],
            thinking_mode: "slow".into(),
        };
        assert_eq!(
            agent_order(
                Some(&task),
                Some(&dispatch),
                &["c"],
                Some(ThinkingMode::Med)
            ),
            [choice("b", Thinking::Fast), choice("a", Thinking::Slow)]
        );
        assert_eq!(
            agent_order(None, None, &["c", "d"], Some(ThinkingMode::Med)),
            [choice("c", Thinking::Med), choice("d", Thinking::Med)]
        );
        assert!(agent_order(None, None, &[], None).is_empty());
    }
}
