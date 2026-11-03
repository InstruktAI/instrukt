//! Conditions: the structural operators a definition composes over facts
//! (`docs/local/spec/feature/workflow/workflow-definition.md`, "Conditions").

use serde_yaml_ng::Value;

use super::_facts::{FactName, FactValue, Facts};
use super::_finding::Problem;

/// A typed literal a fact is compared with.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Literal {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
}

/// A numeric bound.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Bound {
    Int(i64),
    Float(f64),
}

/// One condition. A YAML map with several operator keys parses to `All`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Condition {
    Equals(FactName, Literal),
    Same(FactName, FactName),
    Exists(FactName),
    All(Vec<Condition>),
    Any(Vec<Condition>),
    Not(Box<Condition>),
    AtLeast(FactName, Bound),
    AtMost(FactName, Bound),
}

impl Condition {
    /// Whether the condition holds on `facts`. Pure and total.
    pub(crate) fn holds(&self, facts: &Facts) -> bool {
        match self {
            Condition::Equals(name, literal) => {
                facts.get(*name).is_some_and(|value| equals(value, literal))
            }
            Condition::Same(left, right) => match (facts.get(*left), facts.get(*right)) {
                (Some(a), Some(b)) => same(a, b),
                _ => false,
            },
            Condition::Exists(name) => facts.get(*name).is_some(),
            Condition::All(all) => all.iter().all(|c| c.holds(facts)),
            Condition::Any(any) => any.iter().any(|c| c.holds(facts)),
            Condition::Not(inner) => !inner.holds(facts),
            Condition::AtLeast(name, bound) => match facts.get(*name) {
                Some(FactValue::Int(value)) => match bound {
                    Bound::Int(b) => value >= b,
                    Bound::Float(b) => (*value as f64) >= *b,
                },
                _ => false,
            },
            Condition::AtMost(name, bound) => match facts.get(*name) {
                Some(FactValue::Int(value)) => match bound {
                    Bound::Int(b) => value <= b,
                    Bound::Float(b) => (*value as f64) <= *b,
                },
                _ => false,
            },
        }
    }

    /// Parses a condition, reporting every problem with its location below
    /// `at`. Returns `None` when any problem was found.
    pub(crate) fn parse(
        value: &Value,
        at: &str,
        problems: &mut Vec<(String, Problem)>,
    ) -> Option<Condition> {
        let Value::Mapping(map) = value else {
            problems.push((at.to_owned(), Problem::Shape("a condition map")));
            return None;
        };
        if map.is_empty() {
            problems.push((at.to_owned(), Problem::EmptyCondition));
            return None;
        }
        let before = problems.len();
        let mut parts = Vec::with_capacity(map.len());
        for (key, operand) in map {
            let Some(op) = key.as_str() else {
                problems.push((at.to_owned(), Problem::Shape("operator names as keys")));
                continue;
            };
            let here = format!("{at}.{op}");
            if let Some(part) = parse_operator(op, operand, &here, problems) {
                parts.push(part);
            }
        }
        if problems.len() > before {
            return None;
        }
        if parts.len() == 1 {
            parts.pop()
        } else {
            Some(Condition::All(parts))
        }
    }
}

