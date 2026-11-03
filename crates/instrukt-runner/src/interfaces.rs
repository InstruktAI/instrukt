//! The interfaces the runner declares for work and workflow
//! (`docs/local/spec/domain/runner.md`, "Context map position";
//! `docs/local/design/architecture/overview.md`, "Modules and crates").
//!
//! Todo dispatch reads work and workflow only through these traits, which
//! preserves `instrukt-todos` → `instrukt-runner` → `instrukt-core`. Work's
//! command contract covers every runner mutation; work owns their
//! validation and persistence.
//!
//! | Trait | Spec it serves | Implemented by | Used by |
//! | --- | --- | --- | --- |
//! | [`Pickup`] | `work-dispatch` "Todo pickup", `task-timing` | work (`instrukt_todos::work`) | runner dispatch |
//! | [`Claims`] | `work-dispatch` "Execution contract" | work | runner dispatch; workflow's `todo prepare\|work` |
//! | [`SessionBindings`] | `session-continuity` "Identity and reuse" | work | runner dispatch and sessions facade |
//! | [`Reflection`] | `work-dispatch` "Execution contract", `github-record` | work | runner tick |
//! | [`TaskFirings`] | `task-timing` "Task and schedule" | work | runner task timing |
//! | [`AutomaticHolds`] | `dependencies-and-holds` "Session bindings, claims and holds" | work | runner dispatch and anomalies |
//! | [`RetentionPurge`] | `todos` "Parking, withdrawal and delivery" | work | runner tick |
//! | [`WorkflowAssignments`] | `workflow` "Published contracts", `work-dispatch` "Todo pickup" | workflow (`instrukt_todos::workflow`) | runner dispatch |
//!
//! The CLI wires them into [`RunnerWiring`] (`crates/instrukt-cli/src/
//! wiring.rs`); the runner checks `modules.todos` per project before using
//! them. The event-log
//! append work's finalize-ready stamp uses is the other direction: the
//! runner implements [`crate::events::EventLog`] and the CLI hands it to
//! work.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;

use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef, SnippetId, TodoSlug};
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

/// A project the runner considers: its slug and checkout path, as recorded
/// in publication state (`work-dispatch`, "Execution contract").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectRef {
    pub slug: ProjectSlug,
    /// The project's primary worktree.
    pub root: PathBuf,
}

/// A todo's kind: the work category selecting its preparation and
/// completion path (`work` domain, "Kind"; `todos`, "Kinds"). The
/// `runner.autostart` map is keyed by these names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TodoKind {
    Feature,
    Change,
    Bug,
    Task,
}

impl TodoKind {
    pub const ALL: [TodoKind; 4] = [
        TodoKind::Feature,
        TodoKind::Change,
        TodoKind::Bug,
        TodoKind::Task,
    ];

    /// The kind's name: `feature`, `change`, `bug`, `task`.
    pub fn as_str(self) -> &'static str {
        match self {
            TodoKind::Feature => "feature",
            TodoKind::Change => "change",
            TodoKind::Bug => "bug",
            TodoKind::Task => "task",
        }
    }
}

impl std::fmt::Display for TodoKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A name that is not one of the four kinds.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown kind `{0}`: expected feature, change, bug or task")]
pub struct UnknownKind(pub String);

impl FromStr for TodoKind {
    type Err = UnknownKind;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        TodoKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == s)
            .ok_or_else(|| UnknownKind(s.to_owned()))
    }
}

/// A work interface call failed. Work's refusals name their door, as the
/// work specs give it.
#[derive(Debug, thiserror::Error)]
pub enum WorkInterfaceError {
    /// The project has no live todo with this slug.
    #[error("{project} has no todo `{todo}`")]
    UnknownTodo {
        project: ProjectSlug,
        todo: TodoSlug,
    },
    /// The item is conflicted with a newer remote revision; commands on it
    /// refuse until it is refreshed (`github-record`, "Writer and
    /// convergence").
    #[error(
        "todo `{todo}` is conflicted with GitHub; refresh it with `instrukt todo refresh {todo}`"
    )]
    Conflicted { todo: TodoSlug },
    /// Work refused the write for a reason its contract states (a hold,
    /// a delivered item, an invalid resulting record).
    #[error("{reason}")]
    Refused { reason: String },
    /// The local work record could not be read or written.
    #[error("work record: {message}")]
    Store { message: String },
}

