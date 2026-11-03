//! The switchable modules: the one place their configuration keys are named.

/// A switchable module. Core has no switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Module {
    Blueprint,
    Todos,
    Events,
}

impl Module {
    /// Every switchable module, in the order the schema lists them.
    pub const ALL: [Module; 3] = [Module::Blueprint, Module::Todos, Module::Events];

    /// The key below `modules` that switches it, and its name in `requires`.
    pub fn key(self) -> &'static str {
        match self {
            Module::Blueprint => "blueprint",
            Module::Todos => "todos",
            Module::Events => "events",
        }
    }

    /// The module a configuration key names.
    pub fn from_key(key: &str) -> Option<Module> {
        Module::ALL.into_iter().find(|module| module.key() == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_module_round_trips_through_its_key() {
        for module in Module::ALL {
            assert_eq!(Module::from_key(module.key()), Some(module));
        }
        assert_eq!(Module::from_key("core"), None);
    }
}
