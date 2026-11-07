//! Authored frontmatter: the typed fields every consumer reads, and the
//! checks that decide where each field is allowed.

use serde::Serialize;
use serde_yaml_ng::{Mapping, Value};

use super::_finding::FindingCode;
use super::_identity::Taxonomy;
use crate::ids::TodoSlug;
use crate::subscription::{SubscriptionAttendance, SubscriptionHome, SubscriptionIdentity};

/// `visibility`: recorded metadata that filters nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Public,
    #[default]
    Internal,
}

/// One launch argument a procedure declares.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Parameter {
    pub name: String,
    pub required: bool,
    pub rest: bool,
}

/// A tier mode a procedure's dispatch defaults may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Light,
    Standard,
    Strong,
    Premium,
}

/// A procedure's launch defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Dispatch {
    /// Ordered agent preference; names are checked against the runtime
    /// mapping by configuration, not here.
    pub agents: Vec<String>,
    pub tier: Tier,
}

/// What a procedure declares about how a session is started into it. Empty
/// for every other snippet. Fields hold only well-formed values; a
/// malformed one is absent here and reported by validation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct ProcedureEntry {
    pub role: Option<String>,
    pub activation_trigger: Option<String>,
    pub subscriptions: Vec<String>,
    pub subscription_home: Option<SubscriptionHome>,
    pub subscription_identity: SubscriptionIdentity,
    pub subscription_attendance: SubscriptionAttendance,
    pub parameters: Vec<Parameter>,
    pub dispatch: Option<Dispatch>,
    pub interruptible: bool,
}

impl ProcedureEntry {
    /// Whether the procedure carries root frontmatter (`role`,
    /// `subscriptions`, `activation_trigger` or `dispatch`): a session is
    /// started into it.
    pub fn is_root(&self) -> bool {
        self.role.is_some()
            || !self.subscriptions.is_empty()
            || self.activation_trigger.is_some()
            || self.dispatch.is_some()
    }
}

/// A snippet's authored frontmatter, read leniently: a field that is
/// malformed reads as absent.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Frontmatter {
    pub description: Option<String>,
    pub visibility: Visibility,
    pub eval: bool,
    /// Non-empty when the snippet is pending.
    pub delivered_by: Vec<TodoSlug>,
    pub entry: ProcedureEntry,
    /// The whole mapping as authored.
    pub raw: Mapping,
}

const PROCEDURE_FIELDS: [&str; 9] = [
    "activation_trigger",
    "subscriptions",
    "subscription_home",
    "subscription_identity",
    "subscription_attendance",
    "role",
    "parameters",
    "dispatch",
    "interruptible",
];
const CLASSIFICATION_FIELDS: [&str; 4] = ["id", "type", "scope", "domain"];

/// The YAML of a frontmatter block as a mapping; `None` when it is not one.
/// An empty block is an empty mapping.
pub(super) fn parse_mapping(yaml: Option<&str>) -> Option<Mapping> {
    let Some(yaml) = yaml else {
        return Some(Mapping::new());
    };
    match serde_yaml_ng::from_str::<Value>(yaml) {
        Ok(Value::Mapping(map)) => Some(map),
        Ok(Value::Null) => Some(Mapping::new()),
        _ => None,
    }
}

fn get<'a>(map: &'a Mapping, key: &str) -> Option<&'a Value> {
    map.get(key)
}

fn non_empty_str(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

impl Frontmatter {
    /// Reads the typed fields of `raw`. Procedure fields are read only for
    /// a procedure.
    pub(super) fn read(raw: Mapping, taxonomy: Option<Taxonomy>) -> Self {
        let description = non_empty_str(get(&raw, "description"));
        let visibility = match get(&raw, "visibility").and_then(Value::as_str) {
            Some("public") => Visibility::Public,
            _ => Visibility::Internal,
        };
        let eval = get(&raw, "eval").and_then(Value::as_bool).unwrap_or(false);
        let delivered_by = get(&raw, "delivered_by")
            .and_then(Value::as_sequence)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|v| v.as_str().and_then(|s| TodoSlug::parse(s).ok()))
                    .collect()
            })
            .unwrap_or_default();
        let entry = if taxonomy == Some(Taxonomy::Procedure) {
            read_entry(&raw)
        } else {
            ProcedureEntry::default()
        };
        Self {
            description,
            visibility,
            eval,
            delivered_by,
            entry,
            raw,
        }
    }
}