// ---------------------------------------------------------------------
// Pickup
// ---------------------------------------------------------------------

/// One todo work offers for pickup (`work-dispatch`, "Todo pickup").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PickupCandidate {
    pub todo: TodoSlug,
    pub kind: TodoKind,
    /// A task's schedule and its last firing; `None` for other kinds and
    /// for a task without a schedule.
    pub timing: Option<TaskTiming>,
    /// A task's optional agent/model preference, consulted before the
    /// procedure's `dispatch` order and global `agents.order`
    /// (`work-dispatch`, "Agent selection and intent checks").
    pub agent: Option<AgentPreference>,
}

/// The agent a task prefers, set with `--agent <name> [--thinking …]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPreference {
    pub agent: String,
    pub thinking: Option<Thinking>,
}

/// The thinking mode of an agent preference (`--thinking slow|med|fast`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Thinking {
    Slow,
    Med,
    Fast,
}

/// Which todos work offers for pickup.
pub trait Pickup: Send + Sync {
    /// The project's todos of the given kinds that the record admits for
    /// pickup, in work's readiness and dependency order: active (not
    /// iceboxed, not a structural holder, not delivered or removed), no
    /// dispatch hold, no execution claim held. Lane readiness (which flow
    /// is owed and whether prerequisites satisfy it) is workflow's, through
    /// [`WorkflowAssignments`]. Serves `work-dispatch`, "Todo pickup", and
    /// `dependencies-and-holds`, "Prerequisites".
    fn pickup_candidates(
        &self,
        project: &ProjectRef,
        kinds: &[TodoKind],
    ) -> Result<Vec<PickupCandidate>, WorkInterfaceError>;
}

// ---------------------------------------------------------------------
// Claims
// ---------------------------------------------------------------------

/// Who records an execution claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claimant {
    /// The runner on automatic dispatch, before any session starts.
    Runner,
    /// A session picking the todo up deliberately with
    /// `instrukt todo prepare|work`.
    Session(SessionRef),
}

/// The outcome of recording an execution claim and reflecting it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClaimOutcome {
    /// The claim is recorded and reflected: this computer holds it.
    Claimed,
    /// The claimant already held it; nothing changed.
    AlreadyHeld,
    /// Reflection met a newer remote revision: another computer claimed
    /// it first. The local change is not reflected; leave the item.
    ClaimedElsewhere,
    /// The claim is recorded locally but could not be reflected (GitHub
    /// unreachable or rate limited); it is pending reflection.
    NotReflected,
}

/// Recording the item-level execution claim (`work-dispatch`, "Execution
/// contract"; `dependencies-and-holds`, "Session bindings, claims and
/// holds"). Release is `instrukt todo update <slug> --unclaim`, the
/// session's own command, not a runner write.
pub trait Claims: Send + Sync {
    /// Records the claim on `todo` for `claimant` and reflects it to
    /// GitHub before returning.
    fn record_claim(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        claimant: &Claimant,
    ) -> Result<ClaimOutcome, WorkInterfaceError>;
}

// ---------------------------------------------------------------------
// Session bindings
// ---------------------------------------------------------------------

/// A started session recorded per todo and role: its runtime and
/// runtime-native id, and the worktree path the runtime returned, retained
/// after the session ends (`work` domain, "Todo" aggregate;
/// `session-continuity`, "Runtime worktree binding").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionBinding {
    pub session: SessionRef,
    pub worktree: Option<PathBuf>,
}

/// Reading and writing per-todo, per-role session bindings
/// (`session-continuity`, "Identity and reuse"). Reuse is a lookup of the
/// binding followed by `alive`; brief similarity is not identity.
pub trait SessionBindings: Send + Sync {
    /// The binding recorded for `todo` and `role`, if any.
    fn session_binding(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        role: &RoleName,
    ) -> Result<Option<SessionBinding>, WorkInterfaceError>;

