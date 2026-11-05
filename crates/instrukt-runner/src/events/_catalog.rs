//! Producer declarations and catalog convention checks.

use std::collections::BTreeMap;

use instrukt_core::event_type::is_type_name;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{EnvelopeError, Event, EventEnvelope, EventHome, HomeKind, ReferenceKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DataValueKind {
    String,
    Integer,
    Number,
    Boolean,
    Array,
    Object,
    Any,
}

impl DataValueKind {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Integer => value.is_i64() || value.is_u64(),
            Self::Number => value.is_number(),
            Self::Boolean => value.is_boolean(),
            Self::Array => value.is_array(),
            Self::Object => value.is_object(),
            Self::Any => true,
        }
    }
}

/// Wire shape derived from the actual Rust field type, never a parallel list.
pub trait DataFieldType {
    const KIND: DataValueKind;
    const NULLABLE: bool = false;
}

macro_rules! field_type {
    ($kind:ident: $($ty:ty),+ $(,)?) => { $(impl DataFieldType for $ty {
        const KIND: DataValueKind = DataValueKind::$kind;
    })+ };
}
field_type!(String: String, instrukt_core::ids::ProjectSlug, instrukt_core::ids::CommitSha,
    instrukt_core::ids::TodoSlug, super::CiResult);
field_type!(Integer: i64, u64, u32, usize);
field_type!(Number: f64);
field_type!(Boolean: bool);
field_type!(Any: Value);
impl<T> DataFieldType for Vec<T> {
    const KIND: DataValueKind = DataValueKind::Array;
}
impl DataFieldType for serde_json::Map<String, Value> {
    const KIND: DataValueKind = DataValueKind::Object;
}
impl<T: DataFieldType> DataFieldType for Option<T> {
    const KIND: DataValueKind = T::KIND;
    const NULLABLE: bool = true;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DataField {
    pub name: String,
    pub kind: DataValueKind,
    pub nullable: bool,
}

impl DataField {
    pub fn of<T: DataFieldType>(name: &str) -> Self {
        Self {
            name: name.into(),
            kind: T::KIND,
            nullable: T::NULLABLE,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateStanding {
    Failing,
    Ok,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Failed,
    Stalled,
    Degraded,
    Detected,
    Succeeded,
    Recovered,
    Subsided,
}

impl LifecycleState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Failed => "failed",
            Self::Stalled => "stalled",
            Self::Degraded => "degraded",
            Self::Detected => "detected",
            Self::Succeeded => "succeeded",
            Self::Recovered => "recovered",
            Self::Subsided => "subsided",
        }
    }
    pub fn standing(self) -> StateStanding {
        match self {
            Self::Failed | Self::Stalled | Self::Degraded | Self::Detected => {
                StateStanding::Failing
            }
            Self::Succeeded | Self::Recovered | Self::Subsided => StateStanding::Ok,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateDeclaration {
    pub family: String,
    pub state: LifecycleState,
    pub standing: StateStanding,
}

impl StateDeclaration {
    pub fn new(family: &str, state: LifecycleState) -> Self {
        Self {
            family: family.into(),
            state,
            standing: state.standing(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventDeclaration {
    pub name: String,
    pub version: u32,
    pub description: String,
    /// None permits any received reference kind.
    pub subject_kind: Option<ReferenceKind>,
    /// Empty means no registered home (EventHome::None).
    pub home_kinds: Vec<HomeKind>,
    pub data_fields: Vec<DataField>,
    pub related_kinds: Vec<ReferenceKind>,
    pub state: Option<StateDeclaration>,
}

impl EventDeclaration {
    /// Producer-side validation. Received data remains open, so unknown fields
    /// and optional additions survive without reinterpretation by consumers.
    pub fn validate(&self, event: &EventEnvelope) -> Result<(), EnvelopeError> {
        event.validate()?;
        let invalid = |why: &str| EnvelopeError::Declaration(format!("{}: {why}", self.name));
        if event.event_type != self.name {
            return Err(invalid("type differs"));
        }
        if self
            .subject_kind
            .is_some_and(|kind| kind.as_str() != event.subject.kind())
        {
            return Err(invalid("subject kind differs"));
        }
        match event.home.kind() {
            Some(kind) if !self.home_kinds.contains(&kind) => {
                return Err(invalid("origin kind is not allowed"));
            }
            None if self.home_kinds.is_empty() && event.home != EventHome::None => {
                return Err(invalid("origin must be none"));
            }
            _ => {}
        }
        for reference in &event.related {
            if !self
                .related_kinds
                .iter()
                .any(|kind| kind.as_str() == reference.kind())
            {
                return Err(invalid("related kind is not allowed"));
            }
        }
        for field in &self.data_fields {
            match event.data.get(&field.name) {
                None if field.nullable => {}
                Some(Value::Null) if field.nullable => {}
                Some(value) if field.kind.accepts(value) && !value.is_null() => {}
                _ => {
                    return Err(invalid(&format!(
                        "invalid or missing data field `{}`",
                        field.name
                    )));
                }
            }
        }
        Ok(())
    }

    /// Typed producers declare every fact they construct. Received readers use
    /// `validate` instead, so additive facts from newer producers remain open.
    pub fn validate_producer(&self, event: &EventEnvelope) -> Result<(), EnvelopeError> {
        self.validate(event)?;
        if let Some(name) = event
            .data
            .keys()
            .find(|name| !self.data_fields.iter().any(|field| &field.name == *name))
        {
            return Err(EnvelopeError::Declaration(format!(
                "{}: undeclared data field `{name}`",
                self.name
            )));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid event catalog: {0}")]
pub struct CatalogError(String);

#[derive(Debug, Clone)]
pub struct EventCatalog {
    declarations: BTreeMap<String, EventDeclaration>,
}

impl EventCatalog {
    /// Validate a declaration set, including complete platform lifecycle families.
    pub fn new(declarations: Vec<EventDeclaration>) -> Result<Self, CatalogError> {
        let mut registry = BTreeMap::new();
        for declaration in declarations {
            let name = &declaration.name;
            let invalid = |why: &str| CatalogError(format!("{name}: {why}"));
            if !is_type_name(name)
                || !name.contains('.')
                || declaration.version == 0
                || declaration.description.trim().is_empty()
            {
                return Err(invalid(
                    "name, positive version and description are required",
                ));
            }
            if registry.contains_key(name) {
                return Err(invalid("declared more than once"));
            }
            let platform = name.split('.').next() == Some("instrukt");
            if let Some(state) = &declaration.state {
                if !platform
                    || name != &format!("{}.{}", state.family, state.state.as_str())
                    || state.standing != state.state.standing()
                {
                    return Err(invalid("inconsistent lifecycle declaration"));
                }
            } else if platform
                && name.rsplit('.').next().is_some_and(|last| {
                    matches!(
                        last,
                        "failed"
                            | "stalled"
                            | "degraded"
                            | "detected"
                            | "succeeded"
                            | "recovered"
                            | "subsided"
                    )
                })
            {
                return Err(invalid(
                    "reserved lifecycle segment without a declared state",
                ));
            }
            registry.insert(declaration.name.clone(), declaration);
        }
        for declaration in registry.values() {
            let Some(state) = &declaration.state else {
                continue;
            };
            let family: Vec<_> = registry
                .values()
                .filter(|other| {
                    other
                        .state
                        .as_ref()
                        .is_some_and(|s| s.family == state.family)
                })
                .collect();
            if family
                .iter()
                .any(|other| other.subject_kind != declaration.subject_kind)
            {
                return Err(CatalogError(format!(
                    "{}: inconsistent family subjects",
                    state.family
                )));
            }
            if state.standing == StateStanding::Failing
                && !family.iter().any(|other| {
                    other
                        .state
                        .as_ref()
                        .is_some_and(|s| s.standing == StateStanding::Ok)
                })
            {
                return Err(CatalogError(format!(
                    "{}: failing family has no ok state",
                    state.family
                )));
            }
        }
        Ok(Self {
            declarations: registry,
        })
    }

    pub fn builtin() -> Result<Self, CatalogError> {
        let mut declarations = Event::declarations();
        declarations.push(super::DeadLettered::declaration());
        Self::new(declarations)
    }

    pub fn get(&self, name: &str) -> Option<&EventDeclaration> {
        self.declarations.get(name)
    }

    pub fn list(&self, domain: Option<&str>) -> Vec<&EventDeclaration> {
        self.declarations
            .values()
            .filter(|declaration| {
                domain.is_none_or(|owner| declaration.name.split('.').next() == Some(owner))
            })
            .collect()
    }
}
