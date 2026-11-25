//! The interfaces the runner declares for work and workflow
//! (`docs/project/spec/domain/runner.md`, "Context map position";
//! `docs/project/design/architecture/overview.md`, "Modules and crates").
//!
//! Todo dispatch reads work and workflow only through these traits, which
//! preserves `instrukt-todos` → `instrukt-runner` → `instrukt-core`. Work's
//! command contract covers every runner mutation; work owns their
//! validation and persistence.
//!
//! | Trait | Spec it serves | Implemented by | Used by |
//! | --- | --- | --- | --- |
//! | [`Pickup`] | `work-dispatch` "Todo pickup", `task-timing` | work (`instrukt_todos::work`) | runner dispatch |
//! | [`Claims`] | `work-dispatch` "Execution contract" | work | runner dispatch; workflow's `todo work` |
//! | [`SessionBindings`] | `session-continuity` "Identity and reuse" | work | runner dispatch and sessions facade |
//! | [`Reflection`] | `work-dispatch` "Execution contract", `github-record` | work | runner tick |
//! | [`TaskFirings`] | `task-timing` "Task and schedule" | work | runner task timing |
//! | [`AutomaticHolds`] | `dependencies-and-holds` "Session bindings, claims and holds" | work | runner dispatch and anomalies |
//! | [`RetentionPurge`] | `todos` "Parking, withdrawal and delivery" | work | runner tick |
//! | [`HeadVerdicts`] | `requested-review` "Kinds of ask", "Admission", "Requested review records" | work (`instrukt_todos::work::review`) | `instrukt review start` |
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

use instrukt_core::ids::{ProjectSlug, RoleName, SessionRef, TodoId, TodoSlug};
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
    /// The runner's own claim on this computer already holds the todo and
    /// no session is bound to it yet: either pending reflection (GitHub was
    /// unreachable when it was recorded) or reflected since by another
    /// command. Claiming it again answers `AlreadyHeld` once reflected,
    /// `NotReflected` while GitHub still cannot be reached (ruling D).
    pub claimed_here: bool,
}

/// The profile a task prefers, set with `--agent <name> [--tier …]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPreference {
    pub agent: instrukt_core::ids::ProfileName,
    pub tier: Option<Tier>,
}

pub use instrukt_core::config::Tier;

/// Which todos work offers for pickup.
pub trait Pickup: Send + Sync {
    /// The project's todos of the given kinds that the record admits for
    /// pickup, in work's readiness and dependency order: active (not
    /// iceboxed, not a structural holder, not delivered or removed), no
    /// dispatch hold, no execution claim held. Lane readiness (which flow
    /// is owed and whether prerequisites satisfy it) is workflow's, through
    /// [`WorkflowAssignments`]. Serves `work-dispatch`, "Todo pickup", and
    /// `dependencies-and-holds`, "Prerequisites". Reads the tick's one
    /// pickup listing of the project from GitHub, whose result replaces the
    /// project's listing copies (`github-record`, "Interest-driven
    /// refresh").
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
    /// `instrukt todo work`.
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

    /// Releases `claimant`'s claim on `todo` and reflects the release
    /// (`work-dispatch`, "Agent selection": an exhausted agent order
    /// releases the runner's own claim). Only the holder on this computer
    /// may release; any other claimant is refused.
    fn release_claim(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        claimant: &Claimant,
    ) -> Result<(), WorkInterfaceError>;
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
    /// Work-owned native placement title; automatic starts retain theirs with the runner request.
    pub title: Option<String>,
    pub key: String,
    pub session: SessionRef,
    pub worktree: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentKind {
    #[default]
    Unspecified,
    Driver,
    Task,
    Workspace,
    Child,
}

pub enum DispatchReservation {
    Existing(SessionBinding),
    Start {
        key: String,

        worktree: Option<crate::runtime::WorktreeRequest>,
    },
}

pub struct AcceptedDispatch {
    pub role: RoleName,
    pub key: String,
    pub binding: SessionBinding,
    pub occurrence: Option<Timestamp>,
}

/// Reading and writing per-todo, per-role session bindings
/// (`session-continuity`, "Identity and reuse"). Reuse is a lookup of the
/// binding followed by `alive`; brief similarity is not identity.
pub trait SessionBindings: Send + Sync {
    /// Retire a freshly reserved assignment after selection exhausted before runtime I/O.
    fn release_unstarted(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        key: &str,
    ) -> Result<(), WorkInterfaceError>;