fn parse_operator(
    op: &str,
    operand: &Value,
    at: &str,
    problems: &mut Vec<(String, Problem)>,
) -> Option<Condition> {
    match op {
        "equals" => {
            let [fact, literal] = pair(operand, at, "[fact, literal]", problems)?;
            let fact = fact_name(fact, &format!("{at}[0]"), problems);
            let literal = parse_literal(literal, &format!("{at}[1]"), problems);
            Some(Condition::Equals(fact?, literal?))
        }
        "same" => {
            let [left, right] = pair(operand, at, "[fact, fact]", problems)?;
            let left = fact_name(left, &format!("{at}[0]"), problems);
            let right = fact_name(right, &format!("{at}[1]"), problems);
            Some(Condition::Same(left?, right?))
        }
        "exists" => fact_name(operand, at, problems).map(Condition::Exists),
        "all" | "any" => {
            let Value::Sequence(items) = operand else {
                problems.push((at.to_owned(), Problem::Shape("a list of conditions")));
                return None;
            };
            let parsed: Vec<_> = items
                .iter()
                .enumerate()
                .map(|(i, item)| Condition::parse(item, &format!("{at}[{i}]"), problems))
                .collect();
            let parsed: Option<Vec<_>> = parsed.into_iter().collect();
            let parsed = parsed?;
            Some(if op == "all" {
                Condition::All(parsed)
            } else {
                Condition::Any(parsed)
            })
        }
        "not" => Condition::parse(operand, at, problems).map(|c| Condition::Not(Box::new(c))),
        "at_least" | "at_most" => {
            let [fact, bound] = pair(operand, at, "[fact, number]", problems)?;
            let fact = fact_name(fact, &format!("{at}[0]"), problems);
            let bound = match bound {
                Value::Number(n) => n
                    .as_i64()
                    .map(Bound::Int)
                    .or_else(|| n.as_f64().map(Bound::Float)),
                _ => None,
            };
            let Some(bound) = bound else {
                problems.push((format!("{at}[1]"), Problem::Shape("a numeric bound")));
                return None;
            };
            let fact = fact?;
            Some(if op == "at_least" {
                Condition::AtLeast(fact, bound)
            } else {
                Condition::AtMost(fact, bound)
            })
        }
        other => {
            problems.push((at.to_owned(), Problem::UnknownOperator(other.to_owned())));
            None
        }
    }
}

fn pair<'v>(
    operand: &'v Value,
    at: &str,
    shape: &'static str,
    problems: &mut Vec<(String, Problem)>,
) -> Option<[&'v Value; 2]> {
    if let Value::Sequence(items) = operand
        && let [first, second] = items.as_slice()
    {
        return Some([first, second]);
    }
    problems.push((at.to_owned(), Problem::Shape(shape)));
    None
}

fn fact_name(value: &Value, at: &str, problems: &mut Vec<(String, Problem)>) -> Option<FactName> {
    let Some(name) = value.as_str() else {
        problems.push((at.to_owned(), Problem::Shape("a fact name")));
        return None;
    };
    let fact = FactName::parse(name);
    if fact.is_none() {
        problems.push((at.to_owned(), Problem::UnknownFact(name.to_owned())));
    }
    fact
}

fn parse_literal(
    value: &Value,
    at: &str,
    problems: &mut Vec<(String, Problem)>,
) -> Option<Literal> {
    let literal = match value {
        Value::Bool(b) => Some(Literal::Bool(*b)),
        Value::String(s) => Some(Literal::Text(s.clone())),
        Value::Number(n) => n
            .as_i64()
            .map(Literal::Int)
            .or_else(|| n.as_f64().map(Literal::Float)),
        _ => None,
    };
    if literal.is_none() {
        problems.push((at.to_owned(), Problem::InvalidLiteral));
    }
    literal
}

/// Typed equality: no coercion between text, numbers and booleans.
fn equals(value: &FactValue, literal: &Literal) -> bool {
    match (value, literal) {
        (FactValue::Bool(a), Literal::Bool(b)) => a == b,
        (FactValue::Int(a), Literal::Int(b)) => a == b,
        (FactValue::Int(a), Literal::Float(b)) => (*a as f64) == *b,
        (_, Literal::Text(b)) => value.as_text() == Some(b.as_str()),
        _ => false,
    }
}