fn read_entry(raw: &Mapping) -> ProcedureEntry {
    let role = non_empty_str(get(raw, "role"));
    let activation_trigger = non_empty_str(get(raw, "activation_trigger"));
    let subscriptions = match get(raw, "subscriptions").and_then(Value::as_sequence) {
        Some(items)
            if items
                .iter()
                .all(|v| v.as_str().is_some_and(is_type_pattern)) =>
        {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        }
        _ => Vec::new(),
    };
    let parameters = get(raw, "parameters")
        .filter(|_| role.is_some())
        .and_then(|v| parse_parameters(v).ok())
        .unwrap_or_default();
    let dispatch = get(raw, "dispatch")
        .filter(|_| role.is_some())
        .and_then(|v| parse_dispatch(v).ok());
    let interruptible = get(raw, "interruptible")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    ProcedureEntry {
        role,
        activation_trigger,
        subscriptions,
        subscription_home: get(raw, "subscription_home")
            .and_then(|v| serde_yaml_ng::from_value(v.clone()).ok()),
        subscription_identity: get(raw, "subscription_identity")
            .and_then(|v| serde_yaml_ng::from_value(v.clone()).ok())
            .unwrap_or_default(),
        subscription_attendance: get(raw, "subscription_attendance")
            .and_then(|v| serde_yaml_ng::from_value(v.clone()).ok())
            .unwrap_or_default(),
        parameters,
        dispatch,
        interruptible,
    }
}

/// An event type pattern: an exact dotted name, a whole-segment prefix
/// followed by `.*`, or `*`.
pub(super) fn is_type_pattern(pattern: &str) -> bool {
    crate::event_type::TypePattern::parse(pattern).is_ok()
}

fn parse_parameters(value: &Value) -> Result<Vec<Parameter>, String> {
    let items = value.as_sequence().ok_or_else(|| "not a list".to_owned())?;
    let mut out: Vec<Parameter> = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let map = item
            .as_mapping()
            .ok_or_else(|| format!("entry {index} is not a mapping"))?;
        if let Some(key) = map
            .keys()
            .find(|k| !matches!(k.as_str(), Some("name" | "required" | "rest")))
        {
            return Err(format!("entry {index} has key {}", yaml_text(key)));
        }
        let name =
            non_empty_str(map.get("name")).ok_or_else(|| format!("entry {index} has no name"))?;
        let flag = |key: &str| match map.get(key) {
            None => Ok(false),
            Some(v) => v
                .as_bool()
                .ok_or_else(|| format!("entry {index} `{key}` is not a boolean")),
        };
        let required = flag("required")?;
        let rest = flag("rest")?;
        if out.iter().any(|p| p.name == name) {
            return Err(format!("duplicate name {name}"));
        }
        out.push(Parameter {
            name,
            required,
            rest,
        });
    }
    if let Some(position) = out.iter().position(|p| p.rest)
        && position + 1 != out.len()
    {
        return Err(format!("rest entry {} is not last", out[position].name));
    }
    Ok(out)
}

fn parse_dispatch(value: &Value) -> Result<Dispatch, String> {
    let map = value
        .as_mapping()
        .ok_or_else(|| "not a mapping".to_owned())?;
    if let Some(key) = map
        .keys()
        .find(|k| !matches!(k.as_str(), Some("agent" | "tier")))
    {
        return Err(format!("unknown key {}", yaml_text(key)));
    }
    let agents: Vec<String> = match map.get("agent").and_then(Value::as_sequence) {
        Some(items) if !items.is_empty() => items
            .iter()
            .map(|v| non_empty_str(Some(v)).ok_or_else(|| format!("agent {}", yaml_text(v))))
            .collect::<Result<_, _>>()?,
        _ => return Err("agent is not a non-empty list".to_owned()),
    };
    let tier = match map.get("tier").and_then(Value::as_str) {
        Some("light") => Tier::Light,
        Some("standard") => Tier::Standard,
        Some("strong") => Tier::Strong,
        Some("premium") => Tier::Premium,
        other => {
            return Err(format!(
                "tier {}",
                other.map_or_else(|| "missing".to_owned(), str::to_owned)
            ));
        }
    };
    Ok(Dispatch { agents, tier })
}

fn yaml_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        other => serde_yaml_ng::to_string(other)
            .map(|s| s.trim().to_owned())
            .unwrap_or_default(),
    }
}

/// One frontmatter finding: its code and offending value.
pub(super) type Issue = (FindingCode, Option<String>);