    /// Allocate and retain the assignment before any runtime command.
    fn reserve_dispatch(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        role: &RoleName,
        occurrence: Option<Timestamp>,
        lane: &Lane,
    ) -> Result<DispatchReservation, WorkInterfaceError>;
    /// The binding recorded for `todo` and `role`, if any.
    fn session_binding(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        role: &RoleName,
    ) -> Result<Option<SessionBinding>, WorkInterfaceError>;

    /// Check that work still owns the stopped assignment before starting a replacement.
    fn check_dispatch_replacement(
        &self,
        project: &ProjectRef,
        key: &str,
        stopped: &SessionRef,
    ) -> Result<(), WorkInterfaceError>;

    /// Commit an operator-confirmed replacement and its runtime acceptance
    /// together with the current work binding and claim.
    fn replace_dispatch(
        &self,
        project: &ProjectRef,
        binding: &SessionBinding,
        stopped: &SessionRef,
        accept_runtime: &mut dyn FnMut(
            &instrukt_core::state_db::rusqlite::Transaction<'_>,
        ) -> Result<(), crate::state::StateError>,
    ) -> Result<(), WorkInterfaceError>;

    /// Retry reflection after local replacement acceptance has committed.
    fn reflect_dispatch_replacement(
        &self,
        project: &ProjectRef,
        key: &str,
    ) -> Result<(), WorkInterfaceError>;

    /// Commit the accepted assignment and optional occurrence stamp together.
    fn record_dispatch(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
        dispatch: &AcceptedDispatch,
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
    /// Polls GitHub, with conditional requests, for the todos this
    /// computer holds claims on and the prerequisites that claimed work is
    /// blocked on (`github-record`, "Interest-driven refresh"), then flushes
    /// pending reflection, under the per-computer writer lock. The tick's
    /// pickup listing is [`Pickup::pickup_candidates`]. An unreachable
    /// GitHub is a report, not an error.
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
            })
            && crate::timing::CronSpec::parse(fields).is_ok();
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
    TaskFailed {
        reason: String,
        occurrence: Timestamp,
        session: Option<SessionRef>,
    },
    /// An anomaly the runner raised for this todo.
    Anomaly { reason: String },
}

/// Pausing automatic pickup with an attributed cause.
pub trait AutomaticHolds: Send + Sync {
    /// Sets an automatic dispatch hold on `todo` with `cause`. A hold
    /// pauses future automatic pickup; it terminates nothing running.
    /// A failed firing's notification record commits with its hold in
    /// one state-database transaction, before reflection to GitHub.
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
// Worktree reclamation
// ---------------------------------------------------------------------

/// Who asks for a section of the work record in which no role attempt
/// opens on a todo's worktree, and what admits it there
/// (`worktree-reclamation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionAdmission {
    /// The runner: admitted while no role attempt is open on the worktree
    /// and the record is still at the revision the runner observed before
    /// its git checks.
    Runner { revision: u64 },
    /// A session the todo's assignment binds: admitted unless another
    /// session holds the worktree.
    Session(SessionRef),
}

/// A session that holds a todo's worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeHolder {
    /// A role attempt open on the worktree.
    Attempt {
        role: RoleName,
        attempt: u64,
        session: Option<SessionRef>,
    },
    /// The todo's execution claim, with no role attempt open.
    Claim { session: Option<SessionRef> },
}

impl std::fmt::Display for WorktreeHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let by = |session: &Option<SessionRef>| {
            session
                .as_ref()
                .map_or_else(|| "no recorded session".to_owned(), ToString::to_string)
        };
        match self {
            Self::Attempt {
                role,
                attempt,
                session,
            } => write!(f, "the role attempt {role}/{attempt} ({})", by(session)),
            Self::Claim { session } => write!(f, "the execution claim ({})", by(session)),
        }
    }
}

