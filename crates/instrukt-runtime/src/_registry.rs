//! Runtime adapter factory and registry constructed at the CLI composition root.
use std::collections::BTreeMap;
use std::sync::Arc;

use instrukt_core::config::Env;

use crate::{RuntimeAdapter, RuntimeError};

/// Constructs runtime adapters for one compiled runtime.
pub trait RuntimeFactory: Send + Sync {
    fn runtime_name(&self) -> &'static str;

    /// One-shot adapter for interactive CLI commands (bounded short-lived tokens).
    fn create(&self, env: &Env) -> Arc<dyn RuntimeAdapter>;

    /// Long-lived adapter for daemon loops (`instrukt run`), reusing a shared read lease.
    fn create_long_lived(&self, env: &Env) -> Arc<dyn RuntimeAdapter> {
        self.create(env)
    }
}

/// Process-local registry mapping runtime names to compiled adapter factories.
#[derive(Clone, Default)]
pub struct RuntimeRegistry {
    factories: BTreeMap<&'static str, Arc<dyn RuntimeFactory>>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, factory: Arc<dyn RuntimeFactory>) {
        self.factories.insert(factory.runtime_name(), factory);
    }

    pub fn with(mut self, factory: Arc<dyn RuntimeFactory>) -> Self {
        self.register(factory);
        self
    }

    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }

    pub fn adapter(&self, name: &str, env: &Env) -> Result<Arc<dyn RuntimeAdapter>, RuntimeError> {
        let factory = self
            .factories
            .get(name)
            .ok_or_else(|| RuntimeError::UnknownRuntime(name.to_owned()))?;
        Ok(factory.create(env))
    }

    pub fn long_lived(
        &self,
        name: &str,
        env: &Env,
    ) -> Result<Arc<dyn RuntimeAdapter>, RuntimeError> {
        let factory = self
            .factories
            .get(name)
            .ok_or_else(|| RuntimeError::UnknownRuntime(name.to_owned()))?;
        Ok(factory.create_long_lived(env))
    }
}