/// The frontmatter findings of a snippet of `taxonomy` (`None` when the
/// path yields no taxonomy) whose file name stem is `stem`. `yaml` is the
/// frontmatter as written, which `raw` was parsed from.
pub(super) fn check(
    raw: &Mapping,
    yaml: Option<&str>,
    taxonomy: Option<Taxonomy>,
    stem: &str,
) -> Vec<Issue> {
    let mut issues = Vec::new();
    let authored: Vec<&str> = CLASSIFICATION_FIELDS
        .into_iter()
        .filter(|key| raw.contains_key(*key))
        .collect();
    if !authored.is_empty() {
        issues.push((
            FindingCode::ClassificationAuthored,
            Some(authored.join(", ")),
        ));
    }
    if non_empty_str(get(raw, "description")).is_none() {
        issues.push((FindingCode::DescriptionMissing, None));
    }
    if let Some(value) = get(raw, "visibility")
        && !matches!(value.as_str(), Some("public" | "internal"))
    {
        issues.push((FindingCode::VisibilityInvalid, Some(yaml_text(value))));
    }
    if let Some(value) = get(raw, "eval")
        && !value.is_bool()
    {
        issues.push((FindingCode::EvalInvalid, Some(yaml_text(value))));
    }
    check_delivered_by(raw, &mut issues);
    if let Some(value) = get(raw, "binding_reservations")
        && (taxonomy != Some(Taxonomy::Spec)
            || !value.as_mapping().is_some_and(|map| {
                map.iter().all(|(id, slug)| {
                    id.as_str()
                        .and_then(|id| crate::ids::UseCaseId::parse(id).ok())
                        .is_some()
                        && slug
                            .as_str()
                            .and_then(|slug| TodoSlug::parse(slug).ok())
                            .is_some()
                })
            }))
    {
        issues.push((
            FindingCode::BindingReservationsInvalid,
            Some(yaml_text(value)),
        ));
    }
    match taxonomy {
        Some(Taxonomy::Procedure) => check_procedure(raw, &mut issues),
        Some(_) => {
            for field in PROCEDURE_FIELDS {
                if raw.contains_key(field) {
                    issues.push((FindingCode::ProcedureFieldMisplaced, Some(field.to_owned())));
                }
            }
        }
        None => {}
    }
    if taxonomy == Some(Taxonomy::Adr) {
        check_adr(raw, yaml, stem, &mut issues);
    }
    issues
}

fn check_delivered_by(raw: &Mapping, issues: &mut Vec<Issue>) {
    let Some(value) = get(raw, "delivered_by") else {
        return;
    };
    let items = match value.as_sequence() {
        Some(items) if !items.is_empty() && items.iter().all(Value::is_string) => items,
        _ => {
            issues.push((FindingCode::DeliveredByInvalid, Some(yaml_text(value))));
            return;
        }
    };
    for slug in items.iter().filter_map(Value::as_str) {
        if TodoSlug::parse(slug).is_err() {
            issues.push((FindingCode::PendingSlugInvalid, Some(slug.to_owned())));
        }
    }
}

fn check_procedure(raw: &Mapping, issues: &mut Vec<Issue>) {
    let trigger = get(raw, "activation_trigger");
    let subscriptions = get(raw, "subscriptions");
    if let Some(value) = trigger
        && non_empty_str(Some(value)).is_none()
    {
        issues.push((
            FindingCode::ActivationTriggerInvalid,
            Some(yaml_text(value)),
        ));
    }
    if let Some(value) = subscriptions {
        match value.as_sequence() {
            Some(items) if !items.is_empty() => {
                for item in items {
                    if !item.as_str().is_some_and(is_type_pattern) {
                        issues.push((FindingCode::SubscriptionsInvalid, Some(yaml_text(item))));
                    }
                }
            }
            _ => issues.push((FindingCode::SubscriptionsInvalid, Some(yaml_text(value)))),
        }
    }
    if trigger.is_some() && subscriptions.is_some() {
        issues.push((FindingCode::ActivationConflict, None));
    }
    let role = get(raw, "role");
    if let Some(value) = role
        && non_empty_str(Some(value)).is_none()
    {
        issues.push((FindingCode::RoleInvalid, Some(yaml_text(value))));
    }
    if subscriptions.is_some() && role.is_none() {
        issues.push((FindingCode::SubscriptionsWithoutRole, None));
    }
    if subscriptions.is_some() && !raw.contains_key("subscription_home") {
        issues.push((
            FindingCode::SubscriptionHomeMissing,
            Some("subscription_home".into()),
        ));
    }
    for field in [
        "subscription_home",
        "subscription_identity",
        "subscription_attendance",
        "interruptible",
    ] {
        let Some(value) = get(raw, field) else {
            continue;
        };
        if subscriptions.is_none() {
            issues.push((
                FindingCode::DeliverySettingMisplaced,
                Some(field.to_owned()),
            ));
            continue;
        }
        let valid = match field {
            "subscription_home" => matches!(
                value.as_str(),
                Some("project" | "platform" | "company" | "person")
            ),
            "subscription_identity" => matches!(value.as_str(), Some("home" | "subject")),
            "subscription_attendance" => matches!(value.as_str(), Some("attended" | "unattended")),
            _ => value.is_bool(),
        };
        if !valid {
            issues.push((
                FindingCode::DeliverySettingInvalid,
                Some(format!("{field}: {}", yaml_text(value))),
            ));
        }
    }
    if let Some(value) = get(raw, "parameters") {
        if role.is_none() {
            issues.push((FindingCode::ParametersWithoutRole, None));
        } else if let Err(why) = parse_parameters(value) {
            issues.push((FindingCode::ParametersInvalid, Some(why)));
        }
    }
    if let Some(value) = get(raw, "dispatch") {
        if role.is_none() {
            issues.push((FindingCode::DispatchWithoutRole, None));
        } else if let Err(why) = parse_dispatch(value) {
            issues.push((FindingCode::DispatchInvalid, Some(why)));
        }
    }
}