/// What asking for an attempt-free section came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SectionOutcome {
    /// The action ran inside the section.
    Ran,
    /// The worktree is held, so the action did not run.
    Held(WorktreeHolder),
    /// The record changed after the revision the runner observed, so the
    /// action did not run.
    RecordChanged,
    /// The project's record holds no todo with that identity.
    NotRecorded,
}

/// A working resource of a delivered todo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveredResource {
    Worktree(PathBuf),
    LocalBranch(String),
    OriginBranch(String),
}

impl std::fmt::Display for DeliveredResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Worktree(path) => write!(f, "the worktree {}", path.display()),
            Self::LocalBranch(name) => write!(f, "the local branch {name}"),
            Self::OriginBranch(name) => write!(f, "the branch {name} on origin"),
        }
    }
}

/// What a delivered release did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveredRelease {
    /// Another delivered release runs on this computer: nothing was done
    /// and the cleanup stays owed.
    Busy,
    /// What was released, and what stays with its cause.
    Done {
        released: Vec<DeliveredResource>,
        kept: Vec<(DeliveredResource, String)>,
    },
}

/// Work's part in worktree reclamation (`work` domain, "Work → runner
/// (worktree reclamation)"): a section in which no role attempt opens, and
/// the release of a delivered todo's worktree and work branch.
pub trait WorktreeReclamation: Send + Sync {
    /// Runs `action` inside one write transaction of the work record, under
    /// the writer lock, in which no role attempt opens on `todo`'s
    /// worktree, when `admission` admits it; the section writes nothing,
    /// and `action` is all it holds the lock for.
    fn attempt_free(
        &self,
        project: &ProjectRef,
        todo: &TodoId,
        admission: &SectionAdmission,
        action: &mut dyn FnMut(),
    ) -> Result<SectionOutcome, WorkInterfaceError>;

    /// Releases what workflow attributes to the delivered todo `delivered`
    /// on this computer, and nothing else: its worktree, then its work
    /// branch locally and on origin (`todos`, "Parking, withdrawal and
    /// delivery"). A worktree or branch workflow attributes to a live todo
    /// is not among them.
    fn release_delivered(
        &self,
        project: &ProjectRef,
        delivered: &TodoWorktree,
    ) -> Result<DeliveredRelease, WorkInterfaceError>;

    /// Whether `project`'s work record on this computer holds the todo
    /// `todo`.
    fn holds(&self, project: &ProjectRef, todo: &TodoId) -> Result<bool, WorkInterfaceError>;
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
    /// Observing the repository or facts failed.
    #[error("workflow observation for `{todo}`: {message}")]
    Observation { todo: TodoSlug, message: String },
    /// Reading what git lists of a project's worktrees and branches failed.
    #[error("git's worktrees and branches of {project}: {message}")]
    Listing {
        project: ProjectSlug,
        message: String,
    },
    /// The work record could not supply the evidence.
    #[error(transparent)]
    Work(#[from] WorkInterfaceError),
}

/// The dispatch lane for a todo (`work-dispatch`, "Todo pickup").
///
/// `Prepare` is retained for deserializing legacy bindings and envelopes;
/// all new todo orchestrator dispatches use `Work`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lane {
    /// Legacy preparation lane (deserialization compatibility only).
    Prepare,
    /// Unified todo orchestrator lane (`todo-orchestrator`).
    Work,
    /// An authorized task ready to run as `task-runner` when due
    /// (`task-timing`, "Task and schedule").
    TaskRun,
}

/// Workflow's current dispatch assignment for a todo, derived from current
/// facts (`workflow` domain, "Published contracts").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub lane: Lane,
    /// The flavor's orchestrator role for `Lane::Work`; `None` for
    /// [`Lane::TaskRun`].
    pub orchestrator: Option<RoleName>,
    /// A factual progress fingerprint over record & workspace state, used by
    /// the runner to detect whether a dispatched session made progress.
    pub state: String,
    /// The session holding the todo's execution claim, relayed from work;
    /// `None` while no session holds it.
    pub holder: Option<SessionRef>,
    /// Whether the todo waits on a person (a hold, or an approval gate
    /// that waits for a human), as work decides it.
    pub waits_on_person: bool,
}

