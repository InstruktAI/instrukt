//! What a call reaches.
use std::fmt;

/// A service the operator recognises, the scope a call reaches within it
/// (such as a company and a connector), and optionally what within that scope
/// the call uses (such as a Web API method). The scope alone is a key of its
/// own, for waits on the whole scope.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    service: &'static str,
    scope: Vec<String>,
    item: Option<String>,
}

impl Key {
    /// The whole `scope` of `service`.
    pub fn new(service: &'static str, scope: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            service,
            scope: scope.into_iter().map(Into::into).collect(),
            item: None,
        }
    }

    /// `item` within this key's scope.
    pub fn item(&self, item: impl Into<String>) -> Self {
        Self {
            service: self.service,
            scope: self.scope.clone(),
            item: Some(item.into()),
        }
    }

    pub fn service(&self) -> &'static str {
        self.service
    }

    /// The key of this key's whole scope.
    pub(crate) fn scope_key(&self) -> Self {
        Self {
            service: self.service,
            scope: self.scope.clone(),
            item: None,
        }
    }
}

/// The scope joined by `/`, then the item when there is one; never the
/// service, which a rendering names on its own.
impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.scope.join("/"))?;
        if let Some(item) = &self.item {
            write!(f, " {item}")?;
        }
        Ok(())
    }
}
