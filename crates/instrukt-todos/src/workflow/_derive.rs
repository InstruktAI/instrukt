//! Evaluation: a definition and facts to the derived state, the selected
//! transition, its action with rendered instructions, and the effects owed
//! (`docs/local/spec/feature/workflow/workflow-definition.md`).

use std::collections::BTreeSet;
use std::sync::Arc;

use instrukt_core::ids::{ProjectSlug, TodoSlug};

use super::_facts::Facts;
use super::_flavor::{Effect, Flavor, Transition};
use super::_names::{ActionId, EffectName, FlowName, StateName};
use super::_render::{Bindings, ConfigLookup, Rendering, render};

/// The todo an evaluation is for, and the facts the provider supplied for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subject {
    pub project: ProjectSlug,
    pub slug: TodoSlug,
    pub facts: Facts,
}

/// The result of one evaluation. Nothing is stored; the next call derives
/// again from the facts it is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Derivation {
    pub flow: FlowName,
    /// The state where derivation stopped.
    pub state: StateName,
    /// The obligation at that state; `None` when no transition was selected.
    pub obligation: Option<Obligation>,
}

/// What the last selected transition makes owed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obligation {
    pub transition: SelectedTransition,
    pub action: ActionId,
    /// The action's command with placeholders filled; `{brief}` is the
    /// rendered template.
    pub command: Rendering,
    /// The action's rendered template: the instructions.
    pub brief: Rendering,
    /// Effects still owed along the selected walk, in listed order.
    pub effects: Vec<OwedEffect>,
}

/// A transition the walk selected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedTransition {
    /// Its position in the definition's `transitions`, from zero.
    pub index: usize,
    pub from: StateName,
    pub to: StateName,
}

/// An effect whose `done_when` is absent or does not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwedEffect {
    pub name: EffectName,
    pub command: Rendering,
}

/// An evaluation could not produce a derivation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EvaluationError {
    #[error("the flavor defines no flow `{0}`")]
    UnknownFlow(String),
    /// The walk returned to a state it had visited; not an executable
    /// obligation.
    #[error("flow `{flow}` revisits state `{state}` (walk: {})", walk_text(walk))]
    Revisit {
        flow: FlowName,
        state: StateName,
        walk: Vec<StateName>,
    },
}

fn walk_text(walk: &[StateName]) -> String {
    walk.iter()
        .map(StateName::as_str)
        .collect::<Vec<_>>()
        .join(" → ")
}

impl Flavor {
    /// Evaluates `flow` for `subject`: starting at the initial state, takes
    /// the first transition whose condition holds, in definition order, until
    /// none does. Pure: reads only its arguments, plus `config` for
    /// placeholders that name configuration paths.
    pub fn evaluate(
        &self,
        flow: &str,
        subject: &Subject,
        config: &dyn ConfigLookup,
    ) -> Result<Derivation, EvaluationError> {
        let (flow, definition) = self
            .flows
            .get_key_value(flow)
            .ok_or_else(|| EvaluationError::UnknownFlow(flow.to_owned()))?;
        let facts = &subject.facts;
        let mut state = &definition.initial;
        let mut walk = vec![state.clone()];
        let mut visited = BTreeSet::from([state]);
        let mut selected: Option<(usize, &Transition)> = None;
        let mut owed: Vec<&(EffectName, Arc<Effect>)> = Vec::new();
        while let Some((index, transition)) = definition
            .transitions
            .iter()
            .enumerate()
            .find(|(_, t)| &t.from == state && t.when.holds(facts))
        {
            selected = Some((index, transition));
            owed.extend(transition.effects.iter().filter(|(_, effect)| {
                effect
                    .done_when
                    .as_ref()
                    .is_none_or(|done| !done.holds(facts))
            }));
            state = &transition.to;
            walk.push(state.clone());
            if !visited.insert(state) {
                return Err(EvaluationError::Revisit {
                    flow: flow.clone(),
                    state: state.clone(),
                    walk,
                });
            }
        }
        let obligation = selected
            .map(|(index, transition)| obligation(index, transition, &owed, subject, config));
        Ok(Derivation {
            flow: flow.clone(),
            state: state.clone(),
            obligation,
        })
    }
}

fn obligation(
    index: usize,
    transition: &Transition,
    owed: &[&(EffectName, Arc<Effect>)],
    subject: &Subject,
    config: &dyn ConfigLookup,
) -> Obligation {
    let mut bindings = Bindings {
        slug: &subject.slug,
        project: &subject.project,
        facts: &subject.facts,
        brief: None,
        config,
    };
    let effects = owed
        .iter()
        .map(|(name, effect)| OwedEffect {
            name: name.clone(),
            command: render(&effect.command, &bindings),
        })
        .collect();
    let (action_id, action) = &transition.action;
    let brief = render(&action.template, &bindings);
    bindings.brief = Some(&brief);
    let command = render(&action.command, &bindings);
    Obligation {
        transition: SelectedTransition {
            index,
            from: transition.from.clone(),
            to: transition.to.clone(),
        },
        action: action_id.clone(),
        command,
        brief,
        effects,
    }
}

#[cfg(test)]
mod tests {
    use super::super::_facts::{FactValue, Facts};
    use super::super::_render::{RenderFailure, RenderFinding};
    use super::super::_testing::{facts, flavor_dir, no_config, subject, text};
    use super::*;

    const ACTIONS: &str = "\
roles: [builder]
first:
  command: run first {slug} --brief '{brief}'
  template: templates/first.md
second:
  command: run second
  template: templates/second.md
effects:
  always:
    command: refresh {slug}
  until-built:
    command: build {project}
    done_when: {equals: [build.complete, true]}
";

