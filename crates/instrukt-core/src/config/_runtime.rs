//! The runtime mapping and the active-runtime selection rule.

use std::collections::BTreeMap;
use std::fmt;

use serde_yaml_ng::Value;

/// The fields every runtime-mapping entry carries.
pub(crate) const REQUIRED_FIELDS: [&str; 2] = ["name", "session_id_env"];
/// The plain (non-verb) field an entry may carry.
pub(crate) const AGENT_FIELD: &str = "agent";
/// The key holding the history verbs `search` and `show`.
pub(crate) const HISTORY_FIELD: &str = "history";

/// The derived field `runtime.session_id`.
pub(crate) const SESSION_ID: &str = "session_id";

/// What `instrukt sessions` does with a verb (`session-continuity`, "Verb
/// kinds").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VerbKind {
    /// instrukt runs the command template.
    Command,
    /// The calling agent carries out the instruction with its own
    /// runtime's tools; instrukt runs nothing.
    Agent,
}

impl VerbKind {
    pub fn as_str(self) -> &'static str {
        match self {
            VerbKind::Command => "command",
            VerbKind::Agent => "agent",
        }
    }
}

/// A verb a runtime entry may fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VerbName {
    Start,
    Send,
    Alive,
    Conversation,
    Notify,
    Available,
    Model,
    End,
    EndSelf,
    Link,
    HistorySearch,
    HistoryShow,
}

impl VerbName {
    pub const ALL: [VerbName; 12] = [
        VerbName::Start,
        VerbName::Send,
        VerbName::Alive,
        VerbName::Conversation,
        VerbName::Notify,
        VerbName::Available,
        VerbName::Model,
        VerbName::End,
        VerbName::EndSelf,
        VerbName::Link,
        VerbName::HistorySearch,
        VerbName::HistoryShow,
    ];

    /// The verb's dotted key in an entry (`start`, `history.search`).
    pub fn key(self) -> &'static str {
        match self {
            VerbName::Start => "start",
            VerbName::Send => "send",
            VerbName::Alive => "alive",
            VerbName::Conversation => "conversation",
            VerbName::Notify => "notify",
            VerbName::Available => "available",
            VerbName::Model => "model",
            VerbName::End => "end",
            VerbName::EndSelf => "end_self",
            VerbName::Link => "link",
            VerbName::HistorySearch => "history.search",
            VerbName::HistoryShow => "history.show",
        }
    }

    /// The verb whose dotted key is `key`.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|verb| verb.key() == key)
    }

    /// Whether only kind `command` is allowed: instrukt runs these itself,
    /// including from an OS-started runner with no agent to instruct.
    pub fn command_only(self) -> bool {
        !matches!(
            self,
            VerbName::End
                | VerbName::EndSelf
                | VerbName::Link
                | VerbName::HistorySearch
                | VerbName::HistoryShow
        )
    }

    /// The verbs stored directly under the entry (all but `history.*`).
    pub(crate) fn top_level(self) -> bool {
        !matches!(self, VerbName::HistorySearch | VerbName::HistoryShow)
    }
}

impl fmt::Display for VerbName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// One filled verb: its kind and its unexpanded command template or
/// instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verb {
    pub kind: VerbKind,
    /// The command template (`command`) or the instruction (`agent`).
    pub text: String,
}

/// Why a verb's value is not one of the verb forms (`layered-configuration`,
/// "Runtime entries").
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VerbShapeError {
    /// Not a string or a mapping.
    NotAVerb,
    /// A mapping whose `kind` is missing or neither `command` nor `agent`.
    UnknownKind,
    /// A mapping lacking the field its kind takes.
    MissingText(&'static str),
    /// A mapping carrying a field its kind does not take.
    ForeignField(String),
    /// Kind `agent` on a verb instrukt runs itself.
    AgentNotAllowed,
}

impl fmt::Display for VerbShapeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerbShapeError::NotAVerb => f.write_str(
                "expected a command string, `{kind: command, template}` or `{kind: agent, instruction}`",
            ),
            VerbShapeError::UnknownKind => f.write_str("expected `kind: command` or `kind: agent`"),
            VerbShapeError::MissingText(field) => write!(f, "this kind needs `{field}`"),
            VerbShapeError::ForeignField(field) => {
                write!(f, "this kind does not take `{field}`")
            }
            VerbShapeError::AgentNotAllowed => f.write_str(
                "instrukt runs this verb itself, so it takes kind `command` only",
            ),
        }
    }
}

/// Reads one verb's value as `verb` takes it.
pub(crate) fn parse_verb(verb: VerbName, value: &Value) -> Result<Verb, VerbShapeError> {
    let parsed = match value {
        Value::String(text) => Verb {
            kind: VerbKind::Command,
            text: text.clone(),
        },
        Value::Mapping(map) => {
            let (kind, field) = match map.get("kind").and_then(Value::as_str) {
                Some("command") => (VerbKind::Command, "template"),
                Some("agent") => (VerbKind::Agent, "instruction"),
                _ => return Err(VerbShapeError::UnknownKind),
            };
            for key in map.keys() {
                let key = key.as_str().unwrap_or_default();
                if key != "kind" && key != field {
                    return Err(VerbShapeError::ForeignField(key.to_owned()));
                }
            }
            let text = map
                .get(field)
                .and_then(Value::as_str)
                .ok_or(VerbShapeError::MissingText(field))?;
            Verb {
                kind,
                text: text.to_owned(),
            }
        }
        _ => return Err(VerbShapeError::NotAVerb),
    };
    if parsed.kind == VerbKind::Agent && verb.command_only() {
        return Err(VerbShapeError::AgentNotAllowed);
    }
    Ok(parsed)
}