    /// Records (or replaces) the binding for `todo` and `role` after a
    /// session starts, or when the runtime returns its worktree path.
    fn bind_session(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        role: &RoleName,
        binding: &SessionBinding,
    ) -> Result<(), WorkInterfaceError>;
}

// ---------------------------------------------------------------------
// Reflection
// ---------------------------------------------------------------------

/// Whether GitHub answered during a reflection pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubReach {
    Reachable,
    Unreachable,
    /// The provider deferred requests until the given time.
    RateLimited {
        until: Option<Timestamp>,
    },
}

/// What one reflection pass left behind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectionReport {
    pub reach: GitHubReach,
    /// Local items whose writes still await reflection.
    pub pending: Vec<TodoSlug>,
    /// Local items marked conflicted by this or an earlier pass.
    pub conflicted: Vec<TodoSlug>,
}

/// The tick's reflection pass (`work-dispatch`, "Execution contract";
/// `github-record`, "Local-leading pull-through" and "Writer and
/// convergence").
pub trait Reflection: Send + Sync {
    /// Pulls GitHub changes for the project's local items with conditional
    /// requests and flushes pending reflection, under the per-computer
    /// writer lock. An unreachable GitHub is a report, not an error.
    fn sync_reflection(&self, project: &ProjectRef)
    -> Result<ReflectionReport, WorkInterfaceError>;
}

// ---------------------------------------------------------------------
// Task timing
// ---------------------------------------------------------------------

/// A task's recorded schedule (`todos`, "Task schedule"; `task-timing`,
/// "Task and schedule").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskSchedule {
    /// One-off: an ISO-8601 instant with an offset.
    At(Timestamp),
    /// Recurring: five cron fields in the machine's local time zone.
    Cron(CronExpr),
}

/// A schedule value that is neither an instant nor `cron:<five fields>`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "invalid schedule `{value}`: expected an ISO-8601 instant with an offset or `cron:<minute hour day month weekday>`"
)]
pub struct ScheduleError {
    pub value: String,
}

impl TaskSchedule {
    /// Parses the `--schedule <value>` form: an ISO-8601 instant with an
    /// offset for `at`, or `cron:<minute hour day month weekday>`. The
    /// empty value that clears a schedule is the caller's to handle.
    pub fn parse(value: &str) -> Result<Self, ScheduleError> {
        let refused = || ScheduleError {
            value: value.to_owned(),
        };
        match value.strip_prefix("cron:") {
            Some(fields) => CronExpr::parse(fields)
                .map(TaskSchedule::Cron)
                .map_err(|_| refused()),
            None => value
                .parse::<Timestamp>()
                .map(TaskSchedule::At)
                .map_err(|_| refused()),
        }
    }
}

/// Five cron fields: minute, hour, day of month, month, day of week. The
/// type guarantees the shape (five fields of cron characters); the
/// runner's due-time logic owns their meaning and ranges.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CronExpr(String);