/// Equality of two present facts; textual values compare by their text.
fn same(a: &FactValue, b: &FactValue) -> bool {
    match (a.as_text(), b.as_text()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<Condition, Vec<(String, Problem)>> {
        let value: Value = serde_yaml_ng::from_str(yaml).unwrap();
        let mut problems = Vec::new();
        Condition::parse(&value, "when", &mut problems).ok_or(problems)
    }

    fn facts(entries: &[(&str, FactValue)]) -> Facts {
        let mut facts = Facts::new();
        for (name, value) in entries {
            facts.insert(name, value.clone()).unwrap();
        }
        facts
    }

    fn text(s: &str) -> FactValue {
        FactValue::Text(s.to_owned())
    }

    #[test]
    fn equals_compares_a_present_fact_with_a_typed_literal() {
        let c = parse("equals: [build.complete, true]").unwrap();
        assert!(c.holds(&facts(&[("build.complete", FactValue::Bool(true))])));
        assert!(!c.holds(&facts(&[("build.complete", FactValue::Bool(false))])));
        assert!(!c.holds(&Facts::new()), "absent never equals");
    }

    #[test]
    fn equals_does_not_coerce_text_to_booleans_or_numbers() {
        let as_text = parse("equals: [build.complete, 'true']").unwrap();
        assert!(!as_text.holds(&facts(&[("build.complete", FactValue::Bool(true))])));
        let number_text = parse("equals: [build_review.round, '2']").unwrap();
        assert!(!number_text.holds(&facts(&[("build_review.round", FactValue::Int(2))])));
    }

    #[test]
    fn same_needs_both_facts_present_and_equal() {
        let c = parse("same: [prd_approval.digest, prd.digest]").unwrap();
        assert!(c.holds(&facts(&[
            ("prd_approval.digest", text("d1")),
            ("prd.digest", text("d1"))
        ])));
        assert!(!c.holds(&facts(&[
            ("prd_approval.digest", text("d0")),
            ("prd.digest", text("d1"))
        ])));
        assert!(!c.holds(&facts(&[("prd.digest", text("d1"))])));
        assert!(!c.holds(&facts(&[
            ("prd_approval.digest", text("")),
            ("prd.digest", text(""))
        ])));
    }

    #[test]
    fn exists_holds_for_false_but_not_for_empty_evidence() {
        let c = parse("exists: worktree.exists").unwrap();
        assert!(c.holds(&facts(&[("worktree.exists", FactValue::Bool(false))])));
        let c = parse("exists: claim_commits.unreviewed").unwrap();
        assert!(!c.holds(&facts(&[(
            "claim_commits.unreviewed",
            FactValue::ShaList(vec![])
        )])));
    }

    #[test]
    fn numeric_bounds_need_a_present_number() {
        let least = parse("at_least: [build_review.round, 3]").unwrap();
        let most = parse("at_most: [build_review.round, 3]").unwrap();
        let round = |n| facts(&[("build_review.round", FactValue::Int(n))]);
        assert!(least.holds(&round(3)) && least.holds(&round(4)) && !least.holds(&round(2)));
        assert!(most.holds(&round(3)) && most.holds(&round(0)) && !most.holds(&round(4)));
        assert!(!least.holds(&Facts::new()) && !most.holds(&Facts::new()));
    }

    #[test]
    fn a_map_with_several_operators_means_all_of_them() {
        let c = parse("{exists: prd.digest, equals: [kind, feature]}").unwrap();
        assert!(matches!(c, Condition::All(ref parts) if parts.len() == 2));
        let both = facts(&[("prd.digest", text("d")), ("kind", text("feature"))]);
        let one = facts(&[("prd.digest", text("d")), ("kind", text("bug"))]);
        assert!(c.holds(&both));
        assert!(!c.holds(&one));
    }

    #[test]
    fn any_not_and_all_compose() {
        let c =
            parse("any: [{not: {exists: prd.digest}}, {all: [{equals: [kind, bug]}]}]").unwrap();
        assert!(c.holds(&Facts::new()));
        assert!(!c.holds(&facts(&[("prd.digest", text("d"))])));
        assert!(c.holds(&facts(&[("prd.digest", text("d")), ("kind", text("bug"))])));
    }

    #[test]
    fn every_shape_problem_is_reported_with_its_location() {
        let problems =
            parse("all: [{equals: [prd.digst, x]}, {bogus: 1}, {same: [kind]}, {}]").unwrap_err();
        assert_eq!(
            problems,
            vec![
                (
                    "when.all[0].equals[0]".to_owned(),
                    Problem::UnknownFact("prd.digst".to_owned())
                ),
                (
                    "when.all[1].bogus".to_owned(),
                    Problem::UnknownOperator("bogus".to_owned())
                ),
                (
                    "when.all[2].same".to_owned(),
                    Problem::Shape("[fact, fact]")
                ),
                ("when.all[3]".to_owned(), Problem::EmptyCondition),
            ]
        );
    }

    #[test]
    fn a_null_literal_is_refused() {
        let problems = parse("equals: [kind, null]").unwrap_err();
        assert_eq!(
            problems,
            vec![("when.equals[1]".to_owned(), Problem::InvalidLiteral)]
        );
    }
}