    fn fixture(flow: &str) -> Flavor {
        let dir = flavor_dir(&[
            ("actions.yaml", ACTIONS),
            ("templates/first.md", "First for {slug} of kind {kind}."),
            ("templates/second.md", "Second."),
            ("work.yaml", flow),
        ]);
        Flavor::load(dir.path()).unwrap_or_else(|err| panic!("{err}"))
    }

    /// start -a-> middle -b-> end, with an earlier start transition that
    /// only holds for bugs.
    const WALK: &str = "\
flow: work
orchestrator: builder
initial: start
states: [start, middle, end, side]
transitions:
- from: start
  to: side
  when: {equals: [kind, bug]}
  action: second
- from: start
  to: middle
  when: {exists: prd.digest}
  action: first
  effects: [always, until-built]
- from: middle
  to: end
  when: {exists: architecture.digest}
  action: second
- from: start
  to: end
  when: {exists: prd.digest}
  action: second
";

    fn names(effects: &[OwedEffect]) -> Vec<&str> {
        effects.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn no_selected_transition_leaves_the_initial_state_with_no_obligation() {
        let derived = fixture(WALK)
            .evaluate("work", &subject(Facts::new()), &no_config)
            .unwrap();
        assert_eq!(derived.state.as_str(), "start");
        assert_eq!(derived.obligation, None);
    }

    #[test]
    fn the_first_holding_transition_in_definition_order_is_selected() {
        let flavor = fixture(WALK);
        let both = facts(&[("kind", text("bug")), ("prd.digest", text("p"))]);
        let derived = flavor.evaluate("work", &subject(both), &no_config).unwrap();
        assert_eq!(derived.state.as_str(), "side");
        let derived = flavor
            .evaluate(
                "work",
                &subject(facts(&[
                    ("kind", text("feature")),
                    ("prd.digest", text("p")),
                ])),
                &no_config,
            )
            .unwrap();
        assert_eq!(derived.state.as_str(), "middle");
        let obligation = derived.obligation.unwrap();
        assert_eq!(obligation.transition.index, 1);
        assert_eq!(obligation.action.as_str(), "first");
    }

    #[test]
    fn the_walk_continues_and_the_last_transition_names_the_action() {
        let derived = fixture(WALK)
            .evaluate(
                "work",
                &subject(facts(&[
                    ("prd.digest", text("p")),
                    ("architecture.digest", text("a")),
                ])),
                &no_config,
            )
            .unwrap();
        assert_eq!(derived.state.as_str(), "end");
        let obligation = derived.obligation.unwrap();
        assert_eq!(obligation.action.as_str(), "second");
        assert_eq!(
            (
                obligation.transition.from.as_str(),
                obligation.transition.to.as_str()
            ),
            ("middle", "end")
        );
        // Traversing `middle` does not discard the effects owed on the way.
        assert_eq!(names(&obligation.effects), ["always", "until-built"]);
    }

    #[test]
    fn an_effect_whose_done_when_holds_is_not_owed() {
        let derived = fixture(WALK)
            .evaluate(
                "work",
                &subject(facts(&[
                    ("prd.digest", text("p")),
                    ("build.complete", FactValue::Bool(true)),
                ])),
                &no_config,
            )
            .unwrap();
        let obligation = derived.obligation.unwrap();
        assert_eq!(names(&obligation.effects), ["always"]);
        assert_eq!(
            obligation.effects[0].command,
            Ok("refresh add-login".to_owned())
        );
    }

    #[test]
    fn the_command_receives_the_rendered_brief() {
        let derived = fixture(WALK)
            .evaluate(
                "work",
                &subject(facts(&[
                    ("kind", text("feature")),
                    ("prd.digest", text("p")),
                ])),
                &no_config,
            )
            .unwrap();
        let obligation = derived.obligation.unwrap();
        assert_eq!(
            obligation.brief,
            Ok("First for add-login of kind feature.".to_owned())
        );
        assert_eq!(
            obligation.command,
            Ok("run first add-login --brief 'First for add-login of kind feature.'".to_owned())
        );
    }

    #[test]
    fn an_unrenderable_brief_fails_the_command_that_uses_it() {
        // `kind` is absent: the template cannot render, nor can the command.
        let derived = fixture(WALK)
            .evaluate(
                "work",
                &subject(facts(&[("prd.digest", text("p"))])),
                &no_config,
            )
            .unwrap();
        let obligation = derived.obligation.unwrap();
        assert_eq!(
            obligation.brief,
            Err(RenderFailure {
                findings: vec![RenderFinding::Absent("kind".into())]
            })
        );
        assert_eq!(
            obligation.command,
            Err(RenderFailure {
                findings: vec![RenderFinding::BriefNotRendered]
            })
        );
    }

    #[test]
    fn revisiting_a_state_is_an_invalid_evaluation() {
        let cyclic = "\
flow: work
orchestrator: builder
initial: start
states: [start, middle]
transitions:
- from: start
  to: middle
  when: {exists: prd.digest}
  action: first
- from: middle
  to: start
  when: {exists: prd.digest}
  action: second
";
        let err = fixture(cyclic)
            .evaluate(
                "work",
                &subject(facts(&[("prd.digest", text("p"))])),
                &no_config,
            )
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "flow `work` revisits state `start` (walk: start → middle → start)"
        );
    }

    #[test]
    fn an_unknown_flow_is_refused() {
        let err = fixture(WALK)
            .evaluate("deploy", &subject(Facts::new()), &no_config)
            .unwrap_err();
        assert_eq!(err, EvaluationError::UnknownFlow("deploy".into()));
    }
}