/// Reading workflow's derived state for dispatch.
pub trait WorkflowAssignments: Send + Sync {
    /// The todo's current assignment, or `None` when no lane is owed: the
    /// todo is not active, it is held while no session holds its claim, a
    /// prerequisite is undelivered, or no claim is held while it waits on
    /// a human approval or on integration of a finalized hand-off. Pure
    /// derivation: it records nothing and appends no event. Serves
    /// `workflow` domain "Published contracts" and `work-dispatch` "Todo
    /// pickup".
    fn assignment(
        &self,
        project: &ProjectRef,
        todo: &TodoSlug,
    ) -> Result<Option<Assignment>, WorkflowInterfaceError>;
}

/// What git still lists on this computer of a delivered todo's working
/// resources: the cleanup it still owes (`delivery-cycle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeliveredRemains {
    pub worktree: bool,
    pub local_branch: bool,
    pub origin_branch: bool,
}

impl DeliveredRemains {
    /// Whether any of them remains.
    pub fn any(self) -> bool {
        self.worktree || self.local_branch || self.origin_branch
    }
}

/// A todo worktree's standing on this computer, as workflow derives it
/// (`workflow` domain, "Worktree standing").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeStanding {
    /// Its todo's rounds of building, reviewing and fixing are under way.
    UnderWay,
    /// Its todo is parked or held, its candidate waits for integration, or
    /// its work has gone back to preparation.
    Idle,
    /// Its todo is delivered, with what git still lists of it.
    Delivered(DeliveredRemains),
}

/// One todo whose worktree or work branch git lists for a project on this
/// computer, with its standing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoWorktree {
    pub todo: TodoSlug,
    pub identity: TodoId,
    /// The record revision the standing was derived at: advanced by every
    /// accepted change to the todo's record.
    pub revision: u64,
    /// The worktree git lists for it, when it has one.
    pub worktree: Option<PathBuf>,
    /// Its work branch, when its record names one.
    pub branch: Option<String>,
    pub standing: WorktreeStanding,
}

/// Reading workflow's worktree standing for reclamation.
pub trait WorktreeStandings: Send + Sync {
    /// Each todo whose worktree, local work branch or work branch on
    /// `origin` git lists for `project` on this computer, with its standing.
    /// Pure derivation from the work record this computer holds and git:
    /// it records nothing and makes no GitHub request
    /// (`worktree-reclamation`, "Idle worktrees").
    fn worktree_standings(
        &self,
        project: &ProjectRef,
    ) -> Result<Vec<TodoWorktree>, WorkflowInterfaceError>;
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
    pub reclamation: Arc<dyn WorktreeReclamation>,
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
    pub github_credentials: Option<Arc<dyn GitHubCredentials>>,
    /// Work's answers for automatic review, wired whether or not a
    /// project's todos module is on.
    pub review: Option<Arc<dyn AutomaticReview>>,
    pub work: Option<WorkPorts>,
    pub workflow: Option<Arc<dyn WorkflowAssignments>>,
    /// Workflow's worktree standing, read for worktree reclamation.
    pub standings: Option<Arc<dyn WorktreeStandings>>,
    /// The company the runner hosts: its conversations. `None` hosts none.
    pub company: Option<Arc<dyn CompanyPorts>>,
}

/// Why a company port could not do what was asked, as the company words it.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct CompanyError(pub String);

/// What caused a pass of the company conversation relay.
#[derive(Clone, Copy)]
pub enum CompanyRelayPass {
    /// Startup, intake or a native source change.
    CatchUp,
    /// The retry timer, which serves only conversations with work remaining.
    Retry,
}