fn check_adr(raw: &Mapping, yaml: Option<&str>, stem: &str, issues: &mut Vec<Issue>) {
    let date = get(raw, "date");
    if !date.and_then(Value::as_str).is_some_and(is_iso_date) {
        issues.push((FindingCode::AdrDateInvalid, date.map(yaml_text)));
    }
    let number = get(raw, "number");
    let prefix: String = stem.chars().take_while(char::is_ascii_digit).collect();
    let matches_prefix =
        adr_number(number, yaml).is_some_and(|n| prefix.parse::<u64>().is_ok_and(|p| p == n));
    if !matches_prefix {
        issues.push((FindingCode::AdrNumberInvalid, number.map(yaml_text)));
    }
}

/// An `adr`'s `number`: an unquoted integer. Written with leading zeros
/// (`0049`, as its file name prefix is) it is read as that decimal integer,
/// never as YAML 1.1 octal; a quoted value is a string, not a number.
fn adr_number(number: Option<&Value>, yaml: Option<&str>) -> Option<u64> {
    let written = yaml.and_then(|yaml| top_level_value(yaml, "number"));
    match written {
        Some(text) if !text.is_empty() && text.chars().all(|c| c.is_ascii_digit()) => {
            text.parse().ok()
        }
        Some(text) if text.starts_with(['\'', '"']) => None,
        _ => number.and_then(Value::as_u64),
    }
}

/// The value text of a top-level `key:` line of `yaml`, as written, without
/// a trailing comment.
fn top_level_value<'y>(yaml: &'y str, key: &str) -> Option<&'y str> {
    yaml.lines().find_map(|line| {
        let rest = line.strip_prefix(key)?.trim_start_matches([' ', '\t']);
        let value = rest.strip_prefix(':')?;
        let value = match value.find(" #") {
            Some(at) => &value[..at],
            None => value,
        };
        Some(value.trim())
    })
}

