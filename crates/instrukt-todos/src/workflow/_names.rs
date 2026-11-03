//! Names a flavor declares: flows, states, actions and effects. Each is
//! constructed only from a loaded flavor, so a value always names something
//! the flavor declares.

use std::fmt;

macro_rules! flavor_name {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(String);

        impl $name {
            pub(crate) fn new(name: &str) -> Self {
                Self(name.to_owned())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl std::borrow::Borrow<str> for $name {
            fn borrow(&self) -> &str {
                &self.0
            }
        }
    };
}

flavor_name!(
    /// A flow: the stem of `workflow/<flow>.yaml` (`prepare`, `work`).
    FlowName
);
flavor_name!(
    /// A state a definition declares.
    StateName
);
flavor_name!(
    /// An action id keyed at the root of `actions.yaml`.
    ActionId
);
flavor_name!(
    /// An effect keyed under `effects:` in `actions.yaml`.
    EffectName
);