/// The stored value of `verb` in one entry's mapping, if present.
pub(crate) fn verb_value(entry: &Value, verb: VerbName) -> Option<&Value> {
    match verb.key().split_once('.') {
        Some((group, name)) => entry.get(group)?.get(name),
        None => entry.get(verb.key()),
    }
}

/// Inside a stored verb value, the field holding its text: the string
/// itself (`None`) or `template` / `instruction` of a verb mapping.
pub(crate) fn verb_text_field(value: &Value) -> Option<&'static str> {
    let map = value.as_mapping()?;
    match map.get("kind").and_then(Value::as_str) {
        Some("command") => Some("template"),
        Some("agent") => Some("instruction"),
        _ => None,
    }
}

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
/// and the verbs it fills. Templates and instructions are unexpanded;
/// expand them with [`super::Configuration::expand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeEntry {
    /// Position in the mapping (`runtimes.<index>`).
    pub index: usize,
    pub name: String,
    pub session_id_env: String,
    /// Set when the entry is an agent CLI: the agent's name.
    pub agent: Option<String>,
    verbs: BTreeMap<VerbName, Verb>,
}

impl RuntimeEntry {
    /// The verb `verb` as filled, with its kind; `None` when empty (or not
    /// in a verb form, which validation reports).
    pub fn verb(&self, verb: VerbName) -> Option<&Verb> {
        self.verbs.get(&verb)
    }

    /// The command template of `verb`, when it is filled with kind
    /// `command`.
    pub fn command(&self, verb: VerbName) -> Option<&str> {
        self.verb(verb)
            .filter(|v| v.kind == VerbKind::Command)
            .map(|v| v.text.as_str())
    }

    /// Whether `verb` is filled, of either kind.
    pub fn fills(&self, verb: VerbName) -> bool {
        self.verbs.contains_key(&verb)
    }

    /// Reads entry `index` of the mapping; `None` when it lacks a `name` or
    /// `session_id_env` (a validation finding).
    pub(crate) fn from_value(index: usize, value: &Value) -> Option<Self> {
        let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
        let verbs = VerbName::ALL
            .into_iter()
            .filter_map(|verb| {
                let parsed = parse_verb(verb, verb_value(value, verb)?).ok()?;
                Some((verb, parsed))
            })
            .collect();
        Some(Self {
            index,
            name: text("name")?,
            session_id_env: text("session_id_env")?,
            agent: text(AGENT_FIELD),
            verbs,
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

    fn verb(verb: VerbName, text: &str) -> Result<Verb, VerbShapeError> {
        parse_verb(verb, &serde_yaml_ng::from_str(text).unwrap())
    }

    #[test]
    fn a_string_and_a_command_map_are_commands_and_an_agent_map_an_instruction() {
        let command = |text: &str| Verb {
            kind: VerbKind::Command,
            text: text.into(),
        };
        assert_eq!(verb(VerbName::Send, "rt send"), Ok(command("rt send")));
        assert_eq!(
            verb(VerbName::Send, "{kind: command, template: rt send}"),
            Ok(command("rt send"))
        );
        assert_eq!(
            verb(VerbName::End, "{kind: agent, instruction: close it}"),
            Ok(Verb {
                kind: VerbKind::Agent,
                text: "close it".into()
            })
        );
    }

    #[test]
    fn a_verb_map_outside_its_kind_is_refused() {
        for (text, error) in [
            ("{kind: shell, template: x}", VerbShapeError::UnknownKind),
            ("{template: x}", VerbShapeError::UnknownKind),
            (
                "{kind: command, template: x, instruction: y}",
                VerbShapeError::ForeignField("instruction".into()),
            ),
            (
                "{kind: agent, template: x}",
                VerbShapeError::ForeignField("template".into()),
            ),
            ("{kind: agent}", VerbShapeError::MissingText("instruction")),
            ("[x]", VerbShapeError::NotAVerb),
        ] {
            assert_eq!(verb(VerbName::End, text), Err(error), "{text}");
        }
    }

    #[test]
    fn only_the_verbs_an_agent_may_carry_out_take_kind_agent() {
        let agent = "{kind: agent, instruction: do it}";
        for name in VerbName::ALL {
            let parsed = verb(name, agent);
            if name.command_only() {
                assert_eq!(parsed, Err(VerbShapeError::AgentNotAllowed), "{name}");
            } else {
                assert!(parsed.is_ok(), "{name}");
            }
        }
    }

    #[test]
    fn every_verb_key_names_its_verb() {
        for name in VerbName::ALL {
            assert_eq!(VerbName::from_key(name.key()), Some(name));
        }
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