impl CronExpr {
    /// Validates the shape of `fields` and normalizes the whitespace
    /// between them to one space.
    pub fn parse(fields: &str) -> Result<Self, ScheduleError> {
        let parts: Vec<&str> = fields.split_whitespace().collect();
        let well_formed = parts.len() == 5
            && parts.iter().all(|part| {
                part.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '*' | ',' | '-' | '/'))
            });
        if well_formed {
            Ok(Self(parts.join(" ")))
        } else {
            Err(ScheduleError {
                value: format!("cron:{fields}"),
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for CronExpr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for CronExpr {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for CronExpr {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// A task's missed-run policy (`catch_up`), `once` by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CatchUp {
    /// Missed occurrences result in one catch-up run.
    #[default]
    Once,
    /// Missed occurrences are skipped.
    Skip,
}

/// What timing needs of a task: its schedule, missed-run policy and its
/// last firing (`task-timing`, "Task and schedule").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskTiming {
    pub schedule: TaskSchedule,
    pub catch_up: CatchUp,
    /// The latest `--task-firing-started` stamp; `None` when the task
    /// never fired.
    pub last_fired: Option<Firing>,
}

/// One firing of a task as its stamps record it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Firing {
    /// The scheduled occurrence the firing executed.
    pub occurrence: Timestamp,
    /// When the firing started.
    pub started: Timestamp,
    /// The recorded `--task-outcome`; `None` while the firing is in flight
    /// or its execution never recorded one.
    pub outcome: Option<TaskOutcome>,
}

/// A recorded task execution result (`--task-outcome success|error`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "result")]
pub enum TaskOutcome {
    Success,
    Error { reason: Option<String> },
}

/// The runner's last-fired accounting (`task-timing`, "Task and schedule":
/// "Last-fired accounting is a stamp on the task record").
pub trait TaskFirings: Send + Sync {
    /// Records the `--task-firing-started` stamp for `occurrence` of task
    /// `todo`, attributed to the session that was started for it.
    fn record_firing_started(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        occurrence: Timestamp,
        session: &SessionRef,
    ) -> Result<(), WorkInterfaceError>;
}

// ---------------------------------------------------------------------
// Automatic holds
// ---------------------------------------------------------------------

/// The cause an automatic hold records (`dependencies-and-holds`, "Session
/// bindings, claims and holds": "An automatic hold records the cause that
/// produced it, such as a failed task").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldCause {
    /// A task execution failed.
    TaskFailed { reason: String },
    /// An anomaly the runner raised for this todo.
    Anomaly { reason: String },
}

/// Pausing automatic pickup with an attributed cause.
pub trait AutomaticHolds: Send + Sync {
    /// Sets an automatic dispatch hold on `todo` with `cause`. A hold
    /// pauses future automatic pickup; it terminates nothing running.
    fn set_automatic_hold(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        cause: &HoldCause,
    ) -> Result<(), WorkInterfaceError>;
}

// ---------------------------------------------------------------------
// Retention purge
// ---------------------------------------------------------------------

/// Purging removed work whose recovery window expired (`todos`, "Parking,
/// withdrawal and delivery": "The next runner tick or todo command after
/// the window expires purges the retained local record").
pub trait RetentionPurge: Send + Sync {
    /// Purges the project's retained local records whose seven-day window
    /// has expired; returns the purged slugs.
    fn purge_expired(&self, project: &ProjectRef) -> Result<Vec<TodoSlug>, WorkInterfaceError>;
}

// ---------------------------------------------------------------------
// Workflow
// ---------------------------------------------------------------------

/// A workflow interface call failed.
#[derive(Debug, thiserror::Error)]
pub enum WorkflowInterfaceError {
    /// The selected flavor could not be loaded from the published domain.
    #[error("workflow flavor: {message}")]
    Flavor { message: String },
    /// Evaluation was invalid (an unknown flow, a revisited state).
    #[error("workflow evaluation of `{todo}`: {message}")]
    Evaluation { todo: TodoSlug, message: String },
    /// A command, template or effect could not be rendered.
    #[error("workflow rendering for `{todo}`: {message}")]
    Render { todo: TodoSlug, message: String },
    /// The work record could not supply the evidence.
    #[error(transparent)]
    Work(#[from] WorkInterfaceError),
}

/// The lane workflow derives for a todo (`work-dispatch`, "Todo pickup";
/// `preparation`, "Kind routing").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lane {
    /// The `prepare` flow is owed; dispatch its orchestrator.
    Prepare,
    /// The `work` flow is owed; dispatch its orchestrator.
    Work,
    /// A prepared, authorized task: it runs as a `run-task` dispatch when
    /// task timing finds it due (`task-timing`, "Task and schedule").
    TaskRun,
}

/// Who an obligation's action belongs to, as the engine reports it
/// (`workflow-definition`, "Flavor source"). The runner resolves a
/// procedure's role from the docs index (`instrukt_core::index`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Responsibility {
    /// The action's command names this procedure; the action belongs to
    /// that procedure's `role`.
    Procedure(SnippetId),
    /// The command names no procedure; the action is a step of the flow's
    /// orchestrator, executed by the session holding the claim.
    Orchestrator(RoleName),
}