/// The company as the runner hosts it: its connections, the conversation
/// work the relay serves beside links, and its section of the inspection
/// report. The runner names no company type; the CLI wires the
/// implementation.
pub trait CompanyPorts: Send + Sync {
    /// Retires old offline coverage when the runner starts, even with no
    /// admitted connectors.
    fn maintain_read_back(
        &self,
        _home: &instrukt_core::home::Home,
        _now: jiff::Timestamp,
    ) -> Result<(), CompanyError> {
        Ok(())
    }
    /// Whether `admitted` hosts any company's connector.
    fn demand(&self, admitted: &instrukt_core::config::Admitted) -> bool;
    /// Opens the hosted connections and supervises them. Each connector kind
    /// is built here, once; a setting a kind refuses is the error, and stops
    /// the runner at its start. `wake` tells the relay a message arrived;
    /// `supervise` tells the run loop a connection thread ended.
    fn start(&self, start: CompanyStart<'_>) -> Result<Box<dyn CompanySupervision>, CompanyError>;
    /// One pass of conversation work beside links: finished turns read into
    /// answers and posted, the next message delivered. Before
    /// [`CompanyPorts::start`] it does nothing.
    fn relay(
        &self,
        home: &instrukt_core::home::Home,
        admitted: &instrukt_core::config::Admitted,
        source: &dyn crate::runtime::conversations::ConversationSource,
        adapter: &dyn crate::runtime::RuntimeAdapter,
        pass: CompanyRelayPass,
    ) -> Result<(), CompanyError>;
    /// Whether conversation work is outstanding.
    fn outstanding(&self, home: &instrukt_core::home::Home) -> Result<bool, CompanyError>;
    /// Records whether the relay serving conversations runs: its failure, or
    /// none.
    fn relay_state(&self, home: &instrukt_core::home::Home, failure: Option<&str>);
    /// The conversations section of the inspection report.
    fn status(
        &self,
        home: &instrukt_core::home::Home,
        now: jiff::Timestamp,
    ) -> Result<serde_json::Value, CompanyError>;
}

/// What [`CompanyPorts::start`] opens the connections with.
pub struct CompanyStart<'a> {
    pub home: &'a instrukt_core::home::Home,
    pub current: &'a instrukt_core::config::Current,
    pub env: &'a instrukt_core::config::Env,
    /// The operator's clock, as the connectors read it.
    pub clock: std::sync::Arc<dyn Fn() -> jiff::Timestamp + Send + Sync>,
    pub wake: std::sync::mpsc::SyncSender<()>,
    pub supervise: std::sync::mpsc::SyncSender<()>,
}

/// The hosted connections, supervised by the run loop; dropping them ends
/// them.
pub trait CompanySupervision: Send {
    /// Restarts every connection whose thread ended unexpectedly.
    fn check(&mut self);
}

impl std::fmt::Debug for RunnerWiring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerWiring")
            .field("work", &self.work.is_some())
            .field("github_credentials", &self.github_credentials.is_some())
            .field("review", &self.review.is_some())
            .field("workflow", &self.workflow.is_some())
            .field("company", &self.company.is_some())
            .finish()
    }
}

/// The configured identities automatic review reads: the App bots trusted
/// as pull-request authors and the reviewer identity's bot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewIdentities {
    /// Every configured App's bot other than the reviewer's.
    pub trusted_bots: Vec<instrukt_core::github::Bot>,
    pub reviewer: instrukt_core::github::Bot,
}

/// What deriving a candidate's coverage found, as the runner acts on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateFinding {
    /// The candidate has its verdict: its own, or a derived approval GitHub
    /// shows.
    Settled,
    /// Every commit is covered; the derived approval is recorded.
    Covered,
    /// A commit is not covered: the candidate owes a review.
    Owed,
    /// The repository cannot answer; derived again next tick.
    CannotAnswer(String),
}

/// A head verdict's value as work answers it at the boundary: the runner
/// cannot name work's own verdict type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadVerdictValue {
    Approve,
    NeedsWork,
}

impl HeadVerdictValue {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::NeedsWork => "needs_work",
        }
    }
}

/// What this computer holds of a commit's head verdict, as work's
/// Settlement row reads it (`github-apps`, Admission of review evidence,
/// Settlement).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerdictStanding {
    /// No verdict stands here and none was handed off.
    Nothing,
    /// A verdict stands here; `recorder` recorded its current version.
    Stands {
        verdict: HeadVerdictValue,
        recorder: SessionRef,
    },
    /// The verdict that stood here was handed off by `export`.
    HandedOff { export: String },
}

/// A commit's head verdict standing with the pinned base work admits its
/// verdict against; `None` when the commit shares no history with the
/// canonical branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadVerdict {
    pub standing: VerdictStanding,
    pub pinned_base: Option<instrukt_core::ids::CommitSha>,
}

