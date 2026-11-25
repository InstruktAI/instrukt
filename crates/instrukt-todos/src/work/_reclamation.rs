//! Work's part in worktree reclamation (`project/instrukt/spec/domain/work`,
//! "Work → runner (worktree reclamation)"): a section of the work record in
//! which no role attempt is open on a todo's worktree and none opens.
//! Opening an attempt is a write under the same writer lock, so inside the
//! section none can; the guard reads the record, never the caller's rank
//! (`general/principle/trust-and-guardrails`).

use instrukt_runner::interfaces::{
    AssignmentKind, ProjectRef, SectionAdmission, SectionOutcome, TodoIdentity, WorkInterfaceError,
    WorktreeHolder, WorktreeReclamation,
};

use super::_model::{Binding, TodoId, TodoRecord};
use super::_store::Store;
use super::Work;

impl WorktreeReclamation for Work {
    fn attempt_free(
        &self,
        project: &ProjectRef,
        todo: &TodoIdentity,
        admission: &SectionAdmission,
        action: &mut dyn FnMut(),
    ) -> Result<SectionOutcome, WorkInterfaceError> {
        let Some(id) = TodoId::parse(todo.as_str()) else {
            return Ok(SectionOutcome::NotRecorded);
        };
        Ok(
            Store::new(self.home(), &project.slug).hold_record(&id, |record| {
                let Some(record) = record else {
                    return SectionOutcome::NotRecorded;
                };
                match refusal(record, admission) {
                    Some(refused) => refused,
                    None => {
                        action();
                        SectionOutcome::Ran
                    }
                }
            })?,
        )
    }
}

/// Why `admission` admits no section on `record`'s worktree; `None` when it
/// admits one. The runner is refused while any role attempt is open there
/// or the record moved past the revision it observed; a session is refused
/// only while another session holds the worktree: a role attempt open there
/// that is not its own, or, with none open, a claim it does not hold.
fn refusal(record: &TodoRecord, admission: &SectionAdmission) -> Option<SectionOutcome> {
    let mut open = record
        .bindings
        .iter()
        .filter(|binding| open_attempt(binding));
    match admission {
        SectionAdmission::Runner { revision } => match open.next() {
            Some(binding) => Some(SectionOutcome::Held(holder(binding))),
            None => (record.revision != *revision).then_some(SectionOutcome::RecordChanged),
        },
        SectionAdmission::Session(caller) => {
            let open: Vec<&Binding> = open.collect();
            if let Some(other) = open
                .iter()
                .find(|binding| binding.session.as_ref() != Some(caller))
            {
                return Some(SectionOutcome::Held(holder(other)));
            }
            match &record.claim {
                Some(claim) if open.is_empty() && claim.session.as_ref() != Some(caller) => {
                    Some(SectionOutcome::Held(WorktreeHolder::Claim {
                        session: claim.session.clone(),
                    }))
                }
                _ => None,
            }
        }
    }
}

/// Whether `binding` is a role attempt open on the todo's worktree: an
/// unreleased workspace or child assignment. A driver binding is not one.
fn open_attempt(binding: &Binding) -> bool {
    matches!(
        binding.attempt.assignment,
        AssignmentKind::Workspace | AssignmentKind::Child
    ) && binding.attempt.released_at.is_none()
}

fn holder(binding: &Binding) -> WorktreeHolder {
    WorktreeHolder::Attempt {
        role: binding.role.clone(),
        attempt: binding.attempt.number,
        session: binding.session.clone(),
    }
}