fn is_iso_date(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    matches!(parts[..], [y, m, d]
        if y.len() == 4 && m.len() == 2 && d.len() == 2
            && [y, m, d].iter().all(|p| p.chars().all(|c| c.is_ascii_digit()))
            && (1..=12).contains(&m.parse::<u8>().unwrap_or(0))
            && (1..=31).contains(&d.parse::<u8>().unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(yaml: &str) -> Mapping {
        parse_mapping(Some(yaml)).unwrap()
    }

    fn codes(yaml: &str, taxonomy: Taxonomy) -> Vec<FindingCode> {
        check(&map(yaml), Some(yaml), Some(taxonomy), "x")
            .into_iter()
            .map(|(code, _)| code)
            .collect()
    }

    #[test]
    fn a_minimal_snippet_passes() {
        assert!(
            codes(
                "description: d\nvisibility: public\neval: true\n",
                Taxonomy::Policy
            )
            .is_empty()
        );
    }

    #[test]
    fn misplaced_and_malformed_fields_are_findings() {
        assert_eq!(
            codes("description: d\nrole: x\nid: a/b\n", Taxonomy::Concept),
            [
                FindingCode::ClassificationAuthored,
                FindingCode::ProcedureFieldMisplaced
            ]
        );
        assert_eq!(
            codes("visibility: secret\neval: yes please\n", Taxonomy::Policy),
            [
                FindingCode::DescriptionMissing,
                FindingCode::VisibilityInvalid,
                FindingCode::EvalInvalid
            ]
        );
    }

    #[test]
    fn procedure_entry_rules_hold() {
        let found = codes(
            "description: d\nactivation_trigger: t\nsubscriptions: ['a.b.*', 'a*b']\n\
             parameters: []\ndispatch: {agent: [x], tier: standard}\ninterruptible: 3\n",
            Taxonomy::Procedure,
        );
        assert_eq!(
            found,
            [
                FindingCode::SubscriptionsInvalid,
                FindingCode::ActivationConflict,
                FindingCode::SubscriptionsWithoutRole,
                FindingCode::SubscriptionHomeMissing,
                FindingCode::DeliverySettingInvalid,
                FindingCode::ParametersWithoutRole,
                FindingCode::DispatchWithoutRole,
            ]
        );
    }

    #[test]
    fn parameters_and_dispatch_are_read_with_a_role() {
        let fm = Frontmatter::read(
            map(
                "description: d\nrole: builder\nparameters: [{name: slug, required: true}, {name: rest, rest: true}]\n\
                 dispatch: {agent: [claude], tier: strong}\n",
            ),
            Some(Taxonomy::Procedure),
        );
        assert_eq!(fm.entry.parameters.len(), 2);
        assert_eq!(fm.entry.dispatch.map(|d| d.tier), Some(Tier::Strong));
        assert!(
            codes(
                "description: d\nrole: r\nparameters: [{name: a, rest: true}, {name: b}]\n",
                Taxonomy::Procedure
            )
            .contains(&FindingCode::ParametersInvalid)
        );
    }

    fn adr_number_codes(number: &str, stem: &str) -> Vec<FindingCode> {
        check(
            &map(&format!(
                "description: d\ndate: 2026-01-02\nnumber: {number}\n"
            )),
            Some(&format!(
                "description: d\ndate: 2026-01-02\nnumber: {number}\n"
            )),
            Some(Taxonomy::Adr),
            stem,
        )
        .into_iter()
        .map(|(code, _)| code)
        .collect()
    }

    #[test]
    fn an_adr_number_with_leading_zeros_is_read_as_that_integer() {
        assert!(adr_number_codes("0049", "0049-choice").is_empty());
    }

    #[test]
    fn an_adr_number_with_leading_zeros_is_never_read_as_octal() {
        assert!(adr_number_codes("0010", "0010-choice").is_empty());
    }

    #[test]
    fn an_adr_number_with_leading_zeros_still_has_to_match_its_prefix() {
        assert_eq!(
            adr_number_codes("0049", "0048-choice"),
            [FindingCode::AdrNumberInvalid]
        );
    }

    #[test]
    fn a_quoted_adr_number_is_refused() {
        assert_eq!(
            adr_number_codes("'1'", "0001-choice"),
            [FindingCode::AdrNumberInvalid]
        );
    }

    #[test]
    fn a_quoted_adr_number_with_leading_zeros_is_refused() {
        assert_eq!(
            adr_number_codes("\"0049\"", "0049-choice"),
            [FindingCode::AdrNumberInvalid]
        );
    }

    #[test]
    fn adr_needs_a_date_and_a_number_matching_its_prefix() {
        let ok = check(
            &map("description: d\ndate: 2026-01-02\nnumber: 7\n"),
            Some("description: d\ndate: 2026-01-02\nnumber: 7\n"),
            Some(Taxonomy::Adr),
            "0007-x",
        );
        assert!(ok.is_empty(), "{ok:?}");
        let bad = check(
            &map("description: d\nnumber: 8\n"),
            Some("description: d\nnumber: 8\n"),
            Some(Taxonomy::Adr),
            "0007-x",
        );
        assert_eq!(bad.len(), 2);
    }

    #[test]
    fn delivered_by_must_be_a_non_empty_list_of_slugs() {
        assert_eq!(
            codes("description: d\ndelivered_by: []\n", Taxonomy::Spec),
            [FindingCode::DeliveredByInvalid]
        );
        assert_eq!(
            codes("description: d\ndelivered_by: [Bad Slug]\n", Taxonomy::Spec),
            [FindingCode::PendingSlugInvalid]
        );
    }

    #[test]
    fn type_patterns_follow_the_grammar() {
        for ok in ["*", "a", "a.b", "a.b.*"] {
            assert!(is_type_pattern(ok), "{ok}");
        }
        for bad in ["", "a.", "a*", "*.a", "a..b", "a b"] {
            assert!(!is_type_pattern(bad), "{bad}");
        }
    }
}
