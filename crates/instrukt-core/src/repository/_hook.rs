//! The git hooks instrukt uses (`hook-sections`, "Hooks used").

use std::fmt;

use serde::{Deserialize, Serialize};

/// A hook point instrukt holds a section in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Hook {
    PreCommit,
    PostCommit,
    PostMerge,
    PostRewrite,
    PostCheckout,
    PrePush,
}

impl Hook {
    /// Every hook instrukt may hold a section in, in reporting order.
    pub const ALL: [Hook; 6] = [
        Hook::PreCommit,
        Hook::PostCommit,
        Hook::PostMerge,
        Hook::PostRewrite,
        Hook::PostCheckout,
        Hook::PrePush,
    ];

    /// The hook file's name.
    pub fn name(self) -> &'static str {
        match self {
            Hook::PreCommit => "pre-commit",
            Hook::PostCommit => "post-commit",
            Hook::PostMerge => "post-merge",
            Hook::PostRewrite => "post-rewrite",
            Hook::PostCheckout => "post-checkout",
            Hook::PrePush => "pre-push",
        }
    }

    /// The hook named `name`, if instrukt uses it.
    pub fn parse(name: &str) -> Option<Hook> {
        Hook::ALL.into_iter().find(|hook| hook.name() == name)
    }

    /// Whether a run of this hook starts `instrukt sync`: every post-hook.
    pub(crate) fn starts_sync(self) -> bool {
        !matches!(self, Hook::PreCommit | Hook::PrePush)
    }

    /// Whether the hook is installed only while the blueprint module is on.
    pub(crate) fn needs_blueprint(self) -> bool {
        matches!(self, Hook::PrePush)
    }
}

impl fmt::Display for Hook {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_hook_round_trips_through_its_file_name() {
        for hook in Hook::ALL {
            assert_eq!(Hook::parse(hook.name()), Some(hook));
        }
        assert_eq!(Hook::parse("pre-rebase"), None);
    }
}