/// Work's answers a requested review is admitted on
/// (`project/instrukt/spec/feature/runner/requested-review`, Kinds of ask,
/// Coordinates, Requested review records). Read-only: no lock is taken.
pub trait HeadVerdicts: Send + Sync {
    /// `head`'s verdict standing on this computer and its pinned base.
    fn head_verdict(
        &self,
        project: &ProjectRef,
        head: &instrukt_core::ids::CommitSha,
    ) -> Result<HeadVerdict, String>;
    /// The moment before which review evidence history retires at `now`.
    fn history_cutoff(&self, now: Timestamp) -> Timestamp;
}

/// Work's answers the runner asks for automatic review
/// (`project/instrukt/spec/feature/runner/pull-request-review`).
pub trait AutomaticReview: Send + Sync {
    /// The two identities' readiness on `repository`; a met answer carries
    /// the observation credential, and work keeps the publication
    /// credential for the pass.
    fn readiness(
        &self,
        repository: &str,
        now: Timestamp,
    ) -> instrukt_core::interfaces::ReviewReadiness;
    /// The configured identities.
    fn identities(&self) -> Result<ReviewIdentities, String>;
    /// The pull requests a todo's work lifecycle owns in `project`.
    fn todo_pulls(
        &self,
        project: &instrukt_core::ids::ProjectSlug,
    ) -> Result<std::collections::BTreeSet<u64>, String>;
    /// Whether `head` carries a verdict: evidence this computer holds, or
    /// what the reviewer bot already published.
    fn carries_verdict(
        &self,
        project: &instrukt_core::ids::ProjectSlug,
        head: &instrukt_core::ids::CommitSha,
    ) -> Result<bool, String>;
    /// Whether `commit`'s current verdict with work is needs work.
    fn needs_work(
        &self,
        project: &instrukt_core::ids::ProjectSlug,
        commit: &instrukt_core::ids::CommitSha,
    ) -> Result<bool, String>;
    /// Derives whether the active integration `candidate` is covered and
    /// records, publishes or withdraws its derived approval accordingly.
    /// `pulls` are the eligible pull requests whose head it is. `now` is the
    /// pass's moment; `clock` is read again when publication's credential is
    /// issued, since coverage may first have spent seconds.
    fn candidate(
        &self,
        project: &ProjectRef,
        repository: &str,
        candidate: &instrukt_core::ids::CommitSha,
        pulls: &[u64],
        now: Timestamp,
        clock: &dyn Fn() -> Timestamp,
    ) -> Result<CandidateFinding, String>;
    /// Publishes every verdict this computer holds for `project`, and
    /// recognises what the reviewer bot already published for each eligible
    /// head it holds none of. `eligible` is each eligible pull request with
    /// its current head, as this pass observed them; `candidate` is the
    /// active integration candidate, whose derived approval is current.
    /// `now` is the pass's moment; `clock` is read again when publication's
    /// credential is issued.
    fn publish(
        &self,
        project: &instrukt_core::ids::ProjectSlug,
        repository: &str,
        eligible: &[(u64, instrukt_core::ids::CommitSha)],
        candidate: Option<&instrukt_core::ids::CommitSha>,
        now: Timestamp,
        clock: &dyn Fn() -> Timestamp,
    ) -> Result<(), String>;
}

/// Work-owned, non-mutating inspection of shared-record GitHub prerequisites,
/// and of automatic review's.
pub trait GitHubReadiness:
    Send + Sync + instrukt_core::interfaces::AutomaticReviewReadiness
{
    fn reflection_ready(
        &self,
        repository: &str,
        board: Option<&instrukt_core::config::BoardRef>,
        now: Timestamp,
    ) -> Result<(), instrukt_core::github::AppError>;
}

/// Bounded work-owned GitHub credentials, composed by CLI for observation.
pub trait GitHubCredentials: Send + Sync {
    fn credential(
        &self,
        repository: &str,
        operation: instrukt_core::github::Operation,
        now: Timestamp,
    ) -> Result<Arc<instrukt_core::github::Credential>, instrukt_core::github::AppError>;
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