/// An effect owed along the selected walk, rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignedEffect {
    pub name: String,
    pub command: String,
}

/// The obligation at the derived state: the selected action, rendered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignedObligation {
    pub action: String,
    pub responsible: Responsibility,
    /// The action's command with placeholders filled.
    pub command: String,
    /// The action's rendered template: the instructions.
    pub brief: String,
    /// Effects still owed, in listed order.
    pub effects: Vec<AssignedEffect>,
}

/// Workflow's current assignment for a todo, derived from the record on
/// this call (`workflow` domain, "Published contracts").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub lane: Lane,
    /// The orchestrator role the lane's flow declares; `None` for
    /// [`Lane::TaskRun`].
    pub orchestrator: Option<RoleName>,
    /// The state where derivation stopped.
    pub state: String,
    /// The obligation at that state; `None` when no transition was
    /// selected (nothing owed now).
    pub obligation: Option<AssignedObligation>,
}

/// Reading workflow's derived state for dispatch.
pub trait WorkflowAssignments: Send + Sync {
    /// The todo's current assignment, or `None` when no lane is owed
    /// (delivered, removed, or a prerequisite blocks every lane). Pure
    /// derivation: it records nothing and appends no event. Serves
    /// `workflow` domain "Published contracts" and `work-dispatch` "Todo
    /// pickup".
    fn assignment(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
    ) -> Result<Option<Assignment>, WorkflowInterfaceError>;
}

// ---------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------

/// Work's implementations of the runner-declared work interfaces.
#[derive(Clone)]
pub struct WorkPorts {
    pub pickup: Arc<dyn Pickup>,
    pub claims: Arc<dyn Claims>,
    pub bindings: Arc<dyn SessionBindings>,
    pub reflection: Arc<dyn Reflection>,
    pub firings: Arc<dyn TaskFirings>,
    pub holds: Arc<dyn AutomaticHolds>,
    pub retention: Arc<dyn RetentionPurge>,
}

impl std::fmt::Debug for WorkPorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkPorts").finish_non_exhaustive()
    }
}

/// What the CLI wired into the runner for this invocation. The
/// implementations serve any project; the runner itself decides per
/// project whether `modules.todos` is on (`work-dispatch`, "Execution
/// contract"). A `None` slot means the implementation is not available in
/// this build, and the runner dispatches no todos.
#[derive(Clone, Default)]
pub struct RunnerWiring {
    pub work: Option<WorkPorts>,
    pub workflow: Option<Arc<dyn WorkflowAssignments>>,
}

impl std::fmt::Debug for RunnerWiring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerWiring")
            .field("work", &self.work.is_some())
            .field("workflow", &self.workflow.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedules_parse_an_instant_with_offset_or_five_cron_fields() {
        let at = TaskSchedule::parse("2026-11-03T10:00:00+01:00").unwrap();
        assert_eq!(
            at,
            TaskSchedule::At("2026-11-03T09:00:00Z".parse().unwrap())
        );
        let cron = TaskSchedule::parse("cron:*/5  9-17 * * MON-FRI").unwrap();
        assert_eq!(
            cron,
            TaskSchedule::Cron(CronExpr("*/5 9-17 * * MON-FRI".into()))
        );
        for bad in [
            "",
            "tomorrow",
            "2026-11-03T10:00:00",
            "cron:* * * *",
            "cron:* * * * * *",
            "cron:@daily * * * *",
        ] {
            assert!(
                TaskSchedule::parse(bad).is_err(),
                "{bad:?} should be refused"
            );
        }
    }

    #[test]
    fn kinds_round_trip_their_names() {
        for kind in TodoKind::ALL {
            assert_eq!(kind.as_str().parse::<TodoKind>(), Ok(kind));
        }
        assert!("chore".parse::<TodoKind>().is_err());
    }
}
