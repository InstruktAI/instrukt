//! Reservation: a todo's planned bindings against the use cases other todos
//! already claim (`binding-reservation`).

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use super::_inventory::{Claim, Inventory};
use super::SpecsError;
use crate::docs::Corpus;
use crate::home::Home;
use crate::ids::{SnippetId, TodoSlug, UseCaseId, UseCaseRef};

/// One `--binding <spec_id>:<UC-id>` value, as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedBinding {
    pub spec: String,
    pub use_case: String,
}

/// A requested binding without the `:` separator.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is not `<spec_id>:<UC-id>`")]
pub struct MalformedBinding(pub String);

impl FromStr for RequestedBinding {
    type Err = MalformedBinding;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (spec, use_case) = value
            .split_once(':')
            .ok_or_else(|| MalformedBinding(value.to_owned()))?;
        Ok(Self {
            spec: spec.to_owned(),
            use_case: use_case.to_owned(),
        })
    }
}

impl fmt::Display for RequestedBinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.spec, self.use_case)
    }
}

impl RequestedBinding {
    fn reference(&self) -> Option<UseCaseRef> {
        Some(UseCaseRef {
            spec: SnippetId::parse(&self.spec).ok()?,
            use_case: UseCaseId::parse(&self.use_case).ok()?,
        })
    }
}

/// The answer for one requested binding that names a use case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// Unclaimed, or claimed by the requesting todo.
    Clear,
    /// Claimed by another todo.
    Reserved { holder: TodoSlug },
    /// In a whole file pending for several todos with no in-body markers.
    Ambiguous { delivered_by: Vec<TodoSlug> },
}

/// What the reservation check found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reservation {
    /// Some requested bindings name no use case; no claim was checked.
    Unknown(Vec<RequestedBinding>),
    /// Every requested binding names a use case: each one's answer, in
    /// request order.
    Checked(Vec<(UseCaseRef, Answer)>),
}

/// The overall verdict, worst first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Unknown,
    Reserved,
    Ambiguous,
    Clear,
}

impl Reservation {
    /// Checks `requested` for the todo `slug` against the claims in the
    /// working tree of the repository rooted at `repo_root`.
    pub fn check(
        repo_root: &Path,
        home: &Home,
        slug: &TodoSlug,
        requested: &[RequestedBinding],
    ) -> Result<Self, SpecsError> {
        let corpus = Corpus::load(repo_root, home)?;
        let inventory = Inventory::read(&corpus);
        let reservation = Self::against(&inventory, slug, requested);
        tracing::debug!(
            event = "specs_reservation_checked",
            repo = %repo_root.display(),
            slug = %slug,
            requested = requested.len(),
            verdict = ?reservation.verdict(),
            "binding reservation checked"
        );
        Ok(reservation)
    }

    fn against(inventory: &Inventory, slug: &TodoSlug, requested: &[RequestedBinding]) -> Self {
        let mut unknown = Vec::new();
        let mut known = Vec::new();
        for request in requested {
            match request.reference().and_then(|r| inventory.use_case(&r)) {
                Some(use_case) => known.push(use_case),
                None => unknown.push(request.clone()),
            }
        }
        if !unknown.is_empty() {
            return Self::Unknown(unknown);
        }
        let answers = known
            .into_iter()
            .map(|use_case| {
                let answer = match use_case.claim() {
                    Claim::Unclaimed => Answer::Clear,
                    Claim::Held(holder) if holder == slug => Answer::Clear,
                    Claim::Held(holder) => Answer::Reserved {
                        holder: holder.clone(),
                    },
                    Claim::Ambiguous(delivered_by) => Answer::Ambiguous {
                        delivered_by: delivered_by.clone(),
                    },
                };
                (use_case.reference().clone(), answer)
            })
            .collect();
        Self::Checked(answers)
    }

    /// Unknown before reserved before ambiguous; clear only when every
    /// requested binding is.
    pub fn verdict(&self) -> Verdict {
        let Self::Checked(answers) = self else {
            return Verdict::Unknown;
        };
        let any = |f: fn(&Answer) -> bool| answers.iter().any(|(_, a)| f(a));
        if any(|a| matches!(a, Answer::Reserved { .. })) {
            Verdict::Reserved
        } else if any(|a| matches!(a, Answer::Ambiguous { .. })) {
            Verdict::Ambiguous
        } else {
            Verdict::Clear
        }
    }
}
