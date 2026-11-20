//! Conversations through the real runner: a company member addresses the
//! company's agent through the Slack stand-in, the agent's session runs in the
//! fake T3 host, and its answer is posted back.
#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]

use crate::common;
use crate::fixtures::conversing::{COMPANY, Company, patch_company, wait};
use crate::fixtures::documented;
use crate::fixtures::slack::Fate;
use instrukt_company::conversations;
use instrukt_connector_slack::{SLACK, SLACK_MESSAGE_LIMIT, TURN_AWAY, UNAVAILABLE, WORKING};
use instrukt_runner::transport::EVENT_STREAM;

/// The source position the state database keeps for the conversation of
/// `key`.
fn position(c: &Company, key: &str) -> Option<String> {
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    db.query_row(
        "SELECT position_json FROM connected_conversations WHERE semantic_key=?1",
        [key],
        |row| row.get(0),
    )
    .unwrap()
}

/// How many snapshot reads T3's readiness server served on sessions T3
/// issued after its first `issued`: the reads of a runner started once that
/// many were issued, since a runner issues its own session at its first
/// refresh. A runner refreshes readiness twice when it starts, once on its
/// own and once for its first wake, and each refresh reads T3's descriptor
/// before its snapshot read: two such reads mean both its startup reads of
/// the descriptor are done. A late read of a runner killed earlier used an
/// earlier session and is not counted, nor is a refused one.
fn reads_since(c: &Company, issued: usize) -> usize {
    common::t3_readiness::reads(&c.env)
        .iter()
        .filter_map(|session| session.rsplit('-').next()?.parse::<usize>().ok())
        .filter(|n| *n > issued)
        .count()
}

/// The runner wakes, and refreshes readiness through T3's descriptor, each
/// minute in real time even with the operator's clock standing still: a held
/// gate stays clear of readiness only while the runner started under a
/// minute ago.
fn before_a_wake(started: std::time::Instant) {
    assert!(
        started.elapsed() < std::time::Duration::from_secs(50),
        "the gate was armed past the runner's first real-time wake, so a readiness refresh may have read through it"
    );
}

/// Where each post went: its channel and thread.
fn destinations(c: &Company) -> Vec<(String, Option<String>)> {
    c.slack
        .posts()
        .into_iter()
        .map(|post| (post.channel, post.thread))
        .collect()
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn a_members_question_is_answered_in_that_conversation_and_nowhere_else() {
    // History is off, so the conversation relay is the stand-in's only
    // source reader while the runner runs.
    let mut c = Company::without_procedure(common::TestEnv::new(), ", history: false", "", "");
    // The operator's clock stands still, so the runner wakes only on its
    // real-time interval.
    c.env.set_env("INSTRUKT_NOW", "2026-11-19T00:00:00Z");
    // The company starts its sessions on its own order of profiles.
    let profile = instrukt_core::ids::ProfileName::parse("antigravity").unwrap();
    patch_company(&c.env, COMPANY, &format!("profiles: [{profile}]"));
    // The computer runs other work: many other threads, one of them
    // streaming and rewritten while instrukt reads.
    let mut elsewhere = c.store.elsewhere(200, "Elsewhere chatter.");
    elsewhere.stream();
    let gate = c.store.gate();
    c.run();
    // The relay runs, so its subscription to T3's database exists; watching
    // it holds no open file on T3's data folder.
    let t3_folder = c.store.file.parent().unwrap().to_path_buf();
    assert_eq!(c.conversations().relay_failure, None);
    assert_eq!(c.runner_files_on(&t3_folder), 0);
    let channel = "C1";
    // No published procedure declares the role conversations start under:
    // the question waits, no session starts, and the cause names the role.
    let question = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> When do we ship?", None);
    wait("the waiting question's cause", || {
        c.conversations()
            .conversations
            .iter()
            .any(|pending| !pending.failures.is_empty())
    });
    let waiting = c.status()["conversations"]["conversations"].clone();
    insta::assert_snapshot!(
        "no-procedure-status",
        serde_json::to_string_pretty(&waiting).unwrap()
    );
    assert!(c.sessions().is_empty());
    // Once the procedure is published, the runner starts the session and
    // dies before the runtime accepted the question's opening.
    c.runtime.flag("die-before-start", true);
    c.publish_procedure();
    c.runner_ended();
    assert!(c.sessions().is_empty());
    // Restarted, the question stays pending with its cause: a start that
    // remains unresolved, its key and the command that resolves it. That
    // start failed, so Slack shows on the thread that the assistant is
    // temporarily unavailable.
    c.run();
    wait("the unavailable status on the thread", || {
        c.slack.status(channel, &question).as_deref() == Some(UNAVAILABLE)
    });
    wait("the pending question's new cause", || {
        c.status()["conversations"]["conversations"] != waiting
    });
    insta::assert_snapshot!(
        "unresolved-opening-status",
        serde_json::to_string_pretty(&c.status()["conversations"]["conversations"]).unwrap()
    );
    assert!(c.sessions().is_empty());
    let key = conversations::key(&c.company(), SLACK, &format!("{channel}/{question}"));
    // Nothing was created: the start is resolved as such, and the retry dies
    // after the runtime accepted the opening and before the runner bound it.
    c.runtime.flag("die-after-start", true);
    let resolve = ["sessions", "resolve-start", &key, "--none"];
    #[cfg(debug_assertions)]
    {
        let held = hold_recovery_key(&c, &key);
        let command = recovery_waiting(&c, resolve, &key).unwrap();
        drop(held);
        let result = command.wait_with_output().unwrap();
        assert!(result.status.success(), "{result:?}");
    }
    #[cfg(not(debug_assertions))]
    c.env.run(c.env.home(), resolve).success();
    c.runner_ended();
    let session = c.session();
    assert_eq!(c.runtime.message_count(&session), 1);
    // Restarted while the opening turn runs, with the source otherwise
    // quiet: recovery binds the session and reads its thread mid-turn. The
    // runner dies once that read is retained, as its next source read
    // begins, and the turn finishes while it is down.
    c.store
        .thread(&session, None, None, "2026-11-08T10:00:00.000Z");
    elsewhere.settle();
    // While T3 cannot list its threads, the restarted runner cannot bind the
    // session and reads nothing of the source. Its two startup readiness
    // refreshes read T3's descriptor.
    c.runtime.flag("list-failure", true);
    let issued = c.runtime.issued();
    let started = std::time::Instant::now();
    c.run();
    wait("the restarted runner's two readiness refreshes", || {
        reads_since(&c, issued) >= 2
    });
    // Every source read is held now, and the relay is the gate's only
    // reader: readiness, past its two startup refreshes, reads again only on
    // a later wake. Once T3 lists again the relay binds the session and
    // reads its thread; reads are let through
    // until that read is retained as the conversation's position, and no
    // further, so the read that reaches the gate after the nudge is the
    // relay's next one.
    let recorded = position(&c, &key);
    gate.hold();
    c.runtime.flag("list-failure", false);
    wait("the recovery read retained", || {
        gate.admit_unless(|_| position(&c, &key) != recorded)
    });
    elsewhere.nudge();
    gate.held();
    before_a_wake(started);
    c.runner_dies();
    gate.open();
    // Someone types in the session's thread directly, and both turns finish
    // while the runner is down: the typed one comes first in the thread, so
    // it is read before the answer's, and it is not posted. An answer longer
    // than one Slack message arrives as its pieces.
    c.store.turn(
        &session,
        "direct",
        0,
        "A note typed in the thread.",
        "Noted.",
    );
    let reply = "x".repeat(SLACK_MESSAGE_LIMIT + 1);
    c.finish_turn(&session, 0, &reply);
    let issued = c.runtime.issued();
    let started = std::time::Instant::now();
    c.run();
    wait("the runner's two readiness refreshes", || {
        reads_since(&c, issued) >= 2
    });
    // The session opens with its procedure's loader under the role, then the
    // question; its recorded assignment is the role in the company's home.
    insta::assert_snapshot!("opening-from-procedure", c.opening(&session));
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());
    let caller = instrukt_core::ids::SessionRef::parse(&format!(
        "{}:{session}",
        instrukt_core::config::Runtime::T3.name()
    ))
    .unwrap();
    let resolved = || {
        instrukt_runner::assignments::resolve(
            &home,
            &caller,
            instrukt_todos::work::recorded_assignment,
            instrukt_company::conversations::conversation_assignment,
        )
        .unwrap()
    };
    wait("the session's recorded binding", || resolved().is_some());
    let assignment = resolved().unwrap();
    assert_eq!(
        assignment.role.as_str(),
        instrukt_core::config::COMPANY_AGENT_ROLE
    );
    assert_eq!(assignment.company, Some(c.company()));
    wait("the answer's pieces", || c.slack.posts().len() >= 2);
    wait("nothing left to post", || {
        c.conversations().conversations.is_empty()
    });
    // While the other thread streams again, a later catch-up of this
    // conversation completes: the next source read stands at the gate
    // after it. Its outcome leaves no cause.
    // The relay is the gate's only reader: this runner's two readiness
    // refreshes read when it started. One catch-up's read is let through, and the next
    // read to reach the gate is the relay's next one.
    elsewhere.stream();
    gate.hold();
    wait("a catch-up through the gate", || {
        gate.admit_unless(|served| served >= 1)
    });
    gate.held();
    before_a_wake(started);
    let status = c.conversations();
    assert!(
        status.conversations.is_empty(),
        "{:?}",
        status.conversations
    );
    assert_eq!(status.relay_failure, None);
    gate.open();
    assert_eq!(
        destinations(&c),
        [
            (channel.to_owned(), Some(question.clone())),
            (channel.to_owned(), Some(question.clone()))
        ]
    );
    // Together the parts carry the whole answer, as Markdown.
    let posts = c.slack.posts();
    assert!(posts.iter().all(|post| post.text.is_none()));
    assert_eq!(
        posts
            .iter()
            .filter_map(|post| post.markdown.as_deref())
            .collect::<String>(),
        reply
    );
    // The session was started on the company's profile.
    assert_eq!(
        c.runtime.provider(&session),
        instrukt_core::config::Agents::load(&home).unwrap().profiles[&profile].instance
    );
    // One session, which received one opening input.
    assert_eq!(c.sessions(), [session.as_str()]);
    assert_eq!(c.runtime.message_count(&session), 1);
    // The turn ended: the status is cleared.
    wait("the status cleared", || {
        c.slack.status(channel, &question).is_none()
    });
    // The database holding the conversation, and the files beside it, are
    // its owner's alone.
    use std::os::unix::fs::PermissionsExt as _;
    for side in ["", "-wal", "-shm"] {
        let file = c
            .env
            .instrukt_home()
            .join(format!("state/instrukt.db{side}"));
        if let Ok(meta) = std::fs::metadata(&file) {
            assert_eq!(meta.permissions().mode() & 0o777, 0o600, "{side}");
        }
    }
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV3
#[test]
fn two_members_in_one_conversation_each_have_their_own_turn_in_order() {
    let mut c = Company::new("");
    c.run();
    let root = c
        .slack
        .mention("C1", "USTEVEN", "<@UAGENT> Is the build green?", None);
    let session = c.session();
    // While the first turn runs, two more arrive out of the order they were
    // written in, within one second. The later-written one is taken in
    // before the earlier one arrives: its lookup waits until then.
    c.slack.hold_lookups();
    c.slack.mention_at(
        "1700000500.000100",
        "C1",
        "USTEVEN",
        "<@UAGENT> And the changelog?",
        Some(&root),
    );
    wait("the later message taken in", || c.slack.held_lookups() > 0);
    c.slack.mention_at(
        "1700000500.000000",
        "C1",
        "UANNA",
        "<@UAGENT> And the release notes?",
        Some(&root),
    );
    wait("all three messages acknowledged", || {
        c.slack.acknowledged().len() == 3
    });
    c.slack.release_lookups();
    // The runner dies right after the runtime accepted the second message
    // and before it recorded that, and that turn finishes while it is down.
    c.runtime.flag("die-after-send", true);
    c.finish_turn(&session, 0, "Green.");
    c.runner_ended();
    assert_eq!(c.runtime.message_count(&session), 2);
    c.finish_turn(&session, 1, "Written.");
    c.run();
    // Restarted, it posts that answer once and the third message follows,
    // with no second copy of the second.
    wait("the third turn", || c.runtime.message_count(&session) == 3);
    assert_eq!(c.slack.posts().len(), 2);
    c.finish_turn(&session, 2, "Done.");
    wait("nothing left to post", || {
        c.conversations().conversations.is_empty() && c.slack.posts().len() == 3
    });
    assert_eq!(c.runtime.message_count(&session), 3);
    // Each names its sender in written order. The third's context excludes
    // the second even though it was captured before the second was delivered.
    insta::assert_snapshot!(
        "three-turns-delivered",
        (0..3)
            .map(|n| c.input(&session, n))
            .collect::<Vec<_>>()
            .join("\n---\n")
    );
    for n in 0..3 {
        let delivered = c.delivered(&session, n);
        assert_eq!(delivered.members, 1, "input {n}");
        assert_eq!(delivered.empty, 0, "input {n}");
        // Each started its own turn: none arrived while a turn was running,
        // as the runtime recorded on its arrival.
        assert!(!c.queued(&session, n), "input {n}");
    }
    assert_eq!(c.sessions().len(), 1);
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV4
#[test]
fn a_person_who_is_not_one_of_the_companys_own_receives_one_fixed_reply_and_no_session_starts() {
    let mut c = Company::new("");
    // The runner's clock stands still, so the wait Slack names does not pass
    // while it runs.
    c.env.set_env("INSTRUKT_NOW", "2030-01-01T09:00:00Z");
    c.run();
    let channel = "C1";
    // Slack limits the lookup of who wrote it and names a wait: the message
    // waits, nothing is delivered or replied, and the cause shows. No lookup
    // is made for this workspace while the wait lasts, also not for a
    // message that arrives meanwhile.
    let waiting = |c: &Company| {
        c.conversations()
            .conversations
            .iter()
            .filter(|pending| pending.pending_messages == 1 && !pending.failures.is_empty())
            .count()
    };
    c.slack.limit_lookup(30);
    let asked = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Are you there?", None);
    wait("the waiting message's cause", || waiting(&c) == 1);
    let looked_up = c.slack.lookups();
    c.slack.direct("D1", "UANNA", "Are you there too?");
    wait("the second waiting message's cause", || waiting(&c) == 2);
    assert_eq!(c.slack.lookups(), looked_up);
    assert!(c.sessions().is_empty());
    assert!(c.slack.posts().is_empty());
    // Restarted, the runner learns any wait again from Slack, which now
    // answers: both are answered.
    c.kill_runner();
    c.env.remove_env("INSTRUKT_NOW");
    c.run();
    wait("both conversations' sessions", || c.sessions().len() == 2);
    let mut inputs: Vec<String> = c
        .sessions()
        .iter()
        .map(|session| c.input(session, 0))
        .collect();
    inputs.sort();
    insta::assert_snapshot!("waited-messages-delivered", inputs.join("\n---\n"));
    // A guest gets one fixed reply in their conversation, however often
    // they write.
    let first = c
        .slack
        .mention(channel, "USTRANGER", "<@UAGENT> Who are you?", None);
    c.slack
        .mention(channel, "USTRANGER", "<@UAGENT> Hello?", Some(&first));
    // A person answered before is decided again for each message: deactivated
    // since, they are not answered.
    c.slack.change("USTEVEN", |person| person.deleted = true);
    c.slack
        .mention(channel, "USTEVEN", "<@UAGENT> Still there?", Some(&asked));
    wait("both replies", || c.slack.posts().len() == 2);
    assert_eq!(
        destinations(&c),
        [
            (channel.to_owned(), Some(first)),
            (channel.to_owned(), Some(asked.clone()))
        ]
    );
    // The fixed reply goes as plain text.
    for post in c.slack.posts() {
        assert_eq!(post.text.as_deref(), Some(TURN_AWAY));
        assert_eq!(post.markdown, None);
    }
    let sessions = c.sessions();
    assert_eq!(sessions.len(), 2);
    assert_eq!(
        sessions
            .iter()
            .map(|session| c.runtime.message_count(session))
            .sum::<usize>(),
        2
    );
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV5
#[test]
fn a_conversation_message_activates_no_subscribed_procedure() {
    let env = common::TestEnv::new();
    let redis = common::redis::RedisServer::new(&env);
    let project = documented::project(
        &env,
        "fork",
        &[(
            "instrukt.yaml",
            format!(
                "schema_version: {}\nproject: fork\n",
                instrukt_core::config::CURRENT_SCHEMA_VERSION
            ),
        )],
    );
    project.write(
        "docs/domain/reactions/procedure/everything.md",
        &documented::snippet(
            "procedure",
            "Everything",
            "Answer every fact.",
            "subscription_home: project\nsubscriptions: ['*']\nrole: responder\n",
            &[],
            "Answer every event.\n",
        ),
    );
    project.commit("docs: answer every event");
    documented::sync(&env, &project);
    let mut c = Company::on(
        env,
        ", events: true",
        &format!("redis: {{cluster: {{url: '{}'}}}}\n", redis.url),
        "",
    );
    c.run();
    c.slack
        .mention("C1", "USTEVEN", "<@UAGENT> Anything new?", None);
    let session = c.session();
    insta::assert_snapshot!("conversation-delivered", c.input(&session, 0));
    c.finish_turn(&session, 0, "Nothing new.");
    wait("the answer's post", || !c.slack.posts().is_empty());
    // An event after the conversation: once the procedure answered it, the
    // event reader has passed everything before it.
    let computer = std::fs::read_to_string(c.env.instrukt_home().join("state/machine-id")).unwrap();
    let barrier = "barrier-event";
    let event = serde_json::json!({"id": barrier, "type": "example.changed", "source": computer.trim(), "subject": "todo:demo", "home": "project:fork", "home_owner": "unresolved", "time": "2026-11-05T00:00:00Z", "data": {}});
    redis::cmd("XADD")
        .arg(EVENT_STREAM)
        .arg("*")
        .arg("event")
        .arg(event.to_string())
        .query::<String>(&mut redis.connection())
        .unwrap();
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let activated = || -> Vec<String> {
        db.prepare("SELECT event_id FROM procedure_activations")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    };
    wait("the event reader's activation", || !activated().is_empty());
    assert_eq!(activated(), [barrier]);
}

/// The texts a conversation's status line showed, in order, each change
/// once: a clearing is an empty text.
fn shown(c: &Company, channel: &str, thread: &str) -> Vec<String> {
    let mut texts = c.slack.status_texts(channel, thread);
    texts.dedup();
    texts
}

/// The next session the runtime starts has its thread in T3's projection
/// from its creation, as T3 projects it.
fn projected(c: &Company) {
    c.store.thread(
        &format!("s{}", c.sessions().len() + 1),
        None,
        None,
        "2026-11-08T10:00:00.000Z",
    );
}

/// The operator's clock in the file `path`, running `rate` times faster than
/// real time from `from` until stopped: a lock's two-minute wait passes in
/// moments, while a write's short wait for the state database still outlasts
/// any write.
struct RunningClock {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    mover: Option<std::thread::JoinHandle<jiff::Timestamp>>,
}
impl RunningClock {
    fn start(path: std::path::PathBuf, from: jiff::Timestamp, rate: u32) -> Self {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopping = std::sync::Arc::clone(&stop);
        let mover = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let mut now = from;
            while !stopping.load(std::sync::atomic::Ordering::Acquire) {
                now = from + jiff::SignedDuration::try_from(started.elapsed() * rate).unwrap();
                let next = path.with_extension("next");
                std::fs::write(&next, now.to_string()).unwrap();
                std::fs::rename(&next, &path).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            now
        });
        Self {
            stop,
            mover: Some(mover),
        }
    }
    /// Stops the clock where it stands, and returns that time.
    fn stop(mut self) -> jiff::Timestamp {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.mover.take().unwrap().join().unwrap()
    }
}

/// When the no-work interval of the addressed message `message` opened, as
/// the state database keeps it; `None` while none is open.
fn interval(c: &Company, message: &str) -> Option<String> {
    use instrukt_core::state_db::rusqlite::OptionalExtension as _;
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    db.query_row(
        "SELECT unavailable_since FROM conversation_messages WHERE message=?1",
        [message],
        |row| row.get(0),
    )
    .optional()
    .unwrap()
    .flatten()
}

/// When the relay's current failure began, as the state database keeps it;
/// `None` while the relay reads its source.
fn relay_since(c: &Company) -> Option<String> {
    use instrukt_core::state_db::rusqlite::OptionalExtension as _;
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    db.query_row(
        "SELECT updated_at FROM conversation_relay WHERE id=1 AND failure IS NOT NULL",
        [],
        |row| row.get(0),
    )
    .optional()
    .unwrap()
}

/// The operator's clock in the file `path` set to `at`, replaced whole.
fn set_clock(path: &std::path::Path, at: jiff::Timestamp) {
    let next = path.with_extension("next");
    std::fs::write(&next, at.to_string()).unwrap();
    std::fs::rename(&next, path).unwrap();
    let wake = path.with_extension("notice-wake");
    if wake.exists() {
        let next_wake = wake.with_extension("notice-wake-next");
        std::fs::write(&next_wake, at.to_string()).unwrap();
        std::fs::rename(next_wake, wake).unwrap();
    }
}

/// Stops `running` once the interval of each of `messages` has opened and
/// sets the clock back to the earliest opening, so no minute of it passes
/// while the row goes on and no notice is owed.
fn halt_at_intervals(
    c: &Company,
    running: RunningClock,
    clock: &std::path::Path,
    messages: &[&str],
) {
    common::wait_until(
        "the intervals opened",
        std::time::Duration::from_secs(60),
        || {
            messages
                .iter()
                .all(|message| interval(c, message).is_some())
        },
    );
    running.stop();
    let since = messages
        .iter()
        .filter_map(|message| interval(c, message))
        .map(|at| at.parse::<jiff::Timestamp>().unwrap())
        .min()
        .unwrap();
    set_clock(clock, since);
}

const EXPECTED_NOTICE: &str = "The assistant is temporarily unavailable. Your message is saved and you'll get the answer here when it's back — no need to send it again.";

fn notice_text(member: &str) -> String {
    format!("<@{member}> {EXPECTED_NOTICE}")
}

/// How many of the app's posts are the one notice to `member`.
fn notices(c: &Company, member: &str) -> usize {
    let expected = notice_text(member);
    c.slack
        .posts()
        .iter()
        .filter(|post| {
            let text = post
                .markdown
                .as_deref()
                .or(post.text.as_deref())
                .unwrap_or_default();
            if !text.starts_with(&expected) {
                return false;
            }
            assert_eq!(text, expected);
            true
        })
        .count()
}

fn notice_state(c: &Company, conversation: &str, member: &str) -> Option<String> {
    notice_record(c, conversation, member).map(|(state, _)| state)
}

fn notice_record(
    c: &Company,
    conversation: &str,
    member: &str,
) -> Option<(String, Option<String>)> {
    use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension as _, params};
    let db = Connection::open(c.env.instrukt_home().join("state/instrukt.db")).unwrap();
    db.query_row(
        "SELECT state,failure FROM conversation_refusals WHERE conversation=?1 AND sender=?2 AND reason='unavailable'",
        params![conversation, member],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .optional()
    .unwrap()
}

fn minute() -> jiff::SignedDuration {
    jiff::SignedDuration::from_mins(1)
}

fn interval_at(c: &Company, message: &str) -> jiff::Timestamp {
    wait("the message's no-work interval", || {
        interval(c, message).is_some()
    });
    interval(c, message).unwrap().parse().unwrap()
}

/// How many of the app's posts are not a notice.
fn answers(c: &Company) -> usize {
    c.slack
        .posts()
        .iter()
        .filter(|post| {
            !post
                .markdown
                .as_deref()
                .or(post.text.as_deref())
                .is_some_and(|text| text.contains(EXPECTED_NOTICE))
        })
        .count()
}

/// The session a message started, beside `before`, the sessions before it.
fn started(c: &Company, before: &[String]) -> String {
    wait("the message's session", || {
        c.sessions().len() > before.len()
    });
    c.sessions()
        .into_iter()
        .find(|session| !before.contains(session))
        .unwrap()
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV7
#[test]
fn a_member_hears_once_that_the_assistant_is_unavailable_then_gets_the_answer() {
    std::thread::scope(|scope| {
        scope.spawn(notice_epochs_and_reconciliation);
        scope.spawn(a_notice_in_flight_reconciles_after_a_noop);
        scope.spawn(a_normalized_notice_is_reconciled_after_a_restart);
        scope.spawn(a_mixed_notice_epoch_waits_for_the_final_answer_piece);
        scope.spawn(successful_empty_turns_rearm_notices);
        scope.spawn(the_notice_minute_starts_when_a_slow_start_fails);
        scope.spawn(working_status_stays_visible_during_a_slow_start);
        scope.spawn(an_answer_to_one_member_does_not_restart_anothers_minute);
        scope.spawn(a_dispatch_failure_notifies_the_member_once_before_the_answer);
    });
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV19
#[test]
fn one_conversation_opens_while_another_start_is_being_verified() {
    let (mut c, _clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    projected(&c);
    let held = c.runtime.state_dir().join("s1.hold-show");
    std::fs::write(&held, "").unwrap();
    c.slack.direct("D30", "USTEVEN", "Where is the release?");
    wait("the first start has reached its post-create check", || {
        c.runtime.held_show_running("s1")
    });

    c.store.thread("s2", None, None, "2026-11-08T10:00:00Z");
    c.slack.direct("D31", "UANNA", "Who is on call?");
    wait(
        "the second conversation starts during the held check",
        || c.sessions().iter().any(|session| session == "s2"),
    );
    c.finish_turn("s2", 0, "Ada is on call.");
    wait(
        "the second conversation answers during the held check",
        || c.slack.posts_beginning("Ada is on call.") == 1,
    );
    assert!(c.runtime.held_show_running("s1"));
    #[cfg(debug_assertions)]
    let key = conversations::key(&c.company(), SLACK, "D30");
    #[cfg(debug_assertions)]
    let same_key =
        recovery_waiting(&c, ["sessions", "resolve-start", &key, "--none"], &key).unwrap();
    std::fs::remove_file(held).unwrap();
    #[cfg(debug_assertions)]
    {
        let result = same_key.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            !result.status.success() && stderr.contains("has no unresolved start"),
            "{stderr}"
        );
    }

    c.finish_turn("s1", 0, "The release is in the wiki.");
    wait("the first conversation gets its own answer", || {
        c.slack.posts_beginning("The release is in the wiki.") == 1 && answers_recorded(&c)
    });
    assert_eq!(c.runtime.message_count("s1"), 1);
    assert_eq!(c.runtime.message_count("s2"), 1);
    assert_eq!(c.slack.posts().len(), 2);

    #[cfg(debug_assertions)]
    {
        c.runtime.delete("s1");
        c.store.unproject_thread("s1");
        let result = timeout_recovery(
            &c,
            &_clock,
            "input:t3:s1",
            ["sessions", "replace-deleted", &key, "--stopped", "t3:s1"],
        );
        let stderr = String::from_utf8_lossy(&result.stderr);
        let diagnostic = format!(
            "instrukt: dispatch is still busy after the recovery wait for semantic key `{key}`. Retry `instrukt sessions replace-deleted {key} --stopped t3:s1` once dispatch releases it"
        );
        let errors = stderr
            .lines()
            .filter(|line| !line.starts_with("instrukt: clock overridden"))
            .collect::<Vec<_>>();
        assert_eq!(errors, [diagnostic.as_str()]);
        retry_as_printed_in_shell(&c, &_clock);
    }
}

#[cfg(debug_assertions)]
fn timeout_recovery<const N: usize>(
    c: &Company,
    clock: &std::path::Path,
    lock_key: &str,
    args: [&str; N],
) -> std::process::Output {
    let _held = hold_recovery_key(c, lock_key);
    let marker = c.env.path("recovery-lock-wait");
    if marker.exists() {
        std::fs::remove_file(marker).unwrap();
    }
    let mut command = recovery_waiting(c, args, lock_key).unwrap();
    let clock = RunningClock::start(clock.to_owned(), interval_clock(clock), 60);
    wait(
        "operator recovery gives up waiting for the held lock",
        || command.try_wait().unwrap().is_some(),
    );
    clock.stop();
    let result = command.wait_with_output().unwrap();
    assert!(!result.status.success());
    result
}

#[cfg(debug_assertions)]
fn retry_as_printed_in_shell(c: &Company, clock: &std::path::Path) {
    let key = "instrukt:run:demo:retry[1]";
    let target = "t3:s[1]";
    c.runtime
        .seed("s[1]", c.env.home(), "An unbound thread", "Opening");
    let request = serde_json::json!({
        "checkout": c.env.home(), "key": key, "title": key, "brief": "Retained opening"
    });
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    db.execute(
        "INSERT INTO runtime_starts(runtime,semantic_key,kind,subject,checkout,brief,request,begun_at,reported,state)
         VALUES('t3',?1,'run',?1,?2,'Retained opening',?3,'2026-11-19T10:00:00Z',0,'unresolved')",
        instrukt_core::state_db::rusqlite::params![key,c.env.home().to_string_lossy(),request.to_string()],
    ).unwrap();
    let failed = timeout_recovery(
        c,
        clock,
        key,
        [
            "sessions",
            "resolve-start",
            key,
            "--session",
            target,
            "--json",
        ],
    );
    let stderr = String::from_utf8_lossy(&failed.stderr);
    let (_, printed) = stderr.split_once("Retry `").unwrap();
    let (printed, _) = printed.split_once("` once dispatch releases it").unwrap();
    // Unquoted brackets would match these files even in a POSIX shell.
    for name in ["instrukt:run:demo:retry1", "t3:s1"] {
        std::fs::write(c.env.home().join(name), "").unwrap();
    }
    let result = c
        .env
        .recovery_shell(c.env.home(), printed)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let response: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(response, serde_json::json!({"key": key, "session": target}));
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV20
#[test]
fn a_member_keeps_their_question_when_the_conversation_thread_is_archived_or_deleted() {
    std::thread::scope(|scope| {
        scope.spawn(held_runtime_checks_leave_another_conversation_free);
        #[cfg(debug_assertions)]
        scope.spawn(a_live_deletion_is_confirmed_once_across_relay_passes);
        scope.spawn(deleted_conversation_waits_for_confirmed_replacement);
        archived_and_deleted_conversations_keep_saved_work();
    });
}

fn wait_for_answer_fixture(what: &str, condition: impl FnMut() -> bool) {
    use std::time::Duration;
    // These scenarios check SQLite, logs and native processes in parallel.
    // A modest polling interval leaves CPU time for the runners they observe.
    common::wait_until_with_interval(
        what,
        Duration::from_secs(45),
        Duration::from_millis(50),
        condition,
    );
}

fn answer_recovery_company(debug: bool) -> (Company, std::path::PathBuf) {
    let mut c = Company::prepared_for_recovery(status_env());
    // The fixture controls native notifications independently of its writes.
    let wakes = c.env.path("answer-recovery-wakes");
    answer_source_changed(&c, 0);
    if debug {
        c.env.set_env("INSTRUKT_LOG_LEVEL", "debug");
    }
    c.env
        .set_env("INSTRUKT_TEST_T3_WAKES", wakes.to_str().unwrap());
    let clock = c.env.path("clock");
    set_clock(&clock, "2026-11-19T10:00:00Z".parse().unwrap());
    c.env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    for (setting, name) in [
        ("INSTRUKT_TEST_CONVERSATION_PRESENCE_HOLD", "presence-hold"),
        ("INSTRUKT_TEST_CONVERSATION_DISPATCH_HOLD", "dispatch-hold"),
        ("INSTRUKT_TEST_CONVERSATION_ACK_HOLD", "ack-hold"),
    ] {
        c.env.set_env(setting, c.env.path(name).to_str().unwrap());
    }
    c.runtime.flag("separate-completion", true);
    c.run();
    (c, clock)
}

fn answer_source_changed(c: &Company, revision: u64) {
    let wakes = c.env.path("answer-recovery-wakes");
    let next = wakes.with_extension("next");
    std::fs::write(&next, revision.to_string()).unwrap();
    std::fs::rename(next, wakes).unwrap();
}

fn finish_answer_turn(c: &mut Company, session: &str, input: usize, answer: &str) {
    c.finish_turn(session, input, answer);
    let revision = std::fs::read_to_string(c.env.path("answer-recovery-wakes")).unwrap();
    answer_source_changed(c, revision.trim().parse::<u64>().unwrap() + 1);
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV8
#[test]
fn a_member_gets_the_answer_after_the_assistants_work_failed() {
    std::thread::scope(|scope| {
        #[cfg(debug_assertions)]
        scope.spawn(a_replacement_reports_the_old_acknowledgement);
        scope.spawn(an_idle_conversation_retries_an_unrecorded_send);
        scope.spawn(an_outage_before_dispatch_keeps_the_question);
        scope.spawn(a_late_original_dispatch_is_deduplicated_after_restart);
        scope.spawn(a_busy_conversation_releases_an_unresolved_send);
        failed_work_is_retried_after_a_restart();
    });
}

fn an_outage_before_dispatch_keeps_the_question() {
    let (mut c, clock, session) = answered_conversation(false);
    c.runtime.project(&c.env, &session);
    let descriptor = c.env.home().join(".t3/userdata/server-runtime.json");
    let held = descriptor.with_extension("held");
    std::fs::rename(&descriptor, &held).unwrap();
    let question = c
        .slack
        .direct("D30", "USTEVEN", "Where is the outage report?");
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let retained = || {
        db.query_row(
        "SELECT dispatch_command_id,dispatch_write_at,send_unknown_at FROM conversation_messages WHERE message=?1",
        [&question],
        |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?)),
    ).ok()
    };
    wait_for_answer_fixture("the operator sees the dispatch outage", || {
        conversation_finding(&c, "t3 conversation delivery").is_some()
            && interval(&c, &question).is_some()
    });
    let before = retained().unwrap();
    let command_id = before.0.as_deref().unwrap();
    assert_eq!((before.1, before.2), (None, None));
    let failure = || {
        db.query_row(
            "SELECT failure FROM conversation_messages WHERE message=?1",
            [&question],
            |row| row.get::<_, Option<String>>(0),
        )
        .unwrap()
    };
    let prior_failure = failure().unwrap();
    set_clock(
        &clock,
        interval_clock(&clock) + jiff::SignedDuration::from_mins(6),
    );
    // Keep dispatch unavailable with a different descriptor error. Only a
    // retry that has finished after the clock advance can persist that cause.
    std::fs::write(&descriptor, "invalid descriptor").unwrap();
    answer_source_changed(&c, 2);
    wait_for_answer_fixture(
        "the relay persists a fresh failure past the send reconciliation bound",
        || failure().is_some_and(|cause| cause != prior_failure),
    );
    assert_eq!(retained(), Some((Some(command_id.to_owned()), None, None)));
    assert_eq!(c.runtime.message_count(&session), 1);
    std::fs::rename(&held, &descriptor).unwrap();
    wait_for_answer_fixture(
        "dispatch resumes without replacing the command identity",
        || c.runtime.message_count(&session) == 2,
    );
    let ids = std::fs::read_to_string(c.runtime.state_dir().join("dispatch-ids")).unwrap();
    assert_eq!(ids.lines().collect::<Vec<_>>(), [command_id]);
    finish_answer_turn(&mut c, &session, 1, "The report is in the drive.");
    wait_for_answer_fixture("the saved question is answered once", || {
        c.slack.posts_beginning("The report is in the drive.") == 1
    });
}

fn an_idle_conversation_retries_an_unrecorded_send() {
    let (mut c, _, session) = answered_conversation(false);
    c.runtime.project(&c.env, &session);
    let staged = c.runtime.state_dir().join(format!("{session}.submit-held"));
    c.runtime.flag(&format!("{session}.lose-next-send"), true);
    c.runtime.flag(&format!("{session}.hold-next-submit"), true);
    let first = c
        .slack
        .direct("D30", "USTEVEN", "Where is the revised report?");
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let id = || {
        db.query_row(
            "SELECT dispatch_command_id FROM conversation_messages WHERE message=?1",
            [&first],
            |row| row.get::<_, Option<String>>(0),
        )
        .unwrap()
    };
    wait_for_answer_fixture("an idle thread retries the ambiguous send", || {
        staged.exists()
    });
    assert_eq!(id(), Some(std::fs::read_to_string(staged).unwrap()));
    assert_eq!(c.runtime.message_count(&session), 1);
    c.slack
        .direct("D30", "USTEVEN", "When does the review start?");
    c.runtime.flag(&format!("{session}.release-submit"), true);
    wait_for_answer_fixture(
        "the saved question is dispatched after the lost response",
        || c.runtime.message_count(&session) == 2,
    );
    assert!(
        c.input(&session, 1)
            .contains("Where is the revised report?")
    );
    finish_answer_turn(&mut c, &session, 1, "The revised report is in the drive.");
    wait_for_answer_fixture("the next question follows without a duplicate", || {
        c.runtime.message_count(&session) == 3 && id().is_none()
    });
    assert!(c.input(&session, 2).contains("When does the review start?"));
    assert!(
        !c.input(&session, 2)
            .contains("Where is the revised report?")
    );
}

fn a_late_original_dispatch_is_deduplicated_after_restart() {
    let (mut c, _, session) = answered_conversation(false);
    c.runtime.project(&c.env, &session);
    let native = c.runtime.state_dir();
    let staged = native.join(format!("{session}.submit-held"));
    let replayed = native.join(format!("{session}.late-deduplicated"));
    let duplicate = native.join(format!("{session}.late-submit"));
    let calls = native.join("dispatch-ids");
    c.runtime.flag(&format!("{session}.hold-next-submit"), true);
    let message = c
        .slack
        .direct("D30", "USTEVEN", "Where is the release plan?");
    wait_for_answer_fixture(
        "the original authenticated request is held before commit",
        || staged.exists(),
    );
    let original = std::fs::read_to_string(staged).unwrap();
    let original_input =
        std::fs::read(native.join(format!("{session}.submit-held-input"))).unwrap();
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let tracked =
        || {
            db.query_row(
        "SELECT attempted_at,dispatch_command_id FROM conversation_messages WHERE message=?1",
        [&message], |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, Option<String>>(1)?)),
    ).unwrap()
        };
    let before = tracked();
    assert!(before.0.is_some());
    assert_eq!(before.1.as_deref(), Some(original.as_str()));
    c.kill_runner();
    assert_eq!(c.runtime.message_count(&session), 1);
    c.run();
    wait_for_answer_fixture(
        "the new runner retries the exact command on a new connection",
        || c.runtime.message_count(&session) == 2,
    );
    assert_eq!(c.runtime.message(&session, 1).as_bytes(), original_input);
    finish_answer_turn(&mut c, &session, 1, "The plan is by the calendar.");
    wait_for_answer_fixture("the retry answers and clears the current attempt", || {
        c.slack.posts_beginning("The plan is by the calendar.") == 1 && tracked() == (None, None)
    });
    c.runtime.flag(&format!("{session}.release-submit"), true);
    wait_for_answer_fixture("the original request reaches T3 after the retry", || {
        replayed.exists() || duplicate.exists()
    });
    assert!(replayed.exists());
    assert!(!duplicate.exists());
    let sent = std::fs::read_to_string(calls).unwrap();
    assert_eq!(
        sent.lines().collect::<Vec<_>>(),
        [original.as_str(), original.as_str()]
    );
    assert_eq!(c.runtime.message_count(&session), 2);
}

fn a_busy_conversation_releases_an_unresolved_send() {
    let (mut c, clock, session) = answered_conversation(false);
    c.runtime.project(&c.env, &session);
    let at = interval_clock(&clock);
    c.runtime.flag(&format!("{session}.lose-next-send"), true);
    c.runtime
        .flag(&format!("{session}.hold-lost-response"), true);
    let first = c
        .slack
        .direct("D30", "USTEVEN", "Is the draft in the drive?");
    let held = c
        .runtime
        .state_dir()
        .join(format!("{session}.lost-response-held"));
    wait_for_answer_fixture("the lost response is held", || held.exists());
    c.runtime
        .person_sends(&c.env, &session, "A turn the operator started in T3.");
    c.runtime
        .flag(&format!("{session}.release-lost-response"), true);
    wait_for_answer_fixture("the failed send has an open interval", || {
        interval(&c, &first).is_some()
    });
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let retained = || {
        db.query_row(
            "SELECT content,send_unknown_at FROM conversation_messages WHERE message=?1",
            [&first],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
        )
        .unwrap()
    };
    let original = retained().0;
    c.slack
        .direct("D30", "USTEVEN", "Do we have meeting notes?");
    set_clock(&clock, at + jiff::SignedDuration::from_mins(5));
    answer_source_changed(&c, 2);
    wait_for_answer_fixture(
        "a busy thread releases the unresolved send at the operator bound",
        || retained().1.is_some(),
    );
    assert_eq!(retained().0, original);
    assert!(conversation_finding(&c, &format!("delivery unknown for message {first}")).is_some());
    assert!(interval(&c, &first).is_none());
    assert_eq!(c.runtime.message_count(&session), 2);
    finish_answer_turn(&mut c, &session, 1, "NOOP");
    wait_for_answer_fixture(
        "later work proceeds without retrying the unknown send",
        || c.runtime.message_count(&session) == 3,
    );
    assert!(c.input(&session, 2).contains("Do we have meeting notes?"));
    assert!(!c.input(&session, 2).contains("Is the draft in the drive?"));
    finish_answer_turn(&mut c, &session, 2, "The notes are saved.");
    wait_for_answer_fixture("the later question's answer is recorded", || {
        c.slack.posts_beginning("The notes are saved.") == 1 && answers_recorded(&c)
    });
    // The native runtime finally exposes the original input and its answer.
    c.runtime.person_sends(&c.env, &session, &original);
    finish_answer_turn(&mut c, &session, 3, "Yes, the draft is in the drive.");
    wait_for_answer_fixture(
        "the late answer posts and clears the unknown delivery",
        || {
            c.slack.posts_beginning("Yes, the draft is in the drive.") == 1
                && retained().1.is_none()
        },
    );
    assert_eq!(c.runtime.message_count(&session), 4);
}

fn failed_work_is_retried_after_a_restart() {
    let (mut c, clock) = answer_recovery_company(true);
    let channel = "C1";
    let parent = c
        .slack
        .say_at("1700000000.000100", channel, "USTEVEN", "Release", None);
    projected(&c);
    c.slack.mention(
        channel,
        "USTEVEN",
        "<@UAGENT> Check readiness.",
        Some(&parent),
    );
    let session = started(&c, &[]);
    finish_answer_turn(&mut c, &session, 0, "Ready to review.");
    wait_for_answer_fixture("the opening answer is recorded", || {
        c.slack.posts_beginning("Ready to review.") == 1 && answers_recorded(&c)
    });
    c.runtime.hold_show_after_send(&session);
    let first = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Can we ship?", Some(&parent));
    wait_for_answer_fixture("the acknowledgement of the old attempt is held", || {
        c.runtime.held_show_running(&session)
    });
    let other = c.slack.mention(
        channel,
        "UANNA",
        "<@UAGENT> What is the version?",
        Some(&parent),
    );
    wait_for_answer_fixture("the second member's question is pending", || {
        retained_conversations(&c)
            .conversations
            .iter()
            .any(|row| row.pending_messages == 2)
    });
    let original = c.runtime.message(&session, 1);
    c.runtime.set_status(&session, "idle");
    c.store.failed_turn_with_progress(
        &session,
        "failed-release",
        2,
        &original,
        "Still checking the release.",
    );
    answer_source_changed(&c, 2);
    let state = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    wait_for_answer_fixture(
        "the failed attempt is recorded while its acknowledgement is held",
        || {
            state
                .query_row(
                    "SELECT state='failed' FROM conversation_messages WHERE message=?1",
                    [&first],
                    |r| r.get::<_, bool>(0),
                )
                .unwrap()
        },
    );
    assert_eq!(c.slack.posts().len(), 1);
    let failed_at = interval_at(&c, &first);
    std::fs::remove_file(c.runtime.state_dir().join(format!("{session}.hold-show"))).unwrap();
    wait_for_answer_fixture(
        "the operator log records the discarded acknowledgement",
        || run_log(&c).contains("event=conversation_stale_acknowledgement"),
    );
    assert!(state.query_row(
        "SELECT state='failed' AND retry_at IS NOT NULL FROM conversation_messages WHERE message=?1",
        [&first],
        |r| r.get::<_, bool>(0),
    ).unwrap());
    wait_for_answer_fixture(
        "the newer question reaches the runtime ahead of the retry",
        || c.runtime.message_count(&session) == 3,
    );
    assert!(
        c.runtime
            .message(&session, 2)
            .contains("What is the version?")
    );
    // Archive before publishing the completed reply. The failed question
    // still waits there when it becomes eligible for redelivery.
    c.runtime.archive(&session);
    c.store.thread(
        &session,
        None,
        Some("2026-11-19T10:01:00Z"),
        "2026-11-19T10:01:00Z",
    );
    c.finish_projected_turn(&session, 2, "Version two.");
    answer_source_changed(&c, 3);
    wait_for_answer_fixture(
        "the other member's completed answer posts from the archived thread",
        || {
            c.slack.posts_beginning("Version two.") == 1
                && answers_recorded(&c)
                && c.slack.status(channel, &other).is_none()
        },
    );
    assert_eq!(notices(&c, "USTEVEN"), 0);
    let schedule =
        || {
            state.query_row(
        "SELECT retry_at FROM conversation_messages WHERE message=?1 AND state='failed'",
        [&first],
        |row| row.get::<_, String>(0),
    ).unwrap()
        };
    let before_restart = schedule();
    c.runner_dies();
    c.run();
    assert_eq!(schedule(), before_restart);
    set_clock(&clock, failed_at + jiff::SignedDuration::from_mins(3));
    wait_for_answer_fixture(
        "the failed question waits at the archive door and earns one notice",
        || {
            notices(&c, "USTEVEN") == 1
                && retained_conversations(&c).conversations.iter().any(|row| {
                    row.failures
                        .iter()
                        .any(|failure| failure.contains("unarchive it or delete it"))
                })
        },
    );
    assert_eq!(c.runtime.message_count(&session), 3);
    assert_eq!(c.sessions().as_slice(), std::slice::from_ref(&session));

    c.runtime.change(&session, "archived", false.into());
    c.runtime.change(&session, "title", "Renamed in T3".into());
    c.runtime.flag("send-failure", true);
    c.store.thread(&session, None, None, "2026-11-19T10:03:00Z");
    answer_source_changed(&c, 4);
    wait_for_answer_fixture("the first redelivery is refused by the runtime", || {
        retained_conversations(&c).conversations.iter().any(|row| {
            row.failures
                .iter()
                .any(|failure| failure.starts_with("t3 send:"))
        })
    });
    c.runtime.flag("send-failure", false);
    wait_for_answer_fixture(
        "the retry reaches the same session after the refusal",
        || c.runtime.message_count(&session) == 4,
    );
    let retried = c.runtime.message(&session, 3);
    assert!(retried.contains("Can we ship?"));
    assert_ne!(retried, original, "redelivery has its own boundary");
    c.finish_turn(&session, 3, "Ship on Friday.");
    answer_source_changed(&c, 5);
    wait_for_answer_fixture(
        "the recovered answer closes the first member's outage",
        || {
            c.slack.posts_beginning("Ship on Friday.") == 1
                && answers_recorded(&c)
                && notice_state(&c, &format!("{channel}/{parent}"), "USTEVEN").is_none()
        },
    );
    assert_eq!(c.runtime.message_count(&session), 4);
    assert_eq!(notices(&c, "USTEVEN"), 1);
    assert_eq!(
        destinations(&c),
        vec![(channel.to_owned(), Some(parent)); 4]
    );
    insta::assert_snapshot!(complete_post_bodies(&c), @r#"
[
  [
    null,
    "Ready to review."
  ],
  [
    null,
    "Version two."
  ],
  [
    null,
    "<@USTEVEN> The assistant is temporarily unavailable. Your message is saved and you'll get the answer here when it's back — no need to send it again."
  ],
  [
    null,
    "Ship on Friday."
  ]
]
"#);
}

#[cfg(debug_assertions)]
fn a_replacement_reports_the_old_acknowledgement() {
    let (mut c, _, session) = answered_conversation_with_logs(false, true);
    let hold = c.env.path("ack-hold");
    let key = conversations::key(&c.company(), SLACK, "D30");
    std::fs::write(&hold, &key).unwrap();
    c.runtime.hold_show_after_send(&session);
    let question = c.slack.direct("D30", "USTEVEN", "What changed?");
    wait_for_answer_fixture("the old input read-back is held", || {
        c.runtime.held_show_running(&session)
    });
    std::fs::remove_file(c.runtime.state_dir().join(format!("{session}.hold-show"))).unwrap();
    wait_for_answer_fixture("the relay holds the completed acknowledgement", || {
        hold.with_extension("observed").exists()
    });

    c.runtime.delete(&session);
    c.store.unproject_thread(&session);
    c.store.thread("s2", None, None, "2026-11-08T10:01:00Z");
    c.env
        .run(
            c.env.home(),
            [
                "sessions",
                "replace-deleted",
                &key,
                "--stopped",
                &format!("t3:{session}"),
            ],
        )
        .success();
    std::fs::remove_file(hold).unwrap();

    deliver_probe_wake(&c, &|revision| answer_source_changed(&c, revision), 2);
    assert!(
        run_log(&c).lines().any(|line| {
            line.contains("event=conversation_stale_acknowledgement") && line.contains(&question)
        }),
        "adopting the replacement records the stale acknowledgement"
    );
    finish_answer_turn(&mut c, "s2", 0, "Replacement ready.");
    wait_for_answer_fixture("the replacement receives the retained question", || {
        c.runtime.message_count("s2") == 2
    });
    assert!(c.runtime.message("s2", 1).contains("What changed?"));
}

fn complete_post_bodies(c: &Company) -> String {
    let posts = c.slack.posts();
    let bodies = posts
        .iter()
        .map(|post| (post.text.as_deref(), post.markdown.as_deref()))
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&bodies).unwrap()
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV21
#[test]
fn a_member_keeps_their_answer_through_a_source_interruption() {
    use instrukt_core::state_db::rusqlite::Connection;

    let (mut c, _clock) = answer_recovery_company(true);
    let channel = "D30";
    projected(&c);
    let request = c
        .slack
        .direct(channel, "USTEVEN", "Which page has the summary?");
    let session = started(&c, &[]);
    let input = c.runtime.message(&session, 0);
    let partial = "The summary is on";
    c.runtime.finish(&session, "idle", partial);
    c.runtime.project(&c.env, &session);
    c.store
        .streaming_turn(&session, "incomplete-summary", 1, &input, partial);
    answer_source_changed(&c, 1);
    wait_for_answer_fixture("the relay sees completed but unstable content", || {
        interval(&c, &request).is_some()
            && retained_conversations(&c).conversations.iter().any(|row| {
                row.failures
                    .iter()
                    .any(|failure| failure.contains("content is unstable"))
            })
    });
    assert!(c.slack.posts().is_empty());

    let original = Connection::open(&c.store.file).unwrap();
    let finish = |db: &Connection, answer: &str| {
        db.execute(
            "UPDATE orchestration_v2_projection_messages SET streaming=0,
                 payload_json=json_set(payload_json, '$.streaming', json('false'), '$.text', ?1)
             WHERE message_id='incomplete-summary-reply'",
            [answer],
        )
        .unwrap();
    };
    finish(&original, "The summary is on page two.");
    answer_source_changed(&c, 2);
    wait_for_answer_fixture("the turn finishes in the original store", || {
        c.slack.posts_beginning("The summary is on page two.") == 1 && answers_recorded(&c)
    });

    projected(&c);
    c.slack.direct("D31", "UANNA", "Check the document.");
    let other = started(&c, std::slice::from_ref(&session));
    c.finish_turn(&other, 0, "NOOP");
    answer_source_changed(&c, 3);
    let moved_request = c.slack.direct(channel, "USTEVEN", "Where is the appendix?");
    let state = Connection::open(c.env.instrukt_home().join("state/instrukt.db")).unwrap();
    wait_for_answer_fixture("the appendix question is accepted", || {
        state
            .query_row(
                "SELECT state='sent' FROM conversation_messages WHERE message=?1",
                [&moved_request],
                |r| r.get::<_, bool>(0),
            )
            .is_ok_and(|sent| sent)
    });
    projected(&c);
    let missing_request = c.slack.direct("D32", "USTEVEN", "Is the checklist ready?");
    let missing = started(&c, &[session.clone(), other.clone()]);
    wait_for_answer_fixture(
        "a third conversation's turn is accepted before the move",
        || {
            state
                .query_row(
                    "SELECT state='sent' FROM conversation_messages WHERE message=?1",
                    [&missing_request],
                    |r| r.get::<_, bool>(0),
                )
                .is_ok_and(|sent| sent)
        },
    );
    c.runtime.flag("die-after-send", true);
    let lost = c.slack.direct("D31", "UANNA", "What did you find?");
    c.runner_ended();
    assert_eq!(c.runtime.message_count(&other), 2);
    let lost_state: String = state
        .query_row(
            "SELECT state FROM conversation_messages WHERE message=?1 AND attempted_at IS NOT NULL",
            [&lost],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(lost_state, "pending");

    let answer = "The appendix is at the end.";
    c.runtime.set_status(&session, "idle");
    c.runtime.set_status(&other, "idle");
    c.runtime.set_status(&missing, "idle");
    let appendix_input = c.runtime.message(&session, 2);
    c.store
        .turn(&session, "after-move", 2, &appendix_input, answer);
    let moved = c.env.path("moved-t3");
    std::fs::create_dir_all(moved.join("userdata")).unwrap();
    let destination = moved.join("userdata/statev2.sqlite");
    original
        .execute("VACUUM INTO ?1", [destination.to_str().unwrap()])
        .unwrap();
    let mut source = Connection::open(destination).unwrap();
    source.pragma_update(None, "journal_mode", "WAL").unwrap();
    let tx = source.transaction().unwrap();
    tx.execute(
        "DELETE FROM orchestration_v2_projection_messages WHERE message_id IN (?1,?2)",
        [
            format!("fake:{other}:input:1"),
            format!("fake:{missing}:input:0"),
        ],
    )
    .unwrap();
    tx.execute(
        "DELETE FROM orchestration_v2_projection_runs WHERE thread_id IN (?1,?2,?3) AND run_id LIKE 'run:%'",
        [&session, &other, &missing],
    ).unwrap();
    // Keep the addressed boundary outside the CLI's latest messages.
    let mut recent: Vec<serde_json::Value> =
        serde_json::from_str(&c.runtime.messages(&session)).unwrap();
    let at = "2026-11-19T10:01:00.000Z";
    for index in 0..150 {
        let id = format!("later-{index:03}");
        let record = serde_json::json!({
            "id": id, "threadId": session, "role": "assistant", "text": "Later commentary.",
            "createdAt": at, "updatedAt": at, "streaming": false
        });
        tx.execute(
            "INSERT INTO orchestration_v2_projection_messages VALUES(?1,?2,'assistant',0,?3,?3,?4)",
            instrukt_core::state_db::rusqlite::params![id, session, at, record.to_string()],
        )
        .unwrap();
        recent.push(serde_json::json!({"role": "assistant", "text": "Later commentary."}));
    }
    c.runtime.change(&session, "messages", recent.into());
    tx.commit().unwrap();
    // Seed a retained position that would skip this exact thread fingerprint
    // if interpreted against the new source. The identity still names the old
    // source. Nothing changes in this thread until the answer is posted.
    use instrukt_runner::runtime::conversations::Scope;
    let operator = instrukt_core::home::Home::at_user_home(c.env.home().into());
    let env = instrukt_core::config::Env::from_pairs([("T3CODE_HOME", moved.to_str().unwrap())]);
    let runtime = instrukt_runner::runtime::adapter(instrukt_core::config::Runtime::T3);
    let projection = runtime.conversation_source(&operator, &env).unwrap();
    let id = instrukt_core::ids::SessionId::parse(&session).unwrap();
    let scope = Scope::Only(std::slice::from_ref(&id));
    let cursor = projection.changed_since(None, scope).unwrap().position;
    assert!(
        projection
            .changed_since(Some(&cursor), scope)
            .unwrap()
            .completed_turns
            .is_empty()
    );
    state
        .execute(
            "UPDATE connected_conversations SET position_json=?1 WHERE conversation='D30'",
            [serde_json::to_string(&cursor).unwrap()],
        )
        .unwrap();

    c.env.set_env("T3CODE_HOME", moved.to_str().unwrap());
    c.run();
    wait_for_answer_fixture(
        "the moved source supplies the answer despite the matching old cursor",
        || c.slack.posts_beginning(answer) == 1 && answers_recorded(&c),
    );
    wait_for_answer_fixture("the lost acceptance is still uncertain", || {
        retained_conversations(&c).conversations.iter().any(|row| {
            row.conversation == "D31"
                && row
                    .failures
                    .iter()
                    .any(|failure| failure.contains("cannot find the outstanding attempt"))
        })
    });
    wait_for_answer_fixture(
        "the accepted turn also stays uncertain without its source",
        || {
            state
                .query_row(
                    "SELECT state='sent' AND source_uncertain=1 AND unavailable_since IS NOT NULL
             FROM conversation_messages WHERE message=?1",
                    [&missing_request],
                    |r| r.get::<_, bool>(0),
                )
                .unwrap()
        },
    );
    assert_eq!(c.slack.posts_beginning("The checklist is ready."), 0);
    assert_eq!(c.runtime.message_count(&missing), 1);

    // An unrelated native turn can run while the send with a lost
    // acknowledgement remains uncertain. Wait for the relay to finish a
    // pass over these records before checking that nothing cleared its status.
    wait_for_answer_fixture("the lost attempt shows as unavailable", || {
        c.slack.status("D31", &lost).as_deref() == Some(UNAVAILABLE)
    });
    let opened = interval(&c, &lost).unwrap();
    let tx = source.transaction().unwrap();
    tx.execute(
        "INSERT INTO orchestration_v2_projection_messages
         VALUES('unrelated-input',?1,'user',0,?2,?2,json_object('id','unrelated-input',
                'threadId',?1,'role','user','text','Direct native work','createdAt',?2,'updatedAt',?2,
                'streaming',json('false')))",
        [&other, at],
    ).unwrap();
    tx.execute(
        "INSERT INTO orchestration_v2_projection_runs
         VALUES('unrelated-running',?1,'running',?2,NULL,json_object('id','unrelated-running',
                'threadId',?1,'userMessageId','unrelated-input','status','running','requestedAt',?2))",
        [&other, at],
    ).unwrap();
    tx.commit().unwrap();
    #[cfg(debug_assertions)]
    deliver_probe_wake(&c, &|revision| answer_source_changed(&c, revision), 4);
    assert!(
        state
            .query_row(
                "SELECT failure IS NULL FROM connected_conversations WHERE conversation='D31'",
                [],
                |r| r.get::<_, bool>(0),
            )
            .unwrap(),
        "the moved projection with the unrelated turn must be readable"
    );
    assert_eq!(interval(&c, &lost).as_deref(), Some(opened.as_str()));
    assert_eq!(c.slack.status("D31", &lost).as_deref(), Some(UNAVAILABLE));
    assert_eq!(c.runtime.message_count(&other), 2);
    assert_eq!(c.runtime.message_count(&session), 153);

    // The original input and its completion become visible in the new store.
    c.store.turn(
        &other,
        "lost-answer",
        2,
        &c.runtime.message(&other, 1),
        "The document is complete.",
    );
    source
        .execute(
            "ATTACH DATABASE ?1 AS old",
            [c.store.file.to_str().unwrap()],
        )
        .unwrap();
    source.execute_batch(
        "BEGIN;
         DELETE FROM orchestration_v2_projection_runs WHERE run_id='unrelated-running';
         INSERT INTO orchestration_v2_projection_runs SELECT * FROM old.orchestration_v2_projection_runs WHERE run_id='lost-answer';
         INSERT INTO orchestration_v2_projection_messages SELECT * FROM old.orchestration_v2_projection_messages WHERE message_id LIKE 'lost-answer-%';
         INSERT INTO orchestration_v2_projection_turn_items SELECT * FROM old.orchestration_v2_projection_turn_items WHERE turn_item_id LIKE 'lost-answer-%';
         COMMIT;",
    ).unwrap();
    answer_source_changed(&c, 5);
    wait_for_answer_fixture("the original send's reply arrives once", || {
        c.slack.posts_beginning("The document is complete.") == 1 && answers_recorded(&c)
    });
    assert_eq!(c.runtime.message_count(&other), 2);
    c.store.turn(
        &missing,
        "missing-answer",
        1,
        &c.runtime.message(&missing, 0),
        "The checklist is ready.",
    );
    source.execute_batch(
        "BEGIN;
         INSERT INTO orchestration_v2_projection_runs SELECT * FROM old.orchestration_v2_projection_runs WHERE run_id='missing-answer';
         INSERT INTO orchestration_v2_projection_messages SELECT * FROM old.orchestration_v2_projection_messages WHERE message_id LIKE 'missing-answer-%';
         INSERT INTO orchestration_v2_projection_turn_items SELECT * FROM old.orchestration_v2_projection_turn_items WHERE turn_item_id LIKE 'missing-answer-%';
         COMMIT;",
    ).unwrap();
    answer_source_changed(&c, 6);
    wait_for_answer_fixture("the missing accepted turn's reply arrives once", || {
        c.slack.posts_beginning("The checklist is ready.") == 1 && answers_recorded(&c)
    });
    assert_eq!(c.runtime.message_count(&missing), 1);
    assert_eq!(
        destinations(&c),
        [
            (channel.to_owned(), None),
            (channel.to_owned(), None),
            ("D31".to_owned(), None),
            ("D32".to_owned(), None),
        ]
    );
    insta::assert_snapshot!(complete_post_bodies(&c), @r#"
[
  [
    null,
    "The summary is on page two."
  ],
  [
    null,
    "The appendix is at the end."
  ],
  [
    null,
    "The document is complete."
  ],
  [
    null,
    "The checklist is ready."
  ]
]
"#);
}

fn answered_conversation(settle_opening: bool) -> (Company, std::path::PathBuf, String) {
    answered_conversation_with_logs(settle_opening, false)
}

fn answered_conversation_with_logs(
    settle_opening: bool,
    debug: bool,
) -> (Company, std::path::PathBuf, String) {
    let (mut c, clock) = answer_recovery_company(debug);
    projected(&c);
    c.slack.direct("D30", "USTEVEN", "Is the old report ready?");
    let session = started(&c, &[]);
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let opening_settled = |watching: bool| {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM runtime_inputs WHERE session=?1 AND state='entered' AND watch_run=?2)",
            instrukt_core::state_db::rusqlite::params![session, watching],
            |row| row.get::<_, bool>(0),
        ).unwrap()
    };
    if settle_opening {
        wait_for_answer_fixture("the opening assignment has been recorded", || {
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM runtime_inputs WHERE session=?1)",
                [&session],
                |row| row.get::<_, bool>(0),
            )
            .unwrap()
        });
        set_clock(
            &clock,
            interval_clock(&clock) + jiff::SignedDuration::from_mins(5),
        );
        wait_for_answer_fixture(
            "the keyed worker records native entry of the opening assignment",
            || opening_settled(true),
        );
    }
    c.finish_turn(&session, 0, "The old report is ready.");
    answer_source_changed(&c, 1);
    wait_for_answer_fixture("the earlier answer recorded", || {
        c.slack.posts_beginning("The old report is ready.") == 1 && answers_recorded(&c)
    });
    if settle_opening {
        set_clock(
            &clock,
            interval_clock(&clock) + jiff::SignedDuration::from_mins(5),
        );
        wait_for_answer_fixture(
            "the keyed worker finishes watching the opening assignment",
            || opening_settled(false),
        );
    }
    (c, clock, session)
}

fn wait_for_presence_notice(c: &Company, message: &str, clock: &std::path::Path, door: &str) {
    wait_for_answer_fixture("the operator sees the recovery door", || {
        retained_conversations(c).conversations.iter().any(|row| {
            row.conversation == "D30" && row.failures.iter().any(|cause| cause.contains(door))
        })
    });
    assert!(c.conversations().conversations.iter().any(|row| {
        row.conversation == "D30" && row.failures.iter().any(|cause| cause.contains(door))
    }));
    wait_for_answer_fixture("the member sees only the unavailable text", || {
        c.slack.status("D30", message).as_deref() == Some(UNAVAILABLE)
    });
    let opened = interval_at(c, message);
    set_clock(clock, opened + jiff::SignedDuration::from_secs(59));
    assert_eq!(notices(c, "USTEVEN"), 0);
    set_clock(clock, opened + minute());
    wait_for_answer_fixture("the member receives the fixed notice", || {
        notices(c, "USTEVEN") == 1
    });
}

fn held_runtime_checks_leave_another_conversation_free() {
    let (mut c, _, session) = answered_conversation(true);
    let held = c.runtime.state_dir().join(format!("{session}.hold-show"));
    let marker = c.runtime.state_dir().join(format!("{session}.show-held"));
    std::fs::write(&held, "").unwrap();
    c.slack.direct("D30", "USTEVEN", "What changed?");
    wait_for_answer_fixture("the bound session's dispatch check is held", || {
        c.runtime.held_show_running(&session)
    });

    c.store.thread("s2", None, None, "2026-11-08T10:00:00Z");
    c.slack.direct("D31", "UANNA", "Where is the meeting?");
    wait_for_answer_fixture(
        "the other conversation starts during the dispatch check",
        || c.sessions().iter().any(|id| id != &session),
    );
    let other = started(&c, std::slice::from_ref(&session));
    c.finish_turn(&other, 0, "In the main office.");
    answer_source_changed(&c, 2);
    wait_for_answer_fixture(
        "the second conversation is answered during the dispatch check",
        || c.slack.posts_beginning("In the main office.") == 1,
    );
    assert!(c.runtime.held_show_running(&session));
    assert_eq!(c.runtime.message_count(&session), 1);

    c.runtime.hold_show_after_send(&session);
    std::fs::remove_file(&marker).unwrap();
    std::fs::remove_file(&held).unwrap();
    wait_for_answer_fixture("the lost acknowledgment's read-back check is held", || {
        c.runtime.held_show_running(&session)
    });
    c.slack.direct("D31", "UANNA", "When does it start?");
    wait_for_answer_fixture("the other conversation dispatches during read-back", || {
        c.runtime.message_count(&other) == 2
    });
    assert!(c.runtime.held_show_running(&session));
    c.finish_turn(&other, 1, "At noon.");
    std::fs::remove_file(&held).unwrap();

    c.finish_turn(&session, 1, "The changes are in the appendix.");
    answer_source_changed(&c, 3);
    wait_for_answer_fixture(
        "the lost acknowledgment reconciles without another send",
        || c.slack.posts_beginning("The changes are in the appendix.") == 1 && answers_recorded(&c),
    );
    assert_eq!(c.runtime.message_count(&session), 2);

    // T3's projection remains absent throughout. The failed send proves
    // the runtime already confirmed that the thread was live.
    let sends = || {
        let send = format!("thread send -- {session} -");
        c.runtime
            .calls()
            .iter()
            .filter(|call| **call == send)
            .count()
    };
    let before = sends();
    c.store.unproject_thread(&session);
    c.runtime.flag("send-failure", true);
    let question = c.slack.direct("D30", "USTEVEN", "Who wrote it?");
    wait_for_answer_fixture(
        "a send is attempted after T3 confirms the thread is present",
        || sends() > before,
    );
    c.runtime.delete(&session);
    let key = conversations::key(&c.company(), SLACK, "D30");
    let door = format!("instrukt sessions replace-deleted {key} --stopped t3:{session}");
    wait_for_answer_fixture("the later deletion replaces the live confirmation", || {
        conversation_finding(&c, &door).is_some()
            && c.slack.status("D30", &question).as_deref() == Some(UNAVAILABLE)
    });
}

#[cfg(debug_assertions)]
fn a_live_deletion_is_confirmed_once_across_relay_passes() {
    let (c, clock, session) = answered_conversation_with_logs(true, true);
    let wake = |revision: u64| answer_source_changed(&c, revision);
    let presence_hold = c.env.path("presence-hold");
    std::fs::write(
        &presence_hold,
        conversations::key(&c.company(), SLACK, "D30"),
    )
    .unwrap();
    wake(2);
    wait_for_answer_fixture("the relay has read the old presence", || {
        presence_hold.with_extension("observed").exists()
    });
    let absence_calls = |c: &Company| {
        let show = format!("thread show --json --messages 0 -- {session}");
        c.runtime
            .calls()
            .into_iter()
            .filter(|call| call == &show)
            .count()
    };
    let before_deletion = absence_calls(&c);
    c.runtime.flag("show-failure", true);
    c.runtime.delete(&session);
    c.store.unproject_thread(&session);
    let question = c.slack.direct("D30", "USTEVEN", "What changed?");
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    wait_for_answer_fixture(
        "intake captures the question after the thread disappears",
        || {
            db.query_row("SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1 AND state='pending')", [&question], |row| row.get::<_, bool>(0)).unwrap()
        },
    );
    std::fs::remove_file(&presence_hold).unwrap();
    wait_for_answer_fixture("the failed confirmation is scheduled for retry", || {
        run_log(&c).contains("event=conversation_presence_probe_retry")
    });
    assert_eq!(
        absence_calls(&c) - before_deletion,
        1,
        "only the absence worker may check the thread"
    );
    c.runtime.flag("show-failure", false);
    deliver_probe_wake(&c, &wake, 3);
    assert_eq!(
        absence_calls(&c) - before_deletion,
        1,
        "the failed probe backs off"
    );

    let held = c.runtime.state_dir().join(format!("{session}.hold-show"));
    std::fs::write(&held, "").unwrap();
    set_clock(
        &clock,
        interval_clock(&clock) + jiff::SignedDuration::from_secs(2),
    );
    let marker = c.runtime.state_dir().join(format!("{session}.show-held"));
    wait_for_answer_fixture("T3 recovers and the next confirmation is held", || {
        marker.exists()
    });

    let held_calls = absence_calls(&c);
    for (revision, present) in [(4, true), (5, false)] {
        if present {
            c.store.thread(&session, None, None, "2026-11-08T10:00:00Z");
        } else {
            c.store.unproject_thread(&session);
        }
        deliver_probe_wake(&c, &wake, revision);
    }
    assert_eq!(
        absence_calls(&c),
        held_calls,
        "only one probe while the projection flips"
    );
    std::fs::remove_file(held).unwrap();
    let stopped = format!("--stopped t3:{session}");
    wait_for_answer_fixture(
        "the missing thread has its confirmed replacement finding",
        || conversation_finding(&c, &stopped).is_some(),
    );
    deliver_probe_wake(&c, &wake, 6);
    assert_eq!(
        absence_calls(&c) - before_deletion,
        3,
        "one failure, one invalidated probe and one fresh confirmation"
    );

    assert_eq!(
        run_log(&c)
            .matches("event=conversation_presence_probe_confirmed")
            .count(),
        1
    );
    assert!(conversation_finding(&c, &stopped).is_some());
}

#[cfg(debug_assertions)]
fn deliver_probe_wake(c: &Company, wake: &impl Fn(u64), revision: u64) {
    wake(revision);
    let marker = format!("event=t3_fixture_wake revision={revision}");
    wait_for_answer_fixture("the relay processes the queued source wake", || {
        run_log(c)
            .split_once(&marker)
            .is_some_and(|(_, after)| after.contains("event=conversation_relay_source_processed"))
    });
}

fn conversation_finding(c: &Company, expected: &str) -> Option<String> {
    retained_conversations(c)
        .conversations
        .into_iter()
        .filter(|row| row.conversation == "D30")
        .flat_map(|row| row.failures)
        .find(|failure| failure.contains(expected))
}

fn run_printed_replacement(c: &Company, expected: &str) {
    wait_for_answer_fixture(
        "the operator finding contains the replacement command",
        || conversation_finding(c, expected).is_some(),
    );
    let finding = conversation_finding(c, expected).unwrap();
    replay_replacement(c, &finding);
}

fn replay_replacement(c: &Company, finding: &str) {
    let (_, command) = finding
        .split_once("`instrukt sessions replace-deleted ")
        .unwrap();
    let (arguments, _) = command.split_once('`').unwrap();
    let command = format!("instrukt sessions replace-deleted {arguments}");
    let result = c
        .env
        .recovery_shell(c.env.home(), &command)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
}

fn archived_and_deleted_conversations_keep_saved_work() {
    let (mut c, _, session) = answered_conversation(false);
    c.runtime.archive(&session);
    c.store.thread(
        &session,
        None,
        Some("2026-11-19T10:01:00Z"),
        "2026-11-19T10:01:00Z",
    );
    answer_source_changed(&c, 2);
    let waiting = c.slack.direct("D30", "USTEVEN", "Who kept the notes?");
    wait_for_answer_fixture(
        "the archived thread waits for the operator to unarchive it",
        || {
            conversation_finding(&c, "unarchive it or delete it").is_some()
                && c.slack.status("D30", &waiting).as_deref() == Some(UNAVAILABLE)
        },
    );
    assert_eq!(c.runtime.message_count(&session), 1);
    assert_eq!(c.sessions().as_slice(), std::slice::from_ref(&session));
    c.runtime.change(&session, "archived", false.into());
    c.store.thread(&session, None, None, "2026-11-19T10:02:00Z");
    answer_source_changed(&c, 3);
    wait_for_answer_fixture("the old session receives the question", || {
        c.runtime.message_count(&session) == 2
    });
    let answer = "Ada kept the notes.";
    c.slack.fate(answer, Fate::Refused, usize::MAX);
    finish_answer_turn(&mut c, &session, 1, answer);
    interval_at(&c, &waiting);
    assert_eq!(c.slack.posts_beginning(answer), 0);

    let key = conversations::key(&c.company(), SLACK, "D30");
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    // Delete after readiness in debug builds. The refused send and the
    // retained answer then recover through the same replacement.
    #[cfg(debug_assertions)]
    let hold = {
        let path = c.env.path("dispatch-hold");
        std::fs::write(&path, &key).unwrap();
        path
    };
    #[cfg(not(debug_assertions))]
    c.runtime.delete(&session);
    let next = c.slack.direct("D30", "USTEVEN", "Where is the attachment?");
    #[cfg(debug_assertions)]
    {
        wait_for_answer_fixture(
            "the follow-up has passed readiness but not revalidated dispatch",
            || hold.with_extension("observed").exists(),
        );
        c.runtime.delete(&session);
        std::fs::remove_file(hold).unwrap();
        wait_for_answer_fixture(
            "T3 refuses the send and clears the attempt while keeping the question pending",
            || {
                db.query_row(
                "SELECT state='pending' AND attempted_at IS NULL AND unavailable_since IS NOT NULL AND failure LIKE '%native target is no longer available%' FROM conversation_messages WHERE message=?1",
                [&next],
                |row| row.get::<_, bool>(0),
            ).unwrap()
            },
        );
    }
    c.store.unproject_thread(&session);
    let stopped = format!("t3:{session}");
    let command = format!("instrukt sessions replace-deleted {key} --stopped {stopped}");
    wait_for_answer_fixture(
        "the deletion is confirmed and the question unavailable",
        || {
            conversation_finding(&c, &command).is_some()
                && c.slack.status("D30", &next).as_deref() == Some(UNAVAILABLE)
        },
    );

    c.store.thread("s2", None, None, "2026-11-08T10:00:00Z");
    db.execute_batch(
        "CREATE TRIGGER hold_live_replacement BEFORE UPDATE OF accepted ON runtime_bindings
         WHEN NEW.session='t3:s2' AND NEW.accepted=1
         BEGIN SELECT RAISE(ABORT, 'held live replacement acceptance'); END",
    )
    .unwrap();
    let refused = c
        .env
        .run(
            c.env.home(),
            ["sessions", "replace-deleted", &key, "--stopped", &stopped],
        )
        .failure();
    assert!(
        String::from_utf8_lossy(&refused.get_output().stderr)
            .contains("held live replacement acceptance")
    );
    wait_for_answer_fixture("the finding names the live pending replacement", || {
        conversation_finding(&c, &command)
            .is_some_and(|failure| failure.contains("replacement thread t3:s2"))
    });

    c.slack.clear_fates();
    wait_for_answer_fixture(
        "the retained reply posts before replacement acceptance",
        || c.slack.posts_beginning(answer) == 1 && answers_recorded(&c),
    );
    assert_eq!(c.runtime.message_count("s2"), 1);
    assert!(
        !db.query_row(
            "SELECT accepted FROM runtime_bindings WHERE semantic_key=?1",
            [&key],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
    );
    assert_eq!(interval(&c, &waiting), None);
    db.execute_batch("DROP TRIGGER hold_live_replacement")
        .unwrap();
    run_printed_replacement(&c, &command);
    finish_answer_turn(&mut c, "s2", 0, "The replacement is ready.");
    wait_for_answer_fixture("the live successor receives the waiting question", || {
        c.runtime.message_count("s2") == 2
    });
    assert!(
        c.runtime
            .message("s2", 1)
            .contains("Where is the attachment?")
    );
    finish_answer_turn(&mut c, "s2", 1, "It is in the report.");
    wait_for_answer_fixture("the member receives the successor's reply", || {
        c.slack.posts_beginning("It is in the report.") == 1
            && answers_recorded(&c)
            && c.slack.status("D30", &next).is_none()
    });
    assert_eq!(c.sessions(), ["s2"]);
    assert_eq!(c.slack.posts().len(), 3);
}

fn deleted_conversation_waits_for_confirmed_replacement() {
    let (mut c, clock, session) = answered_conversation(false);
    let first = c
        .slack
        .direct("D30", "USTEVEN", "Who is reviewing the new report?");
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    wait_for_answer_fixture("the old session accepted the unfinished question", || {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1 AND state='sent')",
            [&first],
            |row| row.get::<_, bool>(0),
        )
        .unwrap()
    });
    c.runtime.delete(&session);
    c.store.unproject_thread(&session);
    let second = c.slack.direct("D30", "USTEVEN", "When will it be ready?");
    let key = conversations::key(&c.company(), SLACK, "D30");
    let stopped = format!("t3:{session}");
    let door = format!("instrukt sessions replace-deleted {key} --stopped {stopped}");
    wait_for_answer_fixture(
        "the unfinished first question also shows the fixed status",
        || c.slack.status("D30", &first).as_deref() == Some(UNAVAILABLE),
    );
    wait_for_presence_notice(&c, &second, &clock, &door);
    assert!(c.sessions().is_empty());

    let pending = "s[2]";
    c.runtime.next_thread_id(pending);
    c.store
        .thread(pending, None, None, "2026-11-08T10:00:00.000Z");
    db.execute_batch(
        "CREATE TRIGGER hold_replacement BEFORE UPDATE OF accepted ON runtime_bindings
         WHEN NEW.session='t3:s[2]' AND NEW.accepted=1
         BEGIN SELECT RAISE(ABORT, 'held replacement acceptance'); END",
    )
    .unwrap();
    let replace = ["sessions", "replace-deleted", &key, "--stopped", &stopped];
    #[cfg(debug_assertions)]
    let interrupted = {
        let input = format!("input:{stopped}");
        let held = hold_recovery_key(&c, &input);
        let command = recovery_waiting(&c, replace, &input).unwrap();
        drop(held);
        command.wait_with_output().unwrap()
    };
    #[cfg(not(debug_assertions))]
    let interrupted = c
        .env
        .run(c.env.home(), replace)
        .failure()
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&interrupted.stderr);
    assert!(
        !interrupted.status.success() && stderr.contains("held replacement acceptance"),
        "{stderr}"
    );
    assert_eq!(c.sessions(), [pending]);
    c.runtime.delete(pending);
    c.store.unproject_thread(pending);
    db.execute_batch("DROP TRIGGER hold_replacement").unwrap();

    let pending_door = format!("instrukt sessions replace-deleted {key} --stopped t3:{pending}");
    wait(
        "conversation status names the deleted successor's ID",
        || {
            retained_conversations(&c)
                .conversations
                .iter()
                .flat_map(|row| &row.failures)
                .any(|failure| failure.replace('\'', "").contains(&pending_door))
        },
    );
    let conversation = c
        .conversations()
        .conversations
        .into_iter()
        .flat_map(|row| row.failures)
        .find(|failure| failure.replace('\'', "").contains(&pending_door))
        .unwrap();

    // The refusal records the keyed finding surfaced by plain events status.
    let refusal = c
        .env
        .run(c.env.home(), ["sessions", "resolve-start", &key, "--none"])
        .failure();
    let stderr = String::from_utf8_lossy(&refusal.get_output().stderr);
    let status = c.env.run(c.env.home(), ["events", "status"]).success();
    let events: serde_json::Value = serde_json::from_slice(&status.get_output().stdout).unwrap();
    let finding = events["start_anomalies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["key"].as_str() == Some(&key))
        .and_then(|row| row["failure"].as_str())
        .unwrap();

    c.store.thread("s3", None, None, "2026-11-08T10:00:00Z");
    c.runtime.next_thread_id("s3");
    // A POSIX shell expands unquoted t3:s[2] to this filename.
    std::fs::write(c.env.home().join("t3:s2"), "").unwrap();
    for hint in [stderr.as_ref(), &conversation, finding] {
        replay_replacement(&c, hint);
    }
    let successor = started(&c, &[]);
    assert_ne!(successor, session);
    assert!(
        !instrukt_runner::envelopes::segment(&c.runtime.message(&successor, 0))
            .iter()
            .any(|span| matches!(span, instrukt_runner::envelopes::Span::Member { .. }))
    );
    assert!(
        !c.runtime
            .message(&successor, 0)
            .contains("Is the old report ready?")
    );
    finish_answer_turn(
        &mut c,
        &successor,
        0,
        "The replacement session is initialized.",
    );

    wait_for_answer_fixture(
        "the successor receives the first unanswered question",
        || c.runtime.message_count(&successor) == 2,
    );
    assert!(
        c.runtime
            .message(&successor, 1)
            .contains("Who is reviewing the new report?")
    );
    finish_answer_turn(&mut c, &successor, 1, "Ada is reviewing it.");
    wait_for_answer_fixture("the successor receives the later question in order", || {
        c.runtime.message_count(&successor) == 3
    });
    assert!(
        c.runtime
            .message(&successor, 2)
            .contains("When will it be ready?")
    );
    finish_answer_turn(&mut c, &successor, 2, "It will be ready on Friday.");
    wait_for_answer_fixture("both answers appear and the final status clears", || {
        c.slack.posts_beginning("Ada is reviewing it.") == 1
            && c.slack.posts_beginning("It will be ready on Friday.") == 1
            && answers_recorded(&c)
            && c.slack.status("D30", &second).is_none()
    });
    assert_eq!(notices(&c, "USTEVEN"), 1);
    assert_eq!(c.slack.posts().len(), 4);
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV15
#[test]
fn a_member_can_follow_the_status_of_an_unanswered_message() {
    std::thread::scope(|scope| {
        scope.spawn(a_members_status_tracks_start_and_turn_updates);
        scope.spawn(working_status_clears_for_a_noop_and_carries_queued_turns);
        scope.spawn(unavailable_status_clears_when_the_source_or_post_recovers);
        scope.spawn(a_relay_restart_keeps_the_unavailable_interval);
        scope.spawn(observing_a_running_turn_again_clears_its_and_the_queued_status);
        scope.spawn(a_running_turn_closes_an_observation_failure);
        scope.spawn(a_queued_turn_acceptance_closes_the_unavailable_interval);
        #[cfg(debug_assertions)]
        scope.spawn(a_completed_turn_is_read_again_after_a_locked_read);
    });
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV16
#[test]
fn an_interrupted_answer_is_reconciled_and_the_member_can_keep_asking() {
    std::thread::scope(|scope| {
        // These held-receipt scenarios need the runner's debug decision marker.
        #[cfg(debug_assertions)]
        {
            scope.spawn(a_lost_answer_response_defers_notices_until_read_back);
            scope.spawn(an_unconfirmed_answer_is_reconciled_before_retry);
        }
        scope.spawn(a_successful_answer_without_timestamp_needs_no_read_back);
        scope.spawn(failed_metadata_reads_expire_without_sending_again);
        scope.spawn(an_unconfirmed_post_waits_after_a_metadata_miss);
        scope.spawn(an_answer_can_post_while_its_notice_is_being_reconciled);
        scope.spawn(the_status_tracks_a_slow_answer_and_its_receipt);
        scope.spawn(an_answer_whose_post_was_tried_before_a_restart_shows_the_unavailable_text);
    });
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV17
#[test]
fn an_operator_can_resume_conversations_with_uncertain_answers_after_upgrade() {
    std::thread::scope(|scope| {
        scope.spawn(found_legacy_answer_is_confirmed);
        scope.spawn(missing_legacy_answer_expires);
        scope.spawn(late_legacy_answer_upgrades_unknown_delivery);
        scope.spawn(late_legacy_answer_leaves_a_newer_notice_intact);
        scope.spawn(late_legacy_answer_leaves_an_unconfirmed_newer_notice_intact);
        scope.spawn(legacy_messages_keep_status_and_the_notice_anchor);
    });
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV18
#[test]
fn a_member_can_keep_asking_after_the_assistant_fails_to_answer() {
    std::thread::scope(|scope| {
        scope.spawn(failed_turns_share_a_notice_until_a_later_turn_settles);
        scope.spawn(a_blank_reply_supersedes_a_failed_question);
        scope.spawn(pruning_keeps_younger_and_unsuperseded_failures);
        scope.spawn(one_members_answer_proceeds_after_another_conversations_lock_times_out);
    });
}

fn found_legacy_answer_is_confirmed() {
    legacy_answer_attempt_releases_conversation(LegacyReceipt::Found);
}

fn missing_legacy_answer_expires() {
    legacy_answer_attempt_releases_conversation(LegacyReceipt::Missing);
}

fn late_legacy_answer_upgrades_unknown_delivery() {
    legacy_answer_attempt_releases_conversation(LegacyReceipt::Late);
}

fn late_legacy_answer_leaves_a_newer_notice_intact() {
    legacy_answer_attempt_releases_conversation(LegacyReceipt::LateWithNewNotice);
}

fn late_legacy_answer_leaves_an_unconfirmed_newer_notice_intact() {
    legacy_answer_attempt_releases_conversation(LegacyReceipt::LateWithUnconfirmedNotice);
}

fn successful_empty_turns_rearm_notices() {
    std::thread::scope(|scope| {
        scope.spawn(|| an_empty_success_lets_the_next_question_receive_its_own_notice("NOOP"));
        an_empty_success_lets_the_next_question_receive_its_own_notice(" \n ");
    });
}

#[cfg(debug_assertions)]
fn hold_recovery_key(c: &Company, key: &str) -> instrukt_core::fsx::LockGuard {
    use sha2::{Digest, Sha256};
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());
    let name = Sha256::digest(format!("t3:{key}").as_bytes());
    instrukt_core::fsx::LockGuard::acquire(
        &home.locks_dir().join(format!("runtime-key-{name:x}.lock")),
        std::time::Duration::from_secs(5),
    )
    .unwrap()
}

#[cfg(debug_assertions)]
fn recovery_waiting<const N: usize>(
    c: &Company,
    args: [&str; N],
    key: &str,
) -> Result<std::process::Child, String> {
    let marker = c.env.path("recovery-lock-wait");
    let mut command = c
        .env
        .instrukt_process(c.env.home())
        .env("INSTRUKT_TEST_RECOVERY_WAIT", &marker)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    wait("operator recovery encounters the held lock", || {
        marker.exists() || command.try_wait().unwrap().is_some()
    });
    if command.try_wait().unwrap().is_some() {
        let result = command.wait_with_output().unwrap();
        return Err(format!(
            "operator recovery exited before acquiring {key}: {}",
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    assert_eq!(std::fs::read_to_string(marker).unwrap(), key);
    Ok(command)
}

fn hold_dispatch(c: &Company) -> instrukt_core::fsx::LockGuard {
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());
    instrukt_core::fsx::LockGuard::acquire(
        &home.state_dir().join("runtime-dispatch.lock"),
        std::time::Duration::from_secs(5),
    )
    .unwrap()
}

fn a_members_status_tracks_start_and_turn_updates() {
    let (mut c, _) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "C1";

    // The status tells the state. While the message waits for its start the
    // member sees the working text; once the start's listing is refused, the
    // unavailable text; once the runtime accepts the message, the working
    // text again; while its turn runs, the newest message the agent has
    // written in it, as written; and nothing once the answer is posted.
    let held = hold_dispatch(&c);
    projected(&c);
    let asked = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> When do we ship?", None);
    wait("the working text", || {
        c.slack.status(channel, &asked).as_deref() == Some(WORKING)
    });
    c.runtime.flag("list-failure", true);
    drop(held);
    wait("the unavailable text", || {
        c.slack.status(channel, &asked).as_deref() == Some(UNAVAILABLE)
    });
    c.runtime.flag("list-failure", false);
    let session = started(&c, &[]);
    wait("the working text after the start", || {
        c.slack.status(channel, &asked).as_deref() == Some(WORKING)
    });
    let run = c.runtime.active_run(&session);
    c.store
        .agent_writes(&session, &run, 1, "Looking at the release plan.");
    wait("the agent's message", || {
        c.slack.status(channel, &asked).as_deref() == Some("Looking at the release plan.")
    });
    c.store.agent_writes(
        &session,
        &run,
        2,
        "Two items are still open — *both small*.",
    );
    wait("the agent's newest message", || {
        c.slack.status(channel, &asked).as_deref()
            == Some("Two items are still open — *both small*.")
    });
    c.finish_turn(&session, 0, "On Friday.");
    wait("the answer", || c.slack.posts().len() == 1);
    wait("the status cleared", || {
        shown(&c, channel, &asked).last().map(String::as_str) == Some("")
    });
    // While the answer is posted the status keeps the agent's newest
    // message; it is cleared once the answer is posted.
    insta::assert_json_snapshot!("status-tells-the-state", shown(&c, channel, &asked));
}

fn working_status_stays_visible_during_a_slow_start() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "C1";
    // A start that is slow but healthy shows the working text throughout.
    let before = c.sessions();
    c.runtime.flag("hold-start", true);
    projected(&c);
    let slow = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Who reviews it?", None);
    wait("the working text", || {
        c.slack.status(channel, &slow).as_deref() == Some(WORKING)
    });
    wait("the healthy start held", || c.runtime.start_held());
    set_clock(
        &clock,
        interval_clock(&clock) + jiff::SignedDuration::from_secs(90),
    );
    assert_eq!(interval(&c, &slow), None);
    c.runtime.flag("hold-start", false);
    let session = started(&c, &before);
    c.finish_turn(&session, 0, "Ada reviews it.");
    wait("the slow start's answer", || c.slack.posts().len() == 1);
    wait("the status cleared", || {
        shown(&c, channel, &slow).last().map(String::as_str) == Some("")
    });
    assert_eq!(shown(&c, channel, &slow), [WORKING, ""]);
    assert_eq!(notices(&c, "USTEVEN"), 0);
}

fn a_dispatch_failure_notifies_the_member_once_before_the_answer() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "C1";
    // A message waits on the runtime's dispatch lock past the wait: the
    // working text, then the unavailable text; once the lock is had, the
    // message is started and answered.
    let held = hold_dispatch(&c);
    let before = c.sessions();
    projected(&c);
    let locked = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Is the build green?", None);
    wait("the working text", || {
        c.slack.status(channel, &locked).as_deref() == Some(WORKING)
    });
    // The clock runs until the lock's wait has passed, from whenever the
    // relay began it.
    let running = RunningClock::start(clock.clone(), "2026-11-19T10:00:00Z".parse().unwrap(), 30);
    halt_at_intervals(&c, running, &clock, &[&locked]);
    wait("the unavailable text", || {
        c.slack.status(channel, &locked).as_deref() == Some(UNAVAILABLE)
    });
    set_clock(&clock, interval_at(&c, &locked) + minute());
    wait(
        "the notice a minute after the dispatch lock timed out",
        || notices(&c, "USTEVEN") == 1,
    );
    drop(held);
    let session = started(&c, &before);
    wait("the working text after the start", || {
        c.slack.status(channel, &locked).as_deref() == Some(WORKING)
    });
    c.finish_turn(&session, 0, "It is green.");
    wait("the locked message's answer", || answers(&c) == 1);
    wait("the status cleared", || {
        shown(&c, channel, &locked).last().map(String::as_str) == Some("")
    });
    assert_eq!(
        shown(&c, channel, &locked),
        [WORKING, UNAVAILABLE, WORKING, ""]
    );
}

fn unavailable_status_clears_when_the_source_or_post_recovers() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "C1";

    // The relay cannot read the conversation source after the message was
    // handed over: the unavailable text; once it reads again, the answer.
    let before = c.sessions();
    projected(&c);
    let outage = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Where are the notes?", None);
    let session = started(&c, &before);
    wait("the working text", || {
        c.slack.status(channel, &outage).as_deref() == Some(WORKING)
    });
    c.store.break_shape();
    wait("the unavailable text", || {
        c.slack.status(channel, &outage).as_deref() == Some(UNAVAILABLE)
    });
    c.store.restore_shape();
    wait("the working text once the source reads again", || {
        c.slack.status(channel, &outage).as_deref() == Some(WORKING)
    });
    c.finish_turn(&session, 0, "In the wiki.");
    wait("the answer after the outage", || answers(&c) == 1);
    wait("the status cleared", || {
        shown(&c, channel, &outage).last().map(String::as_str) == Some("")
    });
    assert_eq!(
        shown(&c, channel, &outage),
        [WORKING, UNAVAILABLE, WORKING, ""]
    );

    // The turn completed, but its answer cannot be posted: the status stays,
    // with the unavailable text, until the answer is posted.
    let before = c.sessions();
    projected(&c);
    let unposted = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Who owns billing?", None);
    let session = started(&c, &before);
    wait("the working text", || {
        c.slack.status(channel, &unposted).as_deref() == Some(WORKING)
    });
    c.slack
        .fate("Grace owns billing.", Fate::Refused, usize::MAX);
    c.finish_turn(&session, 0, "Grace owns billing.");
    wait("the unavailable text", || {
        c.slack.status(channel, &unposted).as_deref() == Some(UNAVAILABLE)
    });
    set_clock(&clock, interval_at(&c, &unposted) + minute());
    wait(
        "the notice while the completed answer cannot be posted",
        || notices(&c, "USTEVEN") == 1,
    );
    assert_eq!(answers(&c), 1);
    c.slack.clear_fates();
    wait("the answer once posts are taken", || answers(&c) == 2);
    wait("the status cleared", || {
        shown(&c, channel, &unposted).last().map(String::as_str) == Some("")
    });
    assert_eq!(shown(&c, channel, &unposted), [WORKING, UNAVAILABLE, ""]);
}

fn working_status_clears_for_a_noop_and_carries_queued_turns() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "C1";
    // A turn that completes with a bare `NOOP` completes its message:
    // nothing is posted, and the status is cleared without ever saying the
    // assistant is unavailable.
    let before = c.sessions();
    projected(&c);
    let quiet = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Thanks!", None);
    let session = started(&c, &before);
    wait("the working text", || {
        c.slack.status(channel, &quiet).as_deref() == Some(WORKING)
    });
    c.finish_turn(&session, 0, "NOOP");
    wait("the status cleared", || {
        shown(&c, channel, &quiet).last().map(String::as_str) == Some("")
    });
    assert_eq!(shown(&c, channel, &quiet), [WORKING, ""]);
    set_clock(
        &clock,
        interval_clock(&clock) + jiff::SignedDuration::from_secs(90),
    );
    assert_eq!(answers(&c), 0);
    assert_eq!(notices(&c, "USTEVEN"), 0);

    // A second message waits behind the conversation's running turn: the
    // status shows that turn's work, never the unavailable text, and both
    // are answered in order.
    let before = c.sessions();
    projected(&c);
    let first = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> What is next?", None);
    let session = started(&c, &before);
    let run = c.runtime.active_run(&session);
    c.store
        .agent_writes(&session, &run, 1, "Checking the board.");
    wait("the agent's message", || {
        c.slack.status(channel, &first).as_deref() == Some("Checking the board.")
    });
    c.slack.mention(
        channel,
        "USTEVEN",
        "<@UAGENT> And after that?",
        Some(&first),
    );
    wait("the second message retained", || {
        retained_conversations(&c)
            .conversations
            .iter()
            .any(|conversation| conversation.pending_messages == 2)
    });
    set_clock(
        &clock,
        interval_clock(&clock) + jiff::SignedDuration::from_secs(90),
    );
    assert_eq!(notices(&c, "USTEVEN"), 0);
    c.finish_turn(&session, 0, "The release.");
    wait("the second message handed over", || {
        c.runtime.message_count(&session) == 2
    });
    c.finish_turn(&session, 1, "The roadmap.");
    wait("both answers", || answers(&c) == 2);
    wait("the status cleared", || {
        shown(&c, channel, &first).last().map(String::as_str) == Some("")
    });
    assert!(!shown(&c, channel, &first).contains(&UNAVAILABLE.to_owned()));
    assert_eq!(notices(&c, "USTEVEN"), 0);
}

fn interval_clock(clock: &std::path::Path) -> jiff::Timestamp {
    std::fs::read_to_string(clock).unwrap().parse().unwrap()
}

fn notice_epochs_and_reconciliation() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    c.runtime.flag("separate-completion", true);
    let channel = "C1";
    let sender = "USTEVEN";
    let other = "UANNA";
    projected(&c);
    c.runner_dies();
    let database = c.store.file.clone();
    let away = database.with_extension("away");
    std::fs::rename(&database, &away).unwrap();
    std::fs::create_dir(&database).unwrap();
    c.run();
    let first_at = interval_clock(&clock);
    wait("the relay cannot acquire the source", || {
        relay_since(&c).as_deref() == Some(&first_at.to_string())
    });
    let first = c
        .slack
        .mention(channel, sender, "<@UAGENT> When do we ship?", None);
    let conversation = format!("{channel}/{first}");
    let other_question =
        c.slack
            .mention(channel, other, "<@UAGENT> What do I review?", Some(&first));
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    let admitted = |message: &str| {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1 AND admitted_at IS NOT NULL)",
            [message],
            |row| row.get::<_, bool>(0),
        ).unwrap()
    };
    wait("both members admitted during the outage", || {
        admitted(&first) && admitted(&other_question)
    });
    assert_eq!(interval(&c, &first), None);
    assert_eq!(interval(&c, &other_question), None);
    // The queued follow-up is admitted later and does not enter the
    // first notice's set of already-due messages.
    set_clock(&clock, first_at + jiff::SignedDuration::from_secs(1));
    let again = c
        .slack
        .mention(channel, sender, "<@UAGENT> And what time?", Some(&first));
    wait(
        "the repeated message admitted while the relay is down",
        || admitted(&again),
    );
    c.slack.fate(&notice_text(sender), Fate::Lost, 1);
    c.slack.fate(&notice_text(other), Fate::Refused, 1);
    set_clock(&clock, first_at + minute());
    wait("the committed notice whose response was lost", || {
        notices(&c, sender) == 1
    });
    wait("the other member's refused notice attempt", || {
        notice_record(&c, &conversation, other)
            .is_some_and(|(state, failure)| state == "posting" && failure.is_some())
    });
    assert_eq!(notices(&c, other), 0);
    c.runner_dies();
    let held = c.slack.held();
    c.slack.hold_reads_from(&first);
    set_clock(&clock, first_at + jiff::SignedDuration::from_mins(3));
    c.run();
    wait("the first member's reconciliation read held", || {
        c.slack.held() > held
    });
    common::wait_until(
        "the other member told while the first read is held",
        std::time::Duration::from_secs(12),
        || notices(&c, other) == 1,
    );
    c.slack.release_reads();
    wait("the lost response reconciled after restart", || {
        notice_state(&c, &conversation, sender).as_deref() == Some("posted")
    });
    wait(
        "the other member told after their own attempt did not land",
        || notices(&c, other) == 1,
    );
    assert_eq!(notices(&c, sender), 1);

    c.runner_dies();
    std::fs::remove_dir(&database).unwrap();
    std::fs::rename(&away, &database).unwrap();
    c.run();
    let session = started(&c, &[]);
    wait("the first input accepted", || {
        c.runtime.message_count(&session) == 1
    });
    c.finish_turn(&session, 0, "Friday.");
    wait("the first answer", || {
        c.slack.posts_beginning("Friday.") == 1
    });
    wait("the second input accepted", || {
        c.runtime.message_count(&session) == 2
    });
    c.runtime.flag("send-failure", true);
    c.finish_turn(&session, 1, "The release notes.");
    wait("the second answer", || {
        c.slack.posts_beginning("The release notes.") == 1
    });
    // The next input was queued before the first notice. Its new epoch
    // must disregard that visible notice when the new attempt is refused.
    let queued_at = interval_at(&c, &again);
    c.slack.fate(&notice_text(sender), Fate::Refused, 1);
    set_clock(&clock, queued_at + minute());
    wait("the new attempt for the queued message", || {
        notice_state(&c, &conversation, sender).as_deref() == Some("posting")
    });
    set_clock(&clock, queued_at + jiff::SignedDuration::from_mins(3));
    wait("the new notice after the refused attempt settles", || {
        notices(&c, sender) == 2
    });
    c.runtime.flag("send-failure", false);
    wait("the third input accepted", || {
        c.runtime.message_count(&session) == 3
    });
    c.finish_turn(&session, 2, "At noon.");
    wait("the third answer", || {
        c.slack.posts_beginning("At noon.") == 1
    });
    wait("the members' notice records rearmed", || {
        notice_state(&c, &conversation, sender).is_none()
            && notice_state(&c, &conversation, other).is_none()
    });
    for post in c.slack.posts() {
        assert_eq!(post.channel, channel);
        assert_eq!(post.thread.as_deref(), Some(first.as_str()));
    }
}

fn a_notice_in_flight_reconciles_after_a_noop() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "C1";
    let sender = "USTEVEN";
    c.runtime.flag("separate-completion", true);
    c.runtime.flag("list-failure", true);
    projected(&c);
    let next = c
        .slack
        .mention(channel, sender, "<@UAGENT> Where are the notes?", None);
    let conversation = format!("{channel}/{next}");
    let next_at = interval_at(&c, &next);
    c.slack.fate(&notice_text(sender), Fate::Late, 1);
    set_clock(&clock, next_at + minute());
    wait("the notice sent", || {
        notice_state(&c, &conversation, sender).as_deref() == Some("posting")
    });
    let reads = c.slack.metadata_reads();
    wait(
        "a reconciliation read while the notice is in flight",
        || c.slack.metadata_reads() > reads,
    );
    assert_eq!(notices(&c, sender), 0);
    c.runtime.flag("list-failure", false);
    let session = started(&c, &[]);
    c.finish_turn(&session, 0, "NOOP");
    wait("the completed message leaves no pending message", || {
        retained_conversations(&c)
            .conversations
            .iter()
            .all(|row| row.pending_messages == 0)
    });
    // The NOOP settles this question. Its in-flight notice keeps its
    // identity until the eventual post is reconciled without a duplicate.
    c.slack.commit_late();
    wait("the in-flight notice becomes visible", || {
        notices(&c, sender) == 1
    });
    set_clock(&clock, next_at + jiff::SignedDuration::from_mins(3));
    wait(
        "the late notice reconciled and its settled epoch removed",
        || notice_state(&c, &conversation, sender).is_none(),
    );
    assert_eq!(notices(&c, sender), 1);
    c.slack
        .mention(channel, sender, "<@UAGENT> And the contract?", Some(&next));
    wait("the next input accepted", || {
        c.runtime.message_count(&session) == 2
    });
    c.finish_turn(&session, 1, "In the wiki.");
    wait("the new epoch's answer", || {
        c.slack.posts_beginning("In the wiki.") == 1
    });
    for post in c.slack.posts() {
        assert_eq!(post.channel, channel);
        assert_eq!(post.thread.as_deref(), Some(next.as_str()));
    }
}

fn a_normalized_notice_is_reconciled_after_a_restart() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "D18";
    let member = "USTEVEN";
    projected(&c);
    c.runtime.flag("list-failure", true);
    let asked = c.slack.direct(channel, member, "When do we ship?");
    let started_at = interval_at(&c, &asked);
    let text = notice_text(member);
    c.slack
        .render_post(&text, &text.replace("you'll", "you’ll"));
    let state = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    state.execute_batch(
        "CREATE TRIGGER hold_notice_receipt BEFORE UPDATE OF replied_at ON conversation_refusals
         WHEN OLD.reason='unavailable' AND NEW.state='posted'
         BEGIN SELECT RAISE(ABORT, 'forced notice receipt failure'); END"
    ).unwrap();
    set_clock(&clock, started_at + minute());
    wait("Slack shows the notice and the receipt write fails", || {
        notices(&c, member) == 1 && run_log(&c).contains("forced notice receipt failure")
    });
    assert_eq!(
        notice_state(&c, channel, member).as_deref(),
        Some("posting")
    );
    c.runner_dies();

    // If read-back misses the rewritten text, the notice can be retried
    // immediately at this clock setting.
    set_clock(&clock, started_at + jiff::SignedDuration::from_mins(4));
    state
        .execute_batch("DROP TRIGGER hold_notice_receipt")
        .unwrap();
    c.run();
    wait(
        "the normalized notice receipt reconciled after restart",
        || notice_state(&c, channel, member).as_deref() == Some("posted"),
    );
    assert_eq!(
        notices(&c, member),
        1,
        "the accepted notice must not be posted again"
    );
    c.runtime.flag("list-failure", false);
    let session = started(&c, &[]);
    c.finish_turn(&session, 0, "Friday.");
    wait("the member answered after the notice", || {
        c.slack.posts_beginning("Friday.") == 1
    });
    assert_eq!(notices(&c, member), 1);
}

fn run_log(c: &Company) -> String {
    std::fs::read_to_string(
        c.env
            .home()
            .join(".local/state/instrukt-ai/instrukt/run.log"),
    )
    .unwrap_or_default()
}

/// Make a control member overdue on the operator clock. Seeing their notice
/// confirms eligibility was evaluated while the answer remained unresolved.
fn observe_notice_pass(c: &Company, clock: &std::path::Path, now: jiff::Timestamp) {
    use instrukt_core::state_db::rusqlite::{Connection, params};
    c.runtime.flag("list-failure", true);
    let control = c
        .slack
        .direct("D99", "UANNA", "Is the assistant available?");
    let db = Connection::open(c.env.instrukt_home().join("state/instrukt.db")).unwrap();
    wait("the control member admitted", || {
        db.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1 AND admission_position IS NOT NULL)",
            [&control],
            |row| row.get::<_, bool>(0),
        ).unwrap()
    });
    // The answer relay can be blocked in the provider read. Give the
    // control member a retained failed turn so no dispatch is needed.
    db.execute(
        "UPDATE conversation_messages SET state='failed',unavailable_since=?2 WHERE message=?1",
        params![control, (now - minute()).to_string()],
    )
    .unwrap();
    set_clock(clock, now);
    wait("the control member's notice posted", || {
        notices(c, "UANNA") == 1
    });
}

/// Wait for this member's notice task to finish its decision at the new
/// time. Another member's post does not establish this ordering.
#[cfg(debug_assertions)]
fn wait_for_notice_decision(
    c: &Company,
    conversation: &str,
    member: &str,
    clock: &std::path::Path,
    at: jiff::Timestamp,
) {
    let path = c.env.path("notice-decisions");
    let decision = format!("{conversation}\t{member}");
    let decisions = || {
        std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter(|line| *line == decision)
            .count()
    };
    let before = decisions();
    set_clock(clock, at);
    wait("the held answer's member evaluated for a notice", || {
        decisions() > before
    });
}

#[cfg(debug_assertions)]
fn a_lost_answer_response_defers_notices_until_read_back() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    projected(&c);
    let asked = c.slack.direct("D13", "USTEVEN", "Who signed?");
    let session = started(&c, &[]);
    c.slack.hold_posts();
    c.slack.hold_reads_from(&asked);
    c.finish_turn(&session, 0, "Ada signed.");
    wait("Slack accepts the answer and holds the response", || {
        c.slack.held_posts() == 1
    });
    assert_eq!(c.slack.posts_beginning("Ada signed."), 1);
    c.slack.time_out_held_posts();
    wait("Slack displays the answer before reconciliation", || {
        c.slack.posts_beginning("Ada signed.") == 1 && c.slack.held() > 0
    });
    c.runner_dies();
    let opened = interval_at(&c, &asked);
    let held = c.slack.held();
    c.run();
    wait("the restarted relay reading back the answer", || {
        c.slack.held() > held
    });
    wait_for_notice_decision(&c, "D13", "USTEVEN", &clock, opened + minute());
    assert_eq!(notices(&c, "USTEVEN"), 0, "the answer is already visible");
    assert!(!answers_recorded(&c));
    c.slack.release_reads();
    wait("the answer reconciled", || answers_recorded(&c));
    assert_eq!(c.slack.posts_beginning("Ada signed."), 1);
    assert_eq!(notices(&c, "USTEVEN"), 0);
}

#[cfg(debug_assertions)]
fn an_unconfirmed_answer_is_reconciled_before_retry() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    projected(&c);
    let asked = c.slack.direct("D16", "USTEVEN", "Is the release ready?");
    let session = started(&c, &[]);
    let answer = "The release is ready.";
    c.slack.fate(answer, Fate::Late, 1);
    c.slack.fate(answer, Fate::Refused, usize::MAX);
    c.slack.hold_reads_from(&asked);
    c.finish_turn(&session, 0, answer);
    wait("the first answer's read-back before retrying", || {
        c.slack.held() > 0
    });
    assert!(c.slack.posts().is_empty());
    set_clock(
        &clock,
        interval_clock(&clock) + jiff::SignedDuration::from_secs(5),
    );
    let reads = c.slack.reads();
    c.slack.release_reads();
    wait("an empty read-back inside the post timeout window", || {
        c.slack.reads() > reads
    });
    assert!(c.slack.posts().is_empty());
    let opened = interval_at(&c, &asked);
    let held = c.slack.held();
    c.slack.hold_reads_from(&asked);
    c.slack.commit_late();
    wait("the later read-back is held", || c.slack.held() > held);
    assert_eq!(c.slack.posts_beginning(answer), 1);
    wait_for_notice_decision(&c, "D16", "USTEVEN", &clock, opened + minute());
    assert_eq!(
        notices(&c, "USTEVEN"),
        0,
        "the earlier answer already landed"
    );
    c.slack.release_reads();
    wait(
        "the earlier answer reconciled without an early retry",
        || answers_recorded(&c),
    );
    assert_eq!(c.slack.posts_beginning(answer), 1);
    assert_eq!(notices(&c, "USTEVEN"), 0);

    // An empty read after the timeout can settle a post that never appears.
    // A confirmed rejection then permits a notice.
    let before = c.sessions();
    projected(&c);
    let absent = c.slack.direct("D17", "USTEVEN", "Who is on call?");
    let session = started(&c, &before);
    c.slack.fate("Ben.", Fate::Late, 1);
    c.slack.fate("Ben.", Fate::Refused, usize::MAX);
    c.finish_turn(&session, 0, "Ben.");
    let opened = interval_at(&c, &absent);
    set_clock(&clock, opened + minute());
    wait(
        "the member notified after read-back establishes absence",
        || notices(&c, "USTEVEN") == 1,
    );
    assert_eq!(c.slack.posts_beginning("Ben."), 0);
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LegacyReceipt {
    Found,
    Missing,
    Late,
    LateWithNewNotice,
    LateWithUnconfirmedNotice,
}

/// Seed the interrupted answer and its conversation into the schema shipped
/// before metadata receipts. The runner opening this file owns its upgrade.
fn seed_v22_answer_attempt(
    c: &Company,
    conversation: &str,
) -> instrukt_core::state_db::rusqlite::Connection {
    use instrukt_core::state_db::rusqlite::Connection;
    let state_dir = c.env.instrukt_home().join("state");
    let state_path = state_dir.join("instrukt.db");
    let seed_path = c.env.path("state-v22.sqlite");
    let legacy = Connection::open(&seed_path).unwrap();
    legacy
        .execute_batch(include_str!("../fixtures/state-v22.sql"))
        .unwrap();
    legacy
        .execute(
            "ATTACH DATABASE ?1 AS interrupted",
            [state_path.to_str().unwrap()],
        )
        .unwrap();
    assert_eq!(legacy.execute(
        "INSERT INTO connected_conversations
         SELECT company,connector,conversation,semantic_key,opening,request,session,source_identity,position_json,failure
         FROM interrupted.connected_conversations WHERE conversation=?1",
        [conversation],
    ).unwrap(), 1);
    assert_eq!(legacy.execute(
        "INSERT INTO conversation_messages(company,connector,conversation,message,sender,sender_name,sender_email,
             written_us,written_at,arrived_at,state,text,context_json,boundary,content,attempted_at,turn,failure,
             admitted_at,unavailable_since,turn_update,turn_ended_at)
         SELECT company,connector,conversation,message,sender,sender_name,sender_email,
             written_us,written_at,arrived_at,state,text,context_json,boundary,content,attempted_at,turn,failure,
             admitted_at,unavailable_since,turn_update,turn_ended_at
         FROM interrupted.conversation_messages WHERE conversation=?1",
        [conversation],
    ).unwrap(), 1);
    assert_eq!(legacy.execute(
        "INSERT INTO conversation_answers(company,connector,conversation,turn,piece,text,posted_at,failure,attempted_at)
         SELECT company,connector,conversation,turn,piece,text,posted_at,failure,attempted_at
         FROM interrupted.conversation_answers WHERE conversation=?1 AND attempted_at IS NOT NULL AND posted_at IS NULL",
        [conversation],
    ).unwrap(), 1);
    drop(legacy);
    for suffix in ["", "-wal", "-shm"] {
        let original = state_dir.join(format!("instrukt.db{suffix}"));
        if original.exists() {
            std::fs::rename(
                original,
                c.env.path(&format!("interrupted-v23.sqlite{suffix}")),
            )
            .unwrap();
        }
    }
    std::fs::rename(seed_path, &state_path).unwrap();
    let state = Connection::open(state_path).unwrap();
    assert_eq!(
        state
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap(),
        22
    );
    assert!(!state.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('conversation_answers') WHERE name='post_id')",
        [],
        |row| row.get::<_, bool>(0),
    ).unwrap());
    state
}

fn legacy_answer_attempt_releases_conversation(receipt: LegacyReceipt) {
    let out_of_order = receipt == LegacyReceipt::LateWithUnconfirmedNotice;
    let notice_before_receipt = out_of_order || receipt == LegacyReceipt::LateWithNewNotice;
    let late = notice_before_receipt || receipt == LegacyReceipt::Late;
    let at = jiff::Timestamp::from_second(1_700_000_010).unwrap();
    let (mut c, clock) = status_company(at);
    c.runtime.flag("separate-completion", true);
    let channel = "C1";
    let member = "USTEVEN";
    projected(&c);
    let (asked, thread) = if out_of_order {
        let thread = c
            .slack
            .say_at("1700000000.000100", channel, member, "Thread context", None);
        let asked = c.slack.mention_at(
            "1700000005.000100",
            channel,
            member,
            "<@UAGENT> Where are the notes?",
            Some(&thread),
        );
        (asked, thread)
    } else {
        let asked = c
            .slack
            .mention(channel, member, "<@UAGENT> Where are the notes?", None);
        (asked.clone(), asked)
    };
    let conversation = format!("{channel}/{thread}");
    let session = started(&c, &[]);
    let legacy = "The legacy answer.";
    c.slack.hold_posts_to_fail();
    c.finish_turn(&session, 0, legacy);
    wait("the original answer attempt is retained", || {
        c.slack.held_failing_posts() == 1
    });
    c.runner_dies();
    c.slack.fail_held_posts();
    let state = seed_v22_answer_attempt(&c, &conversation);
    if receipt != LegacyReceipt::Missing {
        c.slack.say_at(
            "1700000011.000100",
            channel,
            crate::fixtures::slack::BOT_USER,
            legacy,
            Some(&thread),
        );
    } else {
        // A same-text post by the bot before the attempt and another person's
        // message after it cannot establish that this piece was delivered.
        c.slack.say_at(
            "1700000009.000100",
            channel,
            crate::fixtures::slack::BOT_USER,
            legacy,
            Some(&thread),
        );
        c.slack
            .say_at("1700000011.000100", channel, member, legacy, Some(&thread));
    }
    let legacy_state = || -> (Option<String>, Option<String>, Option<String>) {
        state.query_row(
            "SELECT posted_at,legacy_read_at,delivery_unknown_at FROM conversation_answers WHERE conversation=?1 AND turn='turn-1' AND piece=0",
            [&conversation],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap()
    };
    if late {
        set_clock(&clock, at + jiff::SignedDuration::from_secs(119));
        c.slack.hold_reads_from("1700000010.000000");
    }
    c.run();
    wait("the new runner migrates the v22 database", || {
        state
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .unwrap()
            == instrukt_core::state_db::schema_version()
    });
    wait("the migrated id-less attempt is read back once", || {
        legacy_state().1.is_some()
    });
    if late {
        wait("the legacy read-back held inside the window", || {
            c.slack.held() > 0
        });
        assert_eq!(
            legacy_state().1,
            Some((at + jiff::SignedDuration::from_secs(119)).to_string())
        );
        if receipt == LegacyReceipt::Late {
            // The upgraded runner already read the legacy attempt. Retain a
            // confirmed notice before the held receipt can settle its epoch.
            state.execute(
                "INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at,state,replied_at,covered_positions) VALUES (?1,'slack',?2,?3,'unavailable',?4,'posted',?4,json_array((SELECT admission_position FROM conversation_messages WHERE company=?1 AND connector='slack' AND conversation=?2 AND message=?5)))",
                instrukt_core::state_db::rusqlite::params![COMPANY, conversation, member, at.to_string(), asked],
            ).unwrap();
            c.slack.say_at(
                "1700000010.500000",
                channel,
                crate::fixtures::slack::BOT_USER,
                &notice_text(member),
                Some(&thread),
            );
        }
        set_clock(&clock, at + jiff::SignedDuration::from_secs(121));
        common::wait_until(
            "expiry while the matching read-back is held",
            std::time::Duration::from_secs(12),
            || legacy_state().2.is_some(),
        );
        assert!(legacy_state().0.is_none());
    }
    let later_noticed = if notice_before_receipt {
        c.runtime.flag("send-failure", true);
        let later = if out_of_order {
            c.slack.fate(&notice_text(member), Fate::Lost, 1);
            c.slack.hold_reads_from("1700000003.000100");
            c.slack.mention_at(
                "1700000003.000100",
                channel,
                member,
                "<@UAGENT> And the contract?",
                Some(&thread),
            )
        } else {
            c.slack.mention(
                channel,
                member,
                "<@UAGENT> And the contract?",
                Some(&thread),
            )
        };
        // Intake runs while the legacy read is held. Give this admitted
        // question the retained interval of a failed hand-over.
        common::wait_until(
            "the later question admitted while the first receipt is held",
            std::time::Duration::from_secs(7),
            || {
                state.execute(
                "UPDATE conversation_messages SET unavailable_since=?2 WHERE message=?1 AND state='pending' AND admission_position IS NOT NULL",
                instrukt_core::state_db::rusqlite::params![later, (at + jiff::SignedDuration::from_secs(121)).to_string()],
            ).unwrap() == 1
            },
        );
        if out_of_order {
            assert!(state.query_row(
                "SELECT newer.admission_position > older.admission_position AND newer.written_us < older.written_us FROM conversation_messages newer, conversation_messages older WHERE newer.message=?1 AND older.message=?2",
                [&later, &asked], |row| row.get::<_, bool>(0),
            ).unwrap());
        }
        set_clock(&clock, at + jiff::SignedDuration::from_secs(181));
        let phase = if out_of_order { "posting" } else { "posted" };
        common::wait_until(
            "Slack accepts the later question's notice before the older receipt",
            std::time::Duration::from_secs(8),
            || {
                notices(&c, member) == 1
                    && notice_state(&c, &conversation, member).as_deref() == Some(phase)
            },
        );
        let id = state.query_row(
            "SELECT post_id FROM conversation_refusals WHERE conversation=?1 AND sender=?2 AND reason='unavailable'",
            [&conversation, member], |row| row.get::<_, String>(0),
        ).unwrap();
        c.slack.release_reads_from("1700000010.000000");
        Some(id)
    } else {
        None
    };
    if receipt == LegacyReceipt::Late {
        // Expiry settles this position and retires its confirmed notice.
        assert!(notice_state(&c, &conversation, member).is_none());
        state.execute_batch(
            "CREATE TRIGGER hold_late_receipt BEFORE UPDATE OF posted_at ON conversation_answers
             WHEN OLD.delivery_unknown_at IS NOT NULL AND NEW.posted_at IS NOT NULL
             BEGIN SELECT RAISE(ABORT, 'forced late receipt failure'); END",
        ).unwrap();
        c.slack.release_reads();
        wait("the late receipt attempts to write", || {
            run_log(&c).contains("forced late receipt failure")
        });
        assert!(legacy_state().2.is_some());
        assert!(notice_state(&c, &conversation, member).is_none());
        state
            .execute_batch("DROP TRIGGER hold_late_receipt")
            .unwrap();
    }
    let database = c.store.file.clone();
    let hidden_database = database.with_extension("away");
    if receipt != LegacyReceipt::Missing {
        wait("the legacy answer receipt recovered", || {
            legacy_state().0.is_some()
        });
        assert!(legacy_state().2.is_none());
        if let Some(id) = &later_noticed {
            use instrukt_core::state_db::rusqlite::OptionalExtension as _;
            let retained = state.query_row(
                "SELECT post_id FROM conversation_refusals WHERE conversation=?1 AND sender=?2 AND reason='unavailable'",
                [&conversation, member], |row| row.get::<_, String>(0),
            ).optional().unwrap();
            assert_eq!(
                retained.as_ref(),
                Some(id),
                "the recovered answer must leave the later notice and its ID in place"
            );
            if out_of_order {
                assert_eq!(
                    notice_state(&c, &conversation, member).as_deref(),
                    Some("posting")
                );
                c.slack.release_reads();
                wait("the later notice reconciles by its original ID", || {
                    notice_state(&c, &conversation, member).as_deref() == Some("posted")
                });
            }
            assert_eq!(notices(&c, member), 1);
        } else {
            assert!(notice_state(&c, &conversation, member).is_none());
        }
    } else {
        assert!(legacy_state().0.is_none());
        assert!(legacy_state().2.is_none());
        c.runner_dies();
        std::fs::rename(&database, &hidden_database).unwrap();
        std::fs::create_dir(&database).unwrap();
        set_clock(&clock, at + jiff::SignedDuration::from_mins(3));
        c.run();
        wait("T3's source unavailable", || {
            retained_conversations(&c).relay_failure.is_some()
        });
        wait("the legacy answer expired while T3 is unreadable", || {
            legacy_state().2.is_some()
        });
        assert!(legacy_state().0.is_none());
        assert_eq!(legacy_state().1, Some(at.to_string()));
        let status = c.conversations();
        assert!(status.conversations.iter().any(|row| {
            row.pending_answers == 0
                && row.failures.iter().any(|cause| {
                    cause.contains("delivery unknown")
                        && cause.contains(&conversation)
                        && cause.contains("turn-1, piece 0")
                })
        }));
    }
    assert_eq!(interval(&c, &asked), None);
    assert_eq!(c.slack.posts().len(), usize::from(later_noticed.is_some()));

    if later_noticed.is_some() {
        c.runtime.flag("send-failure", false);
    } else {
        if receipt == LegacyReceipt::Late {
            c.runtime.flag("send-failure", true);
        }
        let later = c.slack.mention(
            channel,
            member,
            "<@UAGENT> And the contract?",
            Some(&thread),
        );
        if receipt == LegacyReceipt::Missing {
            wait("the later message admitted while T3 is unreadable", || {
                state.query_row(
                    "SELECT EXISTS(SELECT 1 FROM conversation_messages WHERE message=?1 AND admitted_at IS NOT NULL)",
                    [&later],
                    |row| row.get::<_, bool>(0),
                ).unwrap()
            });
            set_clock(&clock, at + jiff::SignedDuration::from_mins(5));
            wait(
                "the later failure produces a notice before T3 recovers",
                || notices(&c, member) == 1,
            );
            assert!(c.conversations().relay_failure.is_some());
            assert_eq!(c.slack.posts().len(), 1);
            std::fs::remove_dir(&database).unwrap();
            std::fs::rename(&hidden_database, &database).unwrap();
        }
        if receipt == LegacyReceipt::Late {
            let since = interval_at(&c, &later);
            set_clock(&clock, since + minute());
            wait(
                "a later notice posts after the recovered receipt closes the old epoch",
                || notices(&c, member) == 1,
            );
            c.runtime.flag("send-failure", false);
        }
    }
    wait("the next message goes to the same conversation", || {
        c.runtime.message_count(&session) == 2
    });
    c.finish_turn(&session, 1, "In the shared drive.");
    wait("the next answer posted and recorded", || {
        c.slack.posts_beginning("In the shared drive.") == 1 && answers_recorded(&c)
    });
    assert!(notice_state(&c, &conversation, member).is_none());
    let expected_notices = usize::from(receipt != LegacyReceipt::Found);
    assert_eq!(c.slack.posts().len(), 1 + expected_notices);
    assert_eq!(notices(&c, member), expected_notices);
}

fn a_successful_answer_without_timestamp_needs_no_read_back() {
    let (mut c, _clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    projected(&c);
    let event = c.slack.direct("D17", "USTEVEN", "Is the release ready?");
    let session = started(&c, &[]);
    c.slack.fate("Ready.", Fate::MissingTimestamp, 1);
    c.slack.refuse_metadata_reads();
    c.finish_turn(&session, 0, "Ready.");
    wait(
        "the successful response confirmed without a timestamp",
        || c.slack.posts_beginning("Ready.") == 1 && answers_recorded(&c),
    );
    assert_eq!(c.slack.metadata_reads(), 0);
    assert_eq!(interval(&c, &event), None);
}

fn failed_metadata_reads_expire_without_sending_again() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    c.runtime.flag("separate-completion", true);
    projected(&c);
    let channel = "D22";
    let member = "USTEVEN";
    let first = c.slack.direct(channel, member, "When do we ship?");
    let session = started(&c, &[]);
    c.slack.fate("On Friday.", Fate::Lost, 1);
    c.slack.refuse_metadata_reads();
    c.finish_turn(&session, 0, "On Friday.");
    wait("Slack refuses the pending answer's receipt read", || {
        c.slack.metadata_reads() > 0
            && retained_conversations(&c).conversations.iter().any(|row| {
                row.pending_answers == 1
                    && row
                        .failures
                        .iter()
                        .any(|cause| cause.contains("missing_scope"))
            })
    });
    set_clock(&clock, interval_at(&c, &first) + minute());
    let delivery_unknown = |status: conversations::Status| {
        status.conversations.iter().any(|row| {
            row.pending_answers == 0
                && row.failures.iter().any(|cause| {
                    cause.contains("delivery unknown")
                        && cause.contains("missing_scope")
                        && cause.contains(channel)
                        && cause.contains("piece 0")
                })
        })
    };
    wait(
        "the answer with an unreadable receipt resolves as unknown",
        || delivery_unknown(retained_conversations(&c)),
    );
    assert!(delivery_unknown(c.conversations()));
    assert_eq!(interval(&c, &first), None);
    assert_eq!(c.slack.posts_beginning("On Friday."), 1);
    assert_eq!(notices(&c, member), 0);

    c.slack.direct(channel, member, "And the contract?");
    wait("the later question is handed over", || {
        c.runtime.message_count(&session) == 2
    });
    c.finish_turn(&session, 1, "In the wiki.");
    wait("the later answer confirmed", || {
        c.slack.posts_beginning("In the wiki.") == 1 && answers_recorded(&c)
    });
    assert_eq!(c.slack.posts().len(), 2);
}

fn a_mixed_notice_epoch_waits_for_the_final_answer_piece() {
    let at: jiff::Timestamp = "2026-11-19T10:00:00Z".parse().unwrap();
    let (mut c, clock) = status_company(at);
    c.runtime.flag("separate-completion", true);
    projected(&c);
    c.runner_dies();
    let database = c.store.file.clone();
    let hidden = database.with_extension("away");
    std::fs::rename(&database, &hidden).unwrap();
    std::fs::create_dir(&database).unwrap();
    c.run();
    wait("T3 unavailable while both questions arrive", || {
        retained_conversations(&c).relay_failure.is_some()
    });
    let channel = "D23";
    let member = "USTEVEN";
    c.slack.direct(channel, member, "Any changes?");
    let second = c.slack.direct(channel, member, "Can I see the report?");
    let state = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    wait("both questions admitted while T3 is down", || {
        state.query_row("SELECT count(*) FROM conversation_messages WHERE conversation=?1 AND admission_position IS NOT NULL", [channel], |row| row.get::<_, i64>(0)).unwrap() == 2
    });
    set_clock(&clock, at + minute());
    wait("one notice covers both unanswered questions", || {
        notices(&c, member) == 1 && state.query_row(
            "SELECT count(*) FROM conversation_refusals WHERE conversation=?1 AND state='posted' AND json_array_length(covered_positions)=2",
            [channel], |row| row.get::<_, i64>(0),
        ).unwrap() == 1
    });
    std::fs::remove_dir(&database).unwrap();
    std::fs::rename(&hidden, &database).unwrap();
    let session = started(&c, &[]);
    c.finish_turn(&session, 0, "NOOP");
    wait("the report question handed over after the NOOP", || {
        c.runtime.message_count(&session) == 2
    });
    c.slack.fate("Final line.", Fate::Refused, usize::MAX);
    c.finish_turn(
        &session,
        1,
        &format!("{}Final line.", "x".repeat(SLACK_MESSAGE_LIMIT)),
    );
    let opened = interval_at(&c, &second);
    wait(
        "the first piece confirmed and the final piece rejected",
        || {
            retained_conversations(&c).conversations.iter().any(|row| {
                row.pending_answers == 1
                    && row
                        .failures
                        .iter()
                        .any(|cause| cause.contains("not_in_channel"))
            }) && c.slack.posts_beginning("xxxxxxxx") == 1
        },
    );
    assert_eq!(
        notice_state(&c, channel, member).as_deref(),
        Some("posted"),
        "a NOOP and a partial answer must keep the mixed epoch"
    );
    observe_notice_pass(&c, &clock, opened + minute());
    assert_eq!(notices(&c, member), 1);
    c.slack.clear_fates();
    wait(
        "the final piece confirmed and the mixed epoch closed",
        || {
            c.slack.posts_beginning("Final line.") == 1
                && notice_state(&c, channel, member).is_none()
        },
    );
    assert_eq!(notices(&c, member), 1);
}

fn an_unconfirmed_post_waits_after_a_metadata_miss() {
    let (mut c, _clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    projected(&c);
    let asked = c.slack.direct("D17", "USTEVEN", "Is the release ready?");
    let session = started(&c, &[]);
    let answer = "Ready.";
    c.slack.fate(answer, Fate::Lost, 1);
    c.slack.miss_metadata_read_before_holding(&asked);
    c.finish_turn(&session, 0, answer);
    wait("the first metadata read misses the posted answer", || {
        c.slack.metadata_reads() > 0
    });
    common::wait_until(
        "the second metadata read after the first one misses",
        std::time::Duration::from_secs(12),
        || {
            assert!(
                c.slack.posts_beginning(answer) <= 1,
                "an empty read inside the post timeout must not repost"
            );
            c.slack.held() > 0
        },
    );
    assert_eq!(c.slack.posts_beginning(answer), 1);
    assert!(!answers_recorded(&c));
    c.slack.release_reads();
    wait("the provider receipt is recorded from read-back", || {
        answers_recorded(&c)
    });
    assert_eq!(c.slack.posts_beginning(answer), 1);
    assert_eq!(notices(&c, "USTEVEN"), 0);
}

fn an_empty_success_lets_the_next_question_receive_its_own_notice(reply: &str) {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "D21";
    let sender = "USTEVEN";
    c.runtime.flag("separate-completion", true);
    c.runtime.flag("list-failure", true);
    projected(&c);
    let first = c.slack.direct(channel, sender, "Any changes?");
    set_clock(&clock, interval_at(&c, &first) + minute());
    wait("the first question's notice posted", || {
        notices(&c, sender) == 1 && notice_state(&c, channel, sender).as_deref() == Some("posted")
    });
    c.runtime.flag("list-failure", false);
    let session = started(&c, &[]);
    c.finish_turn(&session, 0, reply);
    wait(
        "the first notice epoch settled by the successful empty turn",
        || notice_state(&c, channel, sender).is_none(),
    );
    assert_eq!(interval(&c, &first), None);
    assert_eq!(
        c.slack.posts().len(),
        1,
        "only the original notice was posted"
    );
    c.runtime.flag("send-failure", true);
    let second = c.slack.direct(channel, sender, "Who reviews the release?");
    set_clock(&clock, interval_at(&c, &second) + minute());
    wait("the second question's notice posted", || {
        notices(&c, sender) == 2
    });
    assert_eq!(c.slack.posts().len(), 2);
}

fn failed_turns_share_a_notice_until_a_later_turn_settles() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "D21";
    let sender = "USTEVEN";
    c.runtime.flag("separate-completion", true);
    c.runtime.flag("list-failure", true);
    projected(&c);
    let first = c.slack.direct(channel, sender, "Any changes?");
    set_clock(&clock, interval_at(&c, &first) + minute());
    wait("the notice before the failed turn", || {
        notice_state(&c, channel, sender).as_deref() == Some("posted")
    });
    c.runtime.flag("list-failure", false);
    let session = started(&c, &[]);
    wait("the first turn accepted", || {
        c.slack.status(channel, &first).as_deref() == Some(WORKING)
    });
    let input = c.runtime.message(&session, 0);
    c.runtime.set_status(&session, "idle");
    c.runtime.project(&c.env, &session);
    c.store.failed_turn(&session, "failed-turn", 1, &input);

    // Either terminal outcome changes the indication. A failed turn opens
    // its own interval, so the status must stay unavailable through an outage.
    wait("the failed turn's status", || {
        c.slack.status(channel, &first).as_deref() != Some(WORKING)
    });
    let failed_at = interval(&c, &first);
    c.runtime.flag("send-failure", true);
    let second = c.slack.direct(channel, sender, "Who reviews the release?");
    let second_at = interval_at(&c, &second);
    observe_notice_pass(&c, &clock, second_at + minute());
    assert_eq!(
        notices(&c, sender),
        1,
        "the failed turn's notice still covers the member"
    );
    assert_eq!(notice_state(&c, channel, sender).as_deref(), Some("posted"));
    assert!(
        failed_at.is_some(),
        "the failed turn opens its own interval"
    );
    assert_eq!(interval(&c, &first), failed_at);
    assert_eq!(answers(&c), 0);

    c.runtime.flag("send-failure", false);
    c.runtime.flag("list-failure", false);
    wait(
        "the next question reaches the same session after the failed turn",
        || c.runtime.message_count(&session) == 2,
    );
    assert!(
        c.runtime
            .message(&session, 1)
            .contains("Who reviews the release?")
    );
    wait(
        "the failed question stays unavailable while the next turn works",
        || {
            c.slack.status(channel, &first).as_deref() == Some(UNAVAILABLE)
                && c.slack.status(channel, &second).as_deref() == Some(WORKING)
        },
    );
    let input = c.runtime.message(&session, 1);
    c.runtime.set_status(&session, "idle");
    c.runtime.project(&c.env, &session);
    c.store.failed_turn(&session, "failed-turn-2", 2, &input);
    interval_at(&c, &second);
    assert_eq!(notices(&c, sender), 1);

    let third = c.slack.direct(channel, sender, "When can we release?");
    wait("the third question reaches the same session", || {
        c.runtime.message_count(&session) == 3
    });
    let answer = "Ada reviews the release.";
    let first_settled = interval_clock(&clock);
    c.finish_turn(&session, 2, answer);
    wait("the next question is answered", || {
        c.slack.posts_beginning(answer) == 1
    });
    wait(
        "the answer clears the failures and retires their notice",
        || {
            retained_conversations(&c)
                .conversations
                .iter()
                .all(|row| row.conversation != channel)
                && notice_state(&c, channel, sender).is_none()
        },
    );
    assert!(
        c.conversations()
            .conversations
            .iter()
            .all(|row| row.conversation != channel)
    );
    assert_eq!(interval(&c, &first), None);
    assert_eq!(interval(&c, &second), None);
    assert_eq!(interval(&c, &third), None);
    wait("all three Slack statuses clear", || {
        [&first, &second, &third]
            .into_iter()
            .all(|message| c.slack.status(channel, message).is_none())
    });
    assert_eq!(notices(&c, sender), 1);

    set_clock(&clock, first_settled + jiff::SignedDuration::from_hours(1));
    c.runtime.flag("send-failure", true);
    let fourth = c.slack.direct(channel, sender, "Who is on call?");
    set_clock(&clock, interval_at(&c, &fourth) + minute());
    wait("the later outage gets its own notice", || {
        notices(&c, sender) == 2 && notice_state(&c, channel, sender).as_deref() == Some("posted")
    });

    for message in [&first, &second] {
        assert_eq!(
            superseded_at(&c, message),
            Some(Some(format!("{first_settled:.9}")))
        );
    }
}

fn superseded_at(c: &Company, message: &str) -> Option<Option<String>> {
    use instrukt_core::state_db::rusqlite::{Connection, OptionalExtension as _};
    let db = Connection::open(c.env.instrukt_home().join("state/instrukt.db")).unwrap();
    db.query_row(
        "SELECT superseded_at FROM conversation_messages WHERE message=?1",
        [message],
        |row| row.get(0),
    )
    .optional()
    .unwrap()
}

fn a_blank_reply_supersedes_a_failed_question() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "D21";
    let sender = "USTEVEN";
    c.runtime.flag("separate-completion", true);
    projected(&c);
    let failed = c.slack.direct(channel, sender, "Who is on call?");
    let session = started(&c, &[]);
    let input = c.runtime.message(&session, 0);
    c.runtime.set_status(&session, "idle");
    c.runtime.project(&c.env, &session);
    c.store.failed_turn(&session, "failed-turn", 1, &input);
    set_clock(&clock, interval_at(&c, &failed) + minute());
    wait("the failed question's notice posted", || {
        notice_state(&c, channel, sender).as_deref() == Some("posted")
    });
    c.slack.direct(channel, sender, "Did the rotation change?");
    wait("the next question can settle without a reply", || {
        c.runtime.message_count(&session) == 2
    });
    let settled = interval_clock(&clock);
    c.finish_turn(&session, 1, " \n ");
    wait("the blank reply supersedes the failed question", || {
        notice_state(&c, channel, sender).is_none()
    });
    assert_eq!(
        superseded_at(&c, &failed),
        Some(Some(format!("{settled:.9}")))
    );
    assert_eq!(interval(&c, &failed), None);
    assert_eq!(notices(&c, sender), 1);
}

fn pruning_keeps_younger_and_unsuperseded_failures() {
    use instrukt_core::state_db::rusqlite::{Connection, params};
    let now: jiff::Timestamp = "2026-11-19T10:00:00Z".parse().unwrap();
    let (mut c, _) = status_company(now);
    c.runner_dies();
    let db = Connection::open(c.env.instrukt_home().join("state/instrukt.db")).unwrap();
    let cutoff = now - jiff::SignedDuration::from_hours(7 * 24);
    let second = jiff::SignedDuration::from_secs(1);
    for (position, message, state, superseded) in [
        (1, "old", "failed", Some(cutoff - second)),
        (2, "young", "failed", Some(cutoff + second)),
        (3, "unserved", "failed", None),
        (4, "answered", "answered", None),
    ] {
        db.execute(
            "INSERT INTO conversation_messages(company,connector,conversation,message,written_us,written_at,arrived_at,state,text,admission_position,superseded_at)
             VALUES (?1,'slack','D21',?2,?3,'2026-11-08T10:00:00Z','2026-11-08T10:00:00Z',?4,'Retained question',?3,?5)",
            params![c.company().as_str(), message, position, state, superseded.map(|time| format!("{time:.9}"))],
        ).unwrap();
    }
    c.run();
    wait("the relay prunes the expired superseded failure", || {
        superseded_at(&c, "old").is_none()
    });
    assert_eq!(
        superseded_at(&c, "young"),
        Some(Some(format!("{:.9}", cutoff + second)))
    );
    assert_eq!(superseded_at(&c, "unserved"), Some(None));
    assert_eq!(superseded_at(&c, "answered"), Some(None));
}

fn legacy_messages_keep_status_and_the_notice_anchor() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    c.runtime.flag("separate-completion", true);
    let channel = "D14";
    let sender = "USTEVEN";
    projected(&c);
    let first = c.slack.direct(channel, sender, "Is there anything to do?");
    let session = started(&c, &[]);
    let run = c.runtime.active_run(&session);
    c.store.agent_writes(&session, &run, 1, "Checking.");
    wait("the running turn's update", || {
        c.slack.status(channel, &first).as_deref() == Some("Checking.")
    });
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    c.runtime.hide_inputs(&session);
    c.runtime.project(&c.env, &session);
    let opened = interval_at(&c, &first);
    c.runtime.flag("list-failure", true);
    let anonymous = c.slack.direct("D15", "UANNA", "What did I miss?");
    let anonymous_at = interval_at(&c, &anonymous);
    db.execute(
        "UPDATE conversation_messages SET sender=NULL,admitted_at=NULL WHERE message=?1",
        [&anonymous],
    )
    .unwrap();
    db.execute(
        "UPDATE conversation_messages SET admitted_at=NULL WHERE message=?1",
        [&first],
    )
    .unwrap();
    c.slack.fate(&notice_text(sender), Fate::Lost, 1);
    c.slack.hold_reads_from(&first);
    set_clock(&clock, opened.max(anonymous_at) + minute());
    wait(
        "the member's notice despite a legacy row without sender",
        || notices(&c, sender) == 1,
    );
    wait(
        "the legacy row still gets a status without a sender",
        || c.slack.status("D15", &anonymous).as_deref() == Some(UNAVAILABLE),
    );
    assert_eq!(notices(&c, "UANNA"), 0);
    // Keep the first notice's read-back unresolved until its trigger
    // completed with NOOP and a second message is waiting.
    c.runtime.show_inputs();
    c.finish_turn(&session, 0, "NOOP");
    wait("the legacy trigger finished with no output", || {
        shown(&c, channel, &first).last().map(String::as_str) == Some("")
    });
    c.runtime.flag("send-failure", true);
    let later = c.slack.direct(channel, sender, "And tomorrow?");
    interval_at(&c, &later);
    c.slack.release_reads();
    wait(
        "the original notice reconciled and retired after its legacy trigger completes",
        || notice_state(&c, channel, sender).is_none(),
    );
    assert_eq!(notices(&c, sender), 1);
    c.runtime.flag("send-failure", false);
    wait("the follow-up accepted", || {
        c.runtime.message_count(&session) == 2
    });
    c.finish_turn(&session, 1, "Tomorrow is clear.");
    wait("the follow-up answered", || {
        c.slack.posts_beginning("Tomorrow is clear.") == 1
    });
}

fn an_answer_can_post_while_its_notice_is_being_reconciled() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "D11";
    let sender = "USTEVEN";
    projected(&c);
    let asked = c.slack.direct(channel, sender, "Who signed?");
    let session = started(&c, &[]);
    let run = c.runtime.active_run(&session);
    c.store
        .agent_writes(&session, &run, 1, "Checking the record.");
    wait("the relay observing the running turn", || {
        c.slack.status(channel, &asked).as_deref() == Some("Checking the record.")
    });
    c.runtime.hide_inputs(&session);
    c.runtime.project(&c.env, &session);
    let at = interval_at(&c, &asked);
    c.slack.fate(&notice_text(sender), Fate::Refused, 1);
    set_clock(&clock, at + minute());
    wait("the dropped notice attempt", || {
        notice_state(&c, channel, sender).as_deref() == Some("posting")
    });
    let held = c.slack.held();
    c.slack.hold_reads();
    set_clock(&clock, at + jiff::SignedDuration::from_mins(3));
    wait("the held notice reconciliation read", || {
        c.slack.held() > held
    });
    c.runtime.show_inputs();
    c.finish_turn(&session, 0, "Ada signed.");
    common::wait_until(
        "the answer posted during the read",
        std::time::Duration::from_secs(12),
        || c.slack.posts_beginning("Ada signed.") == 1 && answers_recorded(&c),
    );
    assert_eq!(
        notice_state(&c, channel, sender).as_deref(),
        Some("posting")
    );
    c.slack.release_reads();
    wait(
        "the absent notice reconciled before its epoch closes",
        || notice_state(&c, channel, sender).is_none(),
    );
    // The next message can be admitted after the held read is released.
    let later = c.slack.direct(channel, sender, "Thanks.");
    wait("intake takes up the next message", || {
        c.runtime.message_count(&session) == 2
    });
    c.finish_turn(&session, 1, "NOOP");
    wait("the later message complete", || {
        c.slack.status(channel, &later).is_none()
    });
    assert_eq!(notices(&c, sender), 0);
}

/// Publish once per test process. Each scenario gets its own home, Slack,
/// T3 database and runner, with only the immutable settings and role copied.
fn prepared_status_company() -> Company {
    struct FixtureFile {
        path: &'static str,
        content: Vec<u8>,
        permissions: std::fs::Permissions,
    }
    static FILES: std::sync::OnceLock<Vec<FixtureFile>> = std::sync::OnceLock::new();
    let files = FILES.get_or_init(|| {
        let template = Company::on(common::TestEnv::new(), ", history: false", "", "");
        [
            "instrukt.yaml",
            "companies/acme/instrukt.yaml",
            "companies/acme/secrets.env",
            "state/index.yaml",
            "docs/assistance/procedure/company-assistance.md",
        ]
        .into_iter()
        .map(|path| {
            let source = template.env.instrukt_home().join(path);
            FixtureFile {
                path,
                content: std::fs::read(&source).unwrap(),
                permissions: std::fs::metadata(source).unwrap().permissions(),
            }
        })
        .collect()
    });
    let company = Company::with_fakes(status_env());
    for file in files {
        let path = company.env.instrukt_home().join(file.path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &file.content).unwrap();
        std::fs::set_permissions(path, file.permissions.clone()).unwrap();
    }
    company
}

fn status_env() -> common::TestEnv {
    let mut env = common::TestEnv::new();
    let wake = env.path("clock.notice-wake");
    std::fs::write(&wake, "").unwrap();
    env.set_env("INSTRUKT_TEST_NOTICE_WAKE_FILE", wake.to_str().unwrap());
    #[cfg(debug_assertions)]
    env.set_env(
        "INSTRUKT_TEST_NOTICE_DECISIONS",
        env.path("notice-decisions").to_str().unwrap(),
    );
    env
}

/// A company whose conversations run on a runner under the operator's clock
/// in the file it returns, set to `at`.
fn status_company(at: jiff::Timestamp) -> (Company, std::path::PathBuf) {
    let mut c = prepared_status_company();
    let clock = c.env.path("clock");
    set_clock(&clock, at);
    c.env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    c.run();
    (c, clock)
}

fn one_members_answer_proceeds_after_another_conversations_lock_times_out() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let now: jiff::Timestamp = "2026-11-19T10:00:00Z".parse().unwrap();
    let channel = "C1";
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());

    // Another conversation's lock is held past its wait: that conversation
    // is left for later, and a member in this one is still answered.
    use sha2::Digest as _;
    let posts = c.slack.posts().len();
    projected(&c);
    let first = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> What is next?", None);
    let session = started(&c, &[]);
    c.finish_turn(&session, 0, "The release.");
    wait("the other conversation's answer", || {
        c.slack.posts().len() == posts + 1
    });
    let other = conversations::key(&c.company(), SLACK, &format!("{channel}/{first}"));
    let other_lock = instrukt_core::fsx::LockGuard::acquire(
        &home.locks_dir().join(format!(
            "conversation-{}.lock",
            hex::encode(&sha2::Sha256::digest(other.as_bytes())[..8])
        )),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    let before = c.sessions();
    projected(&c);
    let apart = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Who is on call?", None);
    // Each pass waits out the held lock first, a few seconds on a clock
    // running thirty times faster.
    let running = RunningClock::start(clock.clone(), now, 30);
    common::wait_until(
        "the session past the held lock",
        std::time::Duration::from_secs(60),
        || c.sessions().len() > before.len(),
    );
    let session = started(&c, &before);
    c.finish_turn(&session, 0, "Linus is on call.");
    wait("the answer beside the held lock", || {
        c.slack.posts().len() == posts + 2
    });
    assert_eq!(
        c.slack.posts().last().unwrap().thread.as_deref(),
        Some(apart.as_str())
    );
    running.stop();
    drop(other_lock);
}

fn the_notice_minute_starts_when_a_slow_start_fails() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "C1";
    // A start under way a minute and a half, then refused: the message's
    // interval opens when the refusal is established, on the clock then.
    let posts = c.slack.posts().len();
    let asked_at = interval_clock(&clock);
    set_clock(&clock, asked_at);
    let before = c.sessions();
    c.runtime.flag("hold-start", true);
    projected(&c);
    let late = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Who signs it off?", None);
    wait("the start under way", || c.runtime.start_held());
    assert_eq!(c.slack.status(channel, &late).as_deref(), Some(WORKING));
    let refused_at = asked_at + jiff::SignedDuration::from_secs(90);
    set_clock(&clock, refused_at);
    c.runtime.flag("start-failure", true);
    c.runtime.flag("hold-start", false);
    wait("the unavailable text once the start is refused", || {
        c.slack.status(channel, &late).as_deref() == Some(UNAVAILABLE)
    });
    assert_eq!(interval(&c, &late), Some(refused_at.to_string()));
    set_clock(&clock, refused_at + minute());
    wait("one notice after the refused start", || {
        notices(&c, "USTEVEN") == 1
    });
    // Once the start is resolved as never made, it is made and answered.
    c.runtime.flag("start-failure", false);
    let key = conversations::key(&c.company(), SLACK, &format!("{channel}/{late}"));
    c.env
        .run(c.env.home(), ["sessions", "resolve-start", &key, "--none"])
        .success();
    let session = started(&c, &before);
    c.finish_turn(&session, 0, "Margaret signs it off.");
    wait("the late start's answer", || answers(&c) == posts + 1);
}

fn observing_a_running_turn_again_clears_its_and_the_queued_status() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());
    use sha2::Digest as _;
    // In a direct message each message has a status of its own. A message
    // queued behind the running turn and that turn's own message both have
    // their interval opened while the conversation's lock is held past its
    // wait; once the running turn is observed again, both close.
    let posts = c.slack.posts().len();
    let direct = "D7";
    let before = c.sessions();
    projected(&c);
    let own = c.slack.direct(direct, "USTEVEN", "What is on today?");
    let session = started(&c, &before);
    wait("the working text", || {
        c.slack.status(direct, &own).as_deref() == Some(WORKING)
    });
    let queued = c.slack.direct(direct, "USTEVEN", "And tomorrow?");
    wait("the queued message's working text", || {
        c.slack.status(direct, &queued).as_deref() == Some(WORKING)
    });
    // Acquire the lock before the restarted relay can take it.
    c.runner_dies();
    let key = conversations::key(&c.company(), SLACK, direct);
    let held = instrukt_core::fsx::LockGuard::acquire(
        &home.locks_dir().join(format!(
            "conversation-{}.lock",
            hex::encode(&sha2::Sha256::digest(key.as_bytes())[..8])
        )),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    c.run();
    let running = RunningClock::start(clock.clone(), interval_clock(&clock), 30);
    halt_at_intervals(&c, running, &clock, &[&own, &queued]);
    wait("both statuses unavailable", || {
        c.slack.status(direct, &own).as_deref() == Some(UNAVAILABLE)
            && c.slack.status(direct, &queued).as_deref() == Some(UNAVAILABLE)
    });
    drop(held);
    wait(
        "both statuses working once the turn is seen running",
        || {
            c.slack.status(direct, &own).as_deref() == Some(WORKING)
                && c.slack.status(direct, &queued).as_deref() == Some(WORKING)
        },
    );
    assert_eq!((interval(&c, &own), interval(&c, &queued)), (None, None));
    c.finish_turn(&session, 0, "The standup.");
    wait("the queued message handed over", || {
        c.runtime.message_count(&session) == 2
    });
    c.finish_turn(&session, 1, "The retro.");
    wait("both direct answers", || c.slack.posts().len() == posts + 2);
}

fn the_status_tracks_a_slow_answer_and_its_receipt() {
    let done_at = "2026-11-19T10:00:00Z".parse().unwrap();
    let (mut c, clock) = status_company(done_at);
    let channel = "C1";

    // A turn completes while its answer's post is slow and then fails. While
    // the post is under way nothing keeps the answer from the member: the
    // working text shows and no interval is open. Once the post fails, the
    // status says the assistant is unavailable, counted from the turn's end.
    let posts = c.slack.posts().len();
    let before = c.sessions();
    projected(&c);
    let slowly = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Where is the contract?", None);
    let session = started(&c, &before);
    wait("the working text", || {
        c.slack.status(channel, &slowly).as_deref() == Some(WORKING)
    });
    c.slack.hold_posts_to_fail();
    let held_before = c.slack.held_failing_posts();
    let answer = "In the [shared drive](https://example.com/contract?review=1&share=2).";
    c.slack.render_post(
        answer,
        "In the <https://example.com/contract?review=1&amp;share=2|shared drive>.",
    );
    c.finish_turn(&session, 0, answer);
    wait("the answer's post under way", || {
        c.slack.held_failing_posts() > held_before
    });
    assert_eq!(c.slack.status(channel, &slowly).as_deref(), Some(WORKING));
    assert_eq!(interval(&c, &slowly), None);
    set_clock(&clock, done_at + jiff::SignedDuration::from_secs(30));
    // Retries fail too until the failure is seen, so the answer stays kept
    // from the person.
    c.slack.fail_posts();
    c.slack.fail_held_posts();
    wait("the unavailable text once the post failed", || {
        c.slack.status(channel, &slowly).as_deref() == Some(UNAVAILABLE)
    });
    assert_eq!(interval(&c, &slowly), Some(done_at.to_string()));
    // Fault the interval write after Slack accepts the answer. The database
    // must either keep both changes pending or commit both together.
    let state = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    state
        .execute_batch(&format!(
            "CREATE TRIGGER hold_interval_close BEFORE UPDATE OF unavailable_since ON conversation_messages
             WHEN OLD.message='{slowly}' AND OLD.unavailable_since IS NOT NULL AND NEW.unavailable_since IS NULL
             BEGIN SELECT RAISE(ABORT, 'forced interval closure failure'); END"
        ))
        .unwrap();
    c.slack.accept_posts();
    wait("the answer once posts are taken", || {
        c.slack.posts().len() == posts + 1
    });
    let closure_failed = |c: &Company| {
        run_log(c)
            .matches("forced interval closure failure")
            .count()
    };
    wait("the interval closure write attempted", || {
        closure_failed(&c) > 0
    });
    assert!(
        !answers_recorded(&c),
        "the final receipt must stay pending when its interval cannot close"
    );
    assert_eq!(interval(&c, &slowly), Some(done_at.to_string()));
    assert_eq!(
        shown(&c, channel, &slowly).last().map(String::as_str),
        Some(UNAVAILABLE)
    );
    c.runner_dies();
    let previous = closure_failed(&c);
    let status_calls = c.slack.status_calls(channel, &slowly);
    set_clock(&clock, done_at + minute());
    c.run();
    wait(
        "the restarted relay reconciles the normalized answer",
        || closure_failed(&c) > previous && c.slack.status_calls(channel, &slowly) > status_calls,
    );
    assert_eq!(
        c.slack.posts().len(),
        posts + 1,
        "the accepted answer must not be posted again"
    );
    assert!(!answers_recorded(&c));
    state
        .execute_batch("DROP TRIGGER hold_interval_close")
        .unwrap();
    wait("the status cleared", || {
        shown(&c, channel, &slowly).last().map(String::as_str) == Some("")
    });
    assert_eq!(interval(&c, &slowly), None);
    assert_eq!(c.slack.posts().len(), posts + 1);
    assert_eq!(shown(&c, channel, &slowly), [WORKING, UNAVAILABLE, ""]);

    // A slow post that succeeds never shows the unavailable text.
    let posts = c.slack.posts().len();
    let before = c.sessions();
    projected(&c);
    let patient = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Where is the invoice?", None);
    let session = started(&c, &before);
    wait("the working text", || {
        c.slack.status(channel, &patient).as_deref() == Some(WORKING)
    });
    c.slack.hold_posts();
    let held_before = c.slack.held_posts();
    c.finish_turn(&session, 0, "In the finance folder.");
    wait("the answer's post under way", || {
        c.slack.held_posts() > held_before
    });
    set_clock(&clock, done_at + jiff::SignedDuration::from_mins(5));
    c.slack.release_posts();
    wait("the slow answer posted", || {
        c.slack.posts().len() == posts + 1
    });
    wait("the status cleared", || {
        shown(&c, channel, &patient).last().map(String::as_str) == Some("")
    });
    assert_eq!(shown(&c, channel, &patient), [WORKING, ""]);
    assert_eq!(interval(&c, &patient), None);
}

fn an_answer_to_one_member_does_not_restart_anothers_minute() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    c.runtime.flag("separate-completion", true);
    let channel = "C1";
    projected(&c);
    let first = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> When do we ship?", None);
    let session = started(&c, &[]);
    let run = c.runtime.active_run(&session);
    c.store
        .agent_writes(&session, &run, 1, "Checking the date.");
    wait(
        "the relay observing the first member's running turn",
        || c.slack.status(channel, &first).as_deref() == Some("Checking the date."),
    );
    let next = c
        .slack
        .mention(channel, "UANNA", "<@UAGENT> Who reviews it?", Some(&first));
    wait("the second member admitted in the shared thread", || {
        retained_conversations(&c)
            .conversations
            .iter()
            .any(|row| row.pending_messages == 2)
    });
    set_clock(
        &clock,
        interval_clock(&clock) + jiff::SignedDuration::from_secs(90),
    );
    assert_eq!(interval(&c, &next), None);
    assert!(c.slack.posts().is_empty());
    c.runtime.flag("send-failure", true);
    c.slack.fate("On Friday.", Fate::Refused, usize::MAX);
    c.finish_turn(&session, 0, "On Friday.");
    let at = interval_at(&c, &next);
    set_clock(&clock, at + jiff::SignedDuration::from_secs(5));
    c.slack.clear_fates();
    wait("the first member's answer", || {
        c.slack.posts_beginning("On Friday.") == 1
    });
    wait("the first member's interval cleared", || {
        interval(&c, &first).is_none()
    });
    assert_eq!(interval(&c, &next), Some(at.to_string()));
    set_clock(&clock, at + minute());
    wait(
        "the second member told one minute after their own failure",
        || notices(&c, "UANNA") == 1,
    );
    assert_eq!(notices(&c, "USTEVEN"), 0);
    c.runtime.flag("send-failure", false);
    wait("the second member's input accepted", || {
        c.runtime.message_count(&session) == 2
    });
    c.finish_turn(&session, 1, "Ada.");
    wait("the second member's answer", || {
        c.slack.posts_beginning("Ada.") == 1
    });
}

fn a_relay_restart_keeps_the_unavailable_interval() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "C1";

    // The relay cannot acquire the conversation source at all: no read of
    // it is made, so no message's own interval opens, and the relay's
    // failure alone tells the member the assistant is unavailable.
    let outage_at: jiff::Timestamp = "2026-11-19T11:00:00Z".parse().unwrap();
    set_clock(&clock, outage_at);
    let posts = c.slack.posts().len();
    let before = c.sessions();
    projected(&c);
    let unread = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Who has the keys?", None);
    let session = started(&c, &before);
    wait("the working text", || {
        c.slack.status(channel, &unread).as_deref() == Some(WORKING)
    });
    c.runner_dies();
    // T3's database path holds a folder, which the source never watches.
    let database = c.store.file.clone();
    let away = database.with_extension("away");
    std::fs::rename(&database, &away).unwrap();
    std::fs::create_dir(&database).unwrap();
    c.run();
    wait("the unavailable text from the relay's failure", || {
        c.slack.status(channel, &unread).as_deref() == Some(UNAVAILABLE)
    });
    assert_eq!(interval(&c, &unread), None);
    assert_eq!(relay_since(&c), Some(outage_at.to_string()));
    // A later runner meets the same failure under another cause: the
    // failure keeps the start an earlier runner recorded.
    c.runner_dies();
    let db = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    db.execute(
        "UPDATE conversation_relay SET failure='an earlier cause' WHERE id=1",
        [],
    )
    .unwrap();
    set_clock(&clock, outage_at + jiff::SignedDuration::from_secs(59));
    c.run();
    wait("this runner's cause", || {
        retained_conversations(&c)
            .relay_failure
            .is_some_and(|cause| cause != "an earlier cause")
    });
    assert_eq!(relay_since(&c), Some(outage_at.to_string()));
    assert_eq!(notices(&c, "USTEVEN"), 0);
    set_clock(&clock, outage_at + minute());
    wait("the notice at one minute of relay failure", || {
        notices(&c, "USTEVEN") == 1
    });
    // Restored while no runner runs: the next runner reads the source and
    // ends the failure an earlier runner recorded.
    c.runner_dies();
    std::fs::remove_dir(&database).unwrap();
    std::fs::rename(&away, &database).unwrap();
    c.run();
    wait("the working text once the source is acquired", || {
        c.slack.status(channel, &unread).as_deref() == Some(WORKING)
    });
    assert_eq!(c.conversations().relay_failure, None);
    c.finish_turn(&session, 0, "The office manager.");
    wait("the answer after the restart", || answers(&c) == posts + 1);
}

fn a_running_turn_closes_an_observation_failure() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "C1";
    // The session's running turn cannot be observed: the message's own
    // interval opens; once its turn is observed running again, it closes.
    let posts = c.slack.posts().len();
    let before = c.sessions();
    projected(&c);
    let unseen = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Is the VPN down?", None);
    let session = started(&c, &before);
    wait("the working text", || {
        c.slack.status(channel, &unseen).as_deref() == Some(WORKING)
    });
    // The fake runtime removes the inputs from its projection, and puts
    // them back from the thread as it now stands once told to show them: no
    // copy taken here can go stale.
    c.runtime.hide_inputs(&session);
    c.runtime.project(&c.env, &session);
    wait(
        "the unavailable text while the turn cannot be observed",
        || c.slack.status(channel, &unseen).as_deref() == Some(UNAVAILABLE),
    );
    let opened = interval_at(&c, &unseen);
    set_clock(&clock, opened + jiff::SignedDuration::from_secs(10));
    // A projection the fake runtime began while still told to hide them may
    // remove them once more: they are projected again until the turn is
    // seen running.
    c.runtime.show_inputs();
    let mut shown_at: Option<std::time::Instant> = None;
    wait("the working text once the turn is seen running", || {
        if shown_at.is_none_or(|at| at.elapsed() >= std::time::Duration::from_millis(300)) {
            c.runtime.project(&c.env, &session);
            shown_at = Some(std::time::Instant::now());
        }
        c.slack.status(channel, &unseen).as_deref() == Some(WORKING)
    });
    assert_eq!(interval(&c, &unseen), None);
    set_clock(&clock, opened + jiff::SignedDuration::from_secs(90));
    c.finish_turn(&session, 0, "It is back up.");
    wait("the answer once observed", || {
        c.slack.posts().len() == posts + 1
    });
    assert_eq!(notices(&c, "USTEVEN"), 0);
}

fn a_queued_turn_acceptance_closes_the_unavailable_interval() {
    let (mut c, _) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    let channel = "C1";
    // The runtime accepts the message but has not started executing it: the
    // acceptance alone ends the interval its refused listing opened, and the
    // working text shows before execution starts.
    let posts = c.slack.posts().len();
    let before = c.sessions();
    projected(&c);
    c.runtime.flag("list-failure", true);
    let queued = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Can I see the roadmap?", None);
    wait("the unavailable text once the listing is refused", || {
        c.slack.status(channel, &queued).as_deref() == Some(UNAVAILABLE)
    });
    c.runtime.flag("queued-start", true);
    c.runtime.flag("list-failure", false);
    let session = started(&c, &before);
    wait(
        "the working text once the runtime accepts the message",
        || c.slack.status(channel, &queued).as_deref() == Some(WORKING),
    );
    assert_eq!(interval(&c, &queued), None);
    c.runtime.flag("queued-start", false);
    c.finish_turn(&session, 0, "It is in the wiki.");
    wait("the queued message's answer", || {
        c.slack.posts().len() == posts + 1
    });
    assert_eq!(notices(&c, "USTEVEN"), 0);
}

#[cfg(debug_assertions)]
fn a_completed_turn_is_read_again_after_a_locked_read() {
    use sha2::Digest as _;
    let mut c = prepared_status_company();
    c.env.set_env("INSTRUKT_LOG_LEVEL", "debug");
    c.env.set_env("INSTRUKT_NOW", "2026-11-19T10:00:00Z");
    let notifications = c.env.path("t3-wakes");
    let wake = |revision: u64| {
        let next = notifications.with_extension("next");
        std::fs::write(&next, revision.to_string()).unwrap();
        std::fs::rename(next, &notifications).unwrap();
    };
    wake(0);
    c.env
        .set_env("INSTRUKT_TEST_T3_WAKES", notifications.to_str().unwrap());
    c.run();

    projected(&c);
    let initial = c
        .slack
        .mention("C1", "USTEVEN", "<@UAGENT> Where are the notes?", None);
    let settled = c.session();
    c.finish_projected_turn(&settled, 0, "In the wiki.");
    wake(1);
    wait("the earlier answer posted", || c.slack.posts().len() == 1);

    let channel = "C1";
    projected(&c);
    let asked = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> When do we ship?", None);
    let session = started(&c, &[settled]);
    let run = c.runtime.active_run(&session);
    c.store
        .agent_writes(&session, &run, 1, "Checking the release plan.");
    wake(2);
    wait("the relay to observe the running turn's update", || {
        c.slack.status(channel, &asked).as_deref() == Some("Checking the release plan.")
    });

    let log_path = c
        .env
        .home()
        .join(".local/state/instrukt-ai/instrukt/run.log");
    let locked_pass = |revision: u64| {
        let marker = format!("event=t3_fixture_wake revision={revision}");
        let log = std::fs::read_to_string(&log_path).unwrap_or_default();
        let Some((_, pass)) = log.split_once(&marker) else {
            return false;
        };
        pass.split_once("event=conversation_relay_source_processed")
            .is_some_and(|(pass, _)| {
                pass.lines().any(|line| {
                    line.contains("event=conversation_answer_unread")
                        && line.contains("database is locked")
                })
            })
    };
    c.store.exclude_readers();
    c.store
        .agent_writes(&session, &run, 2, "Checking the remaining items.");
    wake(3);
    wait("the relay tried reading between the fixture writes", || {
        locked_pass(3)
    });

    c.finish_projected_turn(&session, 0, "On Friday.");
    wake(4);
    wait("the final fixture wake processed while reads fail", || {
        locked_pass(4)
    });
    assert_eq!(c.slack.posts().len(), 1);
    // Only the fixture supplies notifications, and revision four was
    // consumed with the source still locked. Leave the earlier, settled
    // conversation locked to prove the timer only visits current work.
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());
    let key = conversations::key(&c.company(), SLACK, &format!("{channel}/{initial}"));
    let _settled = instrukt_core::fsx::LockGuard::acquire(
        &home.locks_dir().join(format!(
            "conversation-{}.lock",
            hex::encode(&sha2::Sha256::digest(key.as_bytes())[..8])
        )),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    c.store.let_readers_in();
    common::wait_until(
        "the answer without another projection write",
        std::time::Duration::from_secs(20),
        || c.slack.posts().len() == 2,
    );
    assert_eq!(c.slack.posts()[1].thread.as_deref(), Some(asked.as_str()));
    common::wait_until(
        "operator status to clear its stale answer-read error",
        std::time::Duration::from_secs(20),
        || retained_conversations(&c).conversations.is_empty(),
    );
    assert!(c.conversations().conversations.is_empty());
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV13
#[test]
fn a_member_is_answered_for_a_message_written_while_the_assistant_was_offline() {
    std::thread::scope(|scope| {
        scope.spawn(an_upgraded_connector_reads_before_the_newest_retained_message);
        a_failed_scan_resumes_without_repeating_saved_messages();
    });
}

fn an_upgraded_connector_reads_before_the_newest_retained_message() {
    use instrukt_core::state_db::{
        rusqlite::{Connection, params},
        schema_version,
    };
    let mut c = Company::prepared_for_recovery(common::TestEnv::new());
    let now: jiff::Timestamp = "2026-11-20T12:00:00Z".parse().unwrap();
    let clock = c.env.path("upgrade-clock");
    set_clock(&clock, now);
    c.env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    c.run();
    let state = Connection::open(c.env.instrukt_home().join("state/instrukt.db")).unwrap();
    wait("the current database has opened", || {
        state
            .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
            .is_ok_and(|version| version == schema_version())
    });
    let scan_path = c
        .env
        .instrukt_home()
        .join("state/connector-read-back.sqlite");
    wait("the fixture scan store is ready", || {
        scan_path.exists()
            && Connection::open(&scan_path).is_ok_and(|db| {
                db.query_row("SELECT retained_until FROM scans", [], |row| {
                    row.get::<_, i64>(0)
                })
                .is_ok()
            })
    });
    c.runner_dies();
    let scans = Connection::open(scan_path).unwrap();
    scans.execute("DELETE FROM scans", []).unwrap();

    let slack_time =
        |at: jiff::Timestamp| format!("{}.{:06}", at.as_second(), at.subsec_nanosecond() / 1000);
    let newer = now - jiff::SignedDuration::from_mins(5);
    let retained = slack_time(newer);
    c.slack
        .say_at(&retained, "D30", "USTEVEN", "Already answered.", None);
    let earlier = "Context Slack can still read after the restart.";
    c.slack.say_at(
        &slack_time(now - jiff::SignedDuration::from_hours(25)),
        "D30",
        "USTEVEN",
        earlier,
        None,
    );
    c.slack.say_at(
        &slack_time(now - jiff::SignedDuration::from_hours(23)),
        "D30",
        "USTEVEN",
        "Did we miss this question?",
        None,
    );
    state.execute(
        "INSERT INTO conversation_messages(company,connector,conversation,message,sender,written_us,written_at,arrived_at,state,text)
         VALUES('acme','slack','D30',?1,'USTEVEN',?2,?3,?3,'answered','Already answered.')",
        params![retained, newer.as_microsecond(), newer.to_string()],
    ).unwrap();
    state
        .execute_batch(
            "DROP TABLE connector_read_back_seed; DROP TABLE conversation_missed_identities;",
        )
        .unwrap();
    state
        .pragma_update(None, "user_version", schema_version() - 1)
        .unwrap();

    c.slack.read_back_channels(&["D30"]);
    projected(&c);
    c.slack.refuse_app_connection(true);
    c.start_runner();
    wait(
        "Slack refusals are retained after the missed question",
        || {
            c.slack.refused_app_connections() >= 2 && state.query_row(
            "SELECT state='failed' FROM connector_states WHERE company='acme' AND connector='slack'",
            [], |row| row.get::<_, bool>(0),
        ).unwrap_or(false)
        },
    );
    c.slack.hold_read_back_channel(Some("D30"));
    c.slack.refuse_app_connection(false);
    wait(
        "the first scan stops with a recent question still unread",
        || c.slack.held() > 0,
    );
    let frozen: (i64, i64, Option<i64>) = scans
        .query_row(
            "SELECT started,lower_bound,retained_until FROM scans",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        frozen,
        (
            now.as_microsecond(),
            (now - jiff::SignedDuration::from_hours(24)).as_microsecond(),
            None
        )
    );
    assert!(c.sessions().is_empty());
    c.runner_dies();
    c.slack.hold_read_back_channel(None);
    set_clock(&clock, now + jiff::SignedDuration::from_hours(2));
    c.run();
    let session = started(&c, &[]);
    assert!(
        c.runtime
            .message(&session, 0)
            .contains("Did we miss this question?")
    );
    let input = c.runtime.message(&session, 0);
    assert!(input.contains(earlier), "{input}");
    assert!(
        !input.contains("Earlier messages could not be read."),
        "{input}"
    );
    assert_eq!(state.query_row(
        "SELECT arrived_at FROM conversation_messages WHERE text='Did we miss this question?'",
        [], |row| row.get::<_, String>(0)
    ).unwrap(), (now + jiff::SignedDuration::from_hours(2)).to_string());
    c.finish_turn(&session, 0, "I found it.");
    wait("the earlier missed question is answered", || {
        c.slack.posts_beginning("I found it.") == 1 && answers_recorded(&c)
    });
    assert_eq!(c.runtime.message_count(&session), 1);
    assert_eq!(destinations(&c), [("D30".to_owned(), None)]);
    assert_eq!(
        scans
            .query_row(
                "SELECT first_seen FROM scans WHERE company='acme' AND connector='slack'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        newer.as_microsecond()
    );
}

fn a_failed_scan_resumes_without_repeating_saved_messages() {
    use instrukt_core::state_db::rusqlite::Connection;
    let mut c = Company::prepared_for_recovery(common::TestEnv::new());
    let now: jiff::Timestamp = "2026-11-20T12:00:00Z".parse().unwrap();
    let clock = c.env.path("read-back-clock");
    set_clock(&clock, now - jiff::SignedDuration::from_hours(40));
    c.env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    c.slack.refuse_app_connection(true);
    c.start_runner();
    let store_path = c
        .env
        .instrukt_home()
        .join("state/connector-read-back.sqlite");
    wait("the connector is known before a scan starts", || {
        store_path.exists()
            && Connection::open(&store_path).is_ok_and(|db| {
                db.query_row("SELECT first_seen FROM scans", [], |row| {
                    row.get::<_, i64>(0)
                })
                .is_ok()
            })
    });
    let scans = Connection::open(&store_path).unwrap();
    let cursor = || {
        scans
            .query_row(
                "SELECT retained_until FROM scans WHERE company='acme' AND connector='slack'",
                [],
                |row| row.get::<_, Option<i64>>(0),
            )
            .ok()
            .flatten()
    };
    assert_eq!(cursor(), None);
    c.runner_dies();
    c.slack.refuse_app_connection(false);

    let slack_time =
        |at: jiff::Timestamp| format!("{}.{:06}", at.as_second(), at.subsec_nanosecond() / 1000);
    let old_parent = slack_time(now - jiff::SignedDuration::from_hours(100 * 24));
    let recent = slack_time(now - jiff::SignedDuration::from_mins(5));
    let old = slack_time(now - jiff::SignedDuration::from_hours(30));
    c.slack
        .say_at(&old_parent, "C1", "USTEVEN", "The maintenance thread", None);
    c.slack.say_at(
        &recent,
        "C1",
        "USTEVEN",
        "<@UAGENT> Has maintenance finished?",
        Some(&old_parent),
    );
    c.slack.broadcast("C1", &recent);
    c.slack.say_at(
        &slack_time(now - jiff::SignedDuration::from_mins(4)),
        "C1",
        "USTEVEN",
        "A separate unaddressed message.",
        None,
    );
    c.slack
        .say_at(&old_parent, "C2", "USTEVEN", "An old thread", None);
    c.slack.say_at(
        &old,
        "C2",
        "USTEVEN",
        "<@UAGENT> Is this release out?",
        Some(&old_parent),
    );
    c.slack
        .say_at(&old, "D31", "USTEVEN", "Was it shipped?", None);
    c.slack
        .say_at(&recent, "D30", "USTEVEN", "Where is the report?", None);
    c.slack
        .say_at(&recent, "C3", "USTEVEN", "The notes are ready.", None);
    c.slack.say_at(
        &old_parent,
        "C4",
        "USTEVEN",
        "A thread about to be deleted.",
        None,
    );
    c.slack.say_at(
        &recent,
        "C4",
        "USTEVEN",
        "<@UAGENT> Do not deliver this deleted reply.",
        Some(&old_parent),
    );
    c.slack.delete_thread_on_scan("C4", &old_parent);
    c.slack
        .read_back_channels(&["D30", "C2", "D31", "C4", "C1", "C3"]);
    c.store.thread("s1", None, None, "2026-11-08T10:00:00Z");
    c.store.thread("s2", None, None, "2026-11-08T10:00:00Z");
    c.slack.throttle_later_history_page("C1", 120);
    set_clock(&clock, now);
    c.run();
    let state = Connection::open(c.env.instrukt_home().join("state/instrukt.db")).unwrap();
    let checkpoint = || {
        scans
            .query_row("SELECT position,retry_at FROM scans", [], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .ok()
    };
    wait("the throttled page and retry time are retained", || {
        checkpoint().is_some()
    });
    assert!(c.slack.scan_deleted_thread());
    let saved = checkpoint().unwrap();
    assert_eq!(
        saved.1,
        (now + jiff::SignedDuration::from_secs(120)).as_microsecond()
    );
    assert_eq!(cursor(), None);
    assert_eq!(
        scans
            .query_row(
                "SELECT count(*) FROM seen WHERE missed_link IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    assert_eq!(
        state
            .query_row("SELECT count(*) FROM conversation_messages", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        2
    );
    let attempts = [None, Some("2".to_owned())];
    assert_eq!(c.slack.history_pages("C1"), attempts);
    let waiting = c.status()["conversations"]["read_back_failures"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(waiting.len(), 1);
    assert!(waiting[0]["retry_at"].is_string());
    assert!(c.slack.posts().is_empty());

    c.runner_dies();
    c.run();
    assert_eq!(checkpoint(), Some(saved));
    assert_eq!(c.slack.history_pages("C1"), attempts);
    let home = instrukt_core::home::Home::at_user_home(c.env.home().to_path_buf());
    let writer = instrukt_core::state_db::WriterLock::acquire(&home).unwrap();
    // The remaining pages contain only one duplicate addressed broadcast.
    // They can complete even when live intake is unavailable for writing.
    set_clock(&clock, now + jiff::SignedDuration::from_secs(121));
    wait(
        "the original scan completes under the held intake writer",
        || cursor().is_some(),
    );
    drop(writer);
    assert!(c.slack.history_pages("C1").starts_with(&[
        None,
        Some("2".to_owned()),
        Some("2".to_owned())
    ]));
    assert_eq!(
        state
            .query_row("SELECT count(*) FROM conversation_messages", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        2
    );
    assert_eq!(
        scans
            .query_row(
                "SELECT count(*) FROM seen WHERE missed_link IS NOT NULL",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        2
    );
    wait("the two recovered questions enter their sessions", || {
        c.sessions().len() == 2
    });
    let sessions = c.sessions();
    let direct = sessions
        .iter()
        .find(|id| c.runtime.message(id, 0).contains("Where is the report?"))
        .unwrap();
    let thread = sessions
        .iter()
        .find(|id| {
            c.runtime
                .message(id, 0)
                .contains("Has maintenance finished?")
        })
        .unwrap();
    c.finish_turn(direct, 0, "It is in the wiki.");
    c.finish_turn(thread, 0, "Maintenance finished.");
    wait("both answers are posted", || {
        c.slack.posts().len() == 2 && answers_recorded(&c)
    });
    let posts = c.slack.posts();
    let bodies: std::collections::BTreeSet<_> = posts
        .iter()
        .map(|post| post.markdown.as_deref().or(post.text.as_deref()).unwrap())
        .collect();
    assert_eq!(
        bodies,
        std::collections::BTreeSet::from(["It is in the wiki.", "Maintenance finished."])
    );
    let first_destinations = destinations(&c);
    assert!(first_destinations.contains(&("D30".to_owned(), None)));
    assert!(first_destinations.contains(&("C1".to_owned(), Some(old_parent.clone()))));
    let missed = c.status()["conversations"]["missed_messages"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(missed.len(), 2);
    assert!(
        missed
            .iter()
            .any(|row| row["conversation"] == "D31" && row["message"] == old)
    );
    assert!(missed.iter().any(
        |row| row["conversation"].as_str().unwrap().starts_with("C2/") && row["message"] == old
    ));
    assert!(missed.iter().all(|row| {
        row["link"]
            .as_str()
            .unwrap()
            .starts_with("https://app.slack.com/archives/")
    }));
    c.slack.notify_mention(
        &old,
        "C2",
        "USTEVEN",
        "<@UAGENT> Is this release out?",
        Some(&old_parent),
    );
    wait("Slack acknowledges the delayed live event", || {
        c.slack.acknowledged().contains(&format!("envelope-{old}"))
    });
    assert_eq!(
        state
            .query_row(
                "SELECT count(*) FROM conversation_messages WHERE conversation LIKE 'C2/%'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    let completed = cursor();
    c.runner_dies();
    let follow_up = slack_time(now + jiff::SignedDuration::from_secs(122));
    // Slack supplies an earlier note that arrived after the first delivery.
    // It still belongs in context even though it predates a received question.
    c.slack.say_at(
        &slack_time(now - jiff::SignedDuration::from_mins(6)),
        "C1",
        "USTEVEN",
        "The west stairwell is clear.",
        Some(&old_parent),
    );
    c.slack.say_at(
        &follow_up,
        "C1",
        "USTEVEN",
        "<@UAGENT> Which stairwell is clear?",
        Some(&old_parent),
    );
    set_clock(&clock, now + jiff::SignedDuration::from_secs(123));
    c.run();
    wait(
        "the repeated scan completes and dispatches the offline reply",
        || cursor() > completed && c.runtime.message_count(thread) == 2,
    );
    let input = c.runtime.message(thread, 1);
    assert!(input.contains("The west stairwell is clear."), "{input}");
    assert!(!input.contains("Has maintenance finished?"), "{input}");
    assert!(input.contains("Which stairwell is clear?"), "{input}");
    let command_id: String = state
        .query_row(
            "SELECT dispatch_command_id FROM conversation_messages WHERE message=?1",
            [&follow_up],
            |row| row.get(0),
        )
        .unwrap();
    let dispatched = std::fs::read_to_string(c.runtime.state_dir().join("dispatch-ids")).unwrap();
    assert_eq!(dispatched.lines().collect::<Vec<_>>(), [command_id]);
    assert_eq!(
        state
            .query_row("SELECT count(*) FROM conversation_messages", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        3
    );
    c.finish_turn(thread, 1, "The west stairwell.");
    wait("the offline reply gets its answer", || {
        c.slack.posts_beginning("The west stairwell.") == 1 && answers_recorded(&c)
    });
    assert_eq!(c.slack.posts().len(), 3);
    assert_eq!(
        destinations(&c).last(),
        Some(&("C1".to_owned(), Some(old_parent)))
    );
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV12
#[test]
fn a_member_is_answered_while_a_projects_event_responses_are_not_admitted() {
    // History is off, so the conversation relay is the stand-in's only
    // source reader while the runner runs. The machine has no Redis settings.
    let mut c = Company::on(common::TestEnv::new(), ", history: false", "", "");
    let profile = instrukt_core::ids::ProfileName::parse("antigravity").unwrap();
    patch_company(&c.env, COMPANY, &format!("profiles: [{profile}]"));
    // A published project switches event responses on through its committed
    // layer; this computer has no event transport for them.
    let events = documented::project(
        &c.env,
        "reacting",
        &[(
            "instrukt.yaml",
            format!(
                "schema_version: {}\nproject: reacting\nmodules: {{history: false, events: true}}\n",
                instrukt_core::config::CURRENT_SCHEMA_VERSION
            ),
        )],
    );
    documented::sync(&c.env, &events);
    // The runner starts under a clock file while the dispatch lock is held:
    // its wake waits for the lock on that clock.
    let clock = c.env.path("clock");
    // The runner reads the clock file at any moment: each new time replaces
    // the file whole, so no read finds it half written.
    let set_clock = |at: &str| {
        let next = clock.with_extension("next");
        std::fs::write(&next, at).unwrap();
        std::fs::rename(&next, &clock).unwrap();
    };
    set_clock("2026-11-18T12:00:00Z");
    c.env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    let lock = c.env.instrukt_home().join("state/runtime-dispatch.lock");
    let held = instrukt_core::fsx::LockGuard::try_acquire(&lock)
        .unwrap()
        .unwrap();
    c.run();
    // Past the run-lock wait the wake is skipped, shown in status with its
    // cause, and the runner goes on. The clock moves on at every look, so
    // it passes the wait whenever the runner began it.
    let mut now: jiff::Timestamp = "2026-11-18T12:00:00Z".parse().unwrap();
    wait("the skipped wake in status", || {
        now += jiff::SignedDuration::from_mins(3);
        set_clock(&now.to_string());
        c.status()["skipped_wake"]["cause"]
            .as_str()
            .is_some_and(|cause| !cause.is_empty())
    });
    assert!(c.runner_running());
    // Once the lock is free, the same runner answers a member.
    drop(held);
    let channel = "C1";
    c.slack
        .mention(channel, "USTEVEN", "<@UAGENT> When do we ship?", None);
    let first = c.session();
    c.finish_turn(&first, 0, "On Friday.");
    wait("the first answer", || answers(&c) == 1);
    // The skipped wake stays until a wake takes the lock: the next one comes
    // a minute on, so a runner started again shows it with its first.
    c.runner_dies();
    c.run();
    wait("the skipped wake to clear", || {
        c.status()["skipped_wake"].is_null()
    });
    // Status names the project's event responses as not admitted.
    let findings = |c: &Company| -> Vec<(std::path::PathBuf, String)> {
        c.status()["admission"]["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                (
                    std::path::PathBuf::from(f["document"].as_str().unwrap()),
                    f["key"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    };
    let project_events = (
        events.path().join("instrukt.yaml"),
        "modules.events".to_owned(),
    );
    assert_eq!(findings(&c), std::slice::from_ref(&project_events));
    let reacting: Vec<serde_json::Value> = c.status()["admission"]["projects"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["project"] == "reacting")
        .map(|p| p["events"].clone())
        .collect();
    assert_eq!(reacting, [serde_json::Value::Bool(false)]);
    // The runner's wakes are brought about on the clock; each admits the
    // files again, and status shows what the last one did with them.
    let runner_admission = |c: &Company| c.status()["admission"]["runner"].clone();
    let mut next_wake = |c: &Company, outcome: &str| {
        let before = runner_admission(c);
        wait(&format!("a wake that {outcome} the generation"), || {
            now += jiff::SignedDuration::from_mins(3);
            set_clock(&now.to_string());
            let admission = runner_admission(c);
            admission["at"] != before["at"] && admission["outcome"] == outcome
        });
    };
    // A published project's layer that cannot be read leaves the generation
    // in force rather than one without that project.
    let project_layer = events.path().join("instrukt.yaml");
    let readable = std::fs::read_to_string(&project_layer).unwrap();
    std::fs::write(&project_layer, "schema_version: [\n").unwrap();
    next_wake(&c, "kept");
    std::fs::write(&project_layer, &readable).unwrap();
    next_wake(&c, "replaced");
    // A start captured while T3 cannot list its threads is made again from
    // what was captured once T3 lists again; a company document made
    // unparseable meanwhile does not stand in its way.
    c.runtime.flag("list-failure", true);
    c.slack
        .mention(channel, "USTEVEN", "<@UAGENT> Who reviews it?", None);
    wait("the retained opening's failure in status", || {
        c.status()["conversations"]["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|conversation| {
                conversation["failures"]
                    .as_array()
                    .is_some_and(|failures| !failures.is_empty())
            })
    });
    let company_document = c
        .env
        .instrukt_home()
        .join(format!("companies/{COMPANY}/instrukt.yaml"));
    let parseable = std::fs::read_to_string(&company_document).unwrap();
    std::fs::write(&company_document, "schema_version: [\n").unwrap();
    c.runtime.flag("list-failure", false);
    wait("the retained session", || c.sessions().len() == 2);
    let retained = c
        .sessions()
        .into_iter()
        .find(|session| *session != first)
        .unwrap();
    c.finish_turn(&retained, 0, "Ada reviews it.");
    wait("the retained answer", || answers(&c) == 2);
    std::fs::write(&company_document, &parseable).unwrap();
    next_wake(&c, "replaced");
    // While the runner runs, the company's document is broken by hand and its
    // bot token's secret blanked.
    let company_dir = c.env.instrukt_home().join(format!("companies/{COMPANY}"));
    let document = company_dir.join("instrukt.yaml");
    let mut text = std::fs::read_to_string(&document).unwrap();
    text.push_str("bogus: 1\n");
    std::fs::write(&document, text).unwrap();
    let secrets = company_dir.join("secrets.env");
    let blanked: String = std::fs::read_to_string(&secrets)
        .unwrap()
        .lines()
        .map(|line| {
            if line.starts_with("SLACK_BOT_TOKEN=") {
                "SLACK_BOT_TOKEN=\"\"\n".to_owned()
            } else {
                format!("{line}\n")
            }
        })
        .collect();
    std::fs::write(&secrets, blanked).unwrap();
    wait("the company's findings in status", || {
        findings(&c).len() == 3
    });
    let mut found = findings(&c);
    found.sort();
    let mut expected = vec![
        project_events,
        (document.clone(), "bogus".to_owned()),
        (document.clone(), "connectors.slack.bot_token".to_owned()),
    ];
    expected.sort();
    assert_eq!(found, expected);
    // The runner's next wake keeps the generation in force.
    next_wake(&c, "kept");
    // The member's next question still starts the agent on the company's
    // profile from the last valid generation, and is answered over the last
    // valid connection.
    c.slack
        .mention(channel, "USTEVEN", "<@UAGENT> And the release notes?", None);
    wait("the second session", || c.sessions().len() == 3);
    let second = c
        .sessions()
        .into_iter()
        .find(|session| *session != first && *session != retained)
        .unwrap();
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());
    assert_eq!(
        c.runtime.provider(&second),
        instrukt_core::config::Agents::load(&home.for_inspection())
            .unwrap()
            .profiles[&profile]
            .instance
    );
    c.finish_turn(&second, 0, "They are ready.");
    wait("the second answer", || answers(&c) == 3);
    // The machine layer is broken by hand as well: every command stops on
    // it, status among them, while the runner keeps its generation through
    // its next wake and the member is answered.
    let machine = c.env.instrukt_home().join("instrukt.yaml");
    let admitted = std::fs::read_to_string(&machine).unwrap();
    std::fs::write(&machine, format!("{admitted}modules: [\n")).unwrap();
    now += jiff::SignedDuration::from_mins(3);
    set_clock(&now.to_string());
    c.env
        .run(c.env.home(), ["events", "status", "--json"])
        .failure()
        .code(1);
    c.slack
        .mention(channel, "USTEVEN", "<@UAGENT> And the date?", None);
    wait("the third session", || c.sessions().len() == 4);
    let third = c
        .sessions()
        .into_iter()
        .find(|session| ![&first, &retained, &second].contains(&session))
        .unwrap();
    c.finish_turn(&third, 0, "Friday the 20th.");
    wait("the third answer", || answers(&c) == 4);
}

/// The operator's clock of a test (`INSTRUKT_NOW=@<file>`), which the
/// runner reads at any moment: each new time replaces the file whole.
struct OperatorClock {
    file: std::path::PathBuf,
    now: jiff::Timestamp,
}

impl OperatorClock {
    fn on(c: &mut Company, at: &str) -> Self {
        let clock = Self {
            file: c.env.path("clock"),
            now: at.parse().unwrap(),
        };
        clock.write();
        c.env
            .set_env("INSTRUKT_NOW", &format!("@{}", clock.file.display()));
        clock
    }

    fn advance(&mut self, minutes: i64) {
        self.advance_seconds(minutes * 60);
    }

    fn advance_seconds(&mut self, seconds: i64) {
        self.now += jiff::SignedDuration::from_secs(seconds);
        self.write();
    }

    fn write(&self) {
        let next = self.file.with_extension("next");
        std::fs::write(&next, self.now.to_string()).unwrap();
        std::fs::rename(&next, &self.file).unwrap();
    }
}

/// The provider instance a shipped profile starts on.
fn instance(profile: &str) -> instrukt_core::ids::ProviderInstanceId {
    let agents =
        instrukt_core::config::Agents::parse(instrukt_core::config::SHIPPED_AGENTS).unwrap();
    agents.profiles[&instrukt_core::ids::ProfileName::parse(profile).unwrap()]
        .instance
        .clone()
}

/// T3 reports `profile` as not installed, in every snapshot read from now.
fn uninstalled(c: &Company, profile: &str) {
    let instance = instance(profile);
    common::t3_readiness::update(&c.env, |snapshot| {
        for provider in snapshot["providers"].as_array_mut().unwrap() {
            if provider["instanceId"] == instance.as_str() {
                provider["installed"] = false.into();
            }
        }
    });
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn a_slow_or_failing_t3_cli_does_not_refuse_a_start() {
    let mut c = Company::new("");
    patch_company(&c.env, COMPANY, "profiles: [claude, codex]");
    // Every snapshot T3 gives reports the first profile not installed: a
    // start that knows a snapshot takes the second; one that knows none takes
    // the configured order's first.
    uninstalled(&c, "claude");
    let mut clock = OperatorClock::on(&mut c, "2030-01-01T00:00:00Z");
    // No snapshot is known yet: T3's CLI blocks issuing the runner's token
    // until its launch limit has passed on the clock.
    c.runtime.flag("hold-issue", true);
    c.run();
    wait("the held token issue", || c.runtime.held_issue_running(1));
    clock.advance(1);
    wait("the failed refresh in status", || c.readiness_failing());
    let started = c.asked_and_answered("When do we ship?", "On Friday.");
    assert_eq!(c.runtime.provider(&started), instance("claude"));
    // Still none known: the CLI now exits non-zero.
    c.runtime.flag("hold-issue", false);
    c.runtime.flag("issue-failure", true);
    let launches = c.issue_launches();
    clock.advance(2);
    wait("the next token issue", || c.issue_launches() > launches);
    assert!(c.readiness_failing());
    let started = c.asked_and_answered("Who reviews it?", "Ada does.");
    assert_eq!(c.runtime.provider(&started), instance("claude"));
    // A snapshot is read once the CLI answers again.
    c.runtime.flag("issue-failure", false);
    clock.advance(2);
    wait("a refresh that succeeds", || !c.readiness_failing());
    // With that snapshot retained, the renewal of the token blocks past its
    // limit, and then fails: the starts still choose by the snapshot.
    let renewal = c.runtime.held_issues() + 1;
    c.runtime.flag("hold-issue", true);
    clock.advance(5);
    wait("the held renewal", || c.runtime.held_issue_running(renewal));
    clock.advance(1);
    wait("the failed refresh in status", || c.readiness_failing());
    let started = c.asked_and_answered("And the release notes?", "They are ready.");
    assert_eq!(c.runtime.provider(&started), instance("codex"));
    c.runtime.flag("hold-issue", false);
    c.runtime.flag("issue-failure", true);
    let launches = c.issue_launches();
    clock.advance(5);
    wait("the next token issue", || c.issue_launches() > launches);
    assert!(c.readiness_failing());
    let started = c.asked_and_answered("And the date?", "The 20th.");
    assert_eq!(c.runtime.provider(&started), instance("codex"));
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn a_start_does_not_wait_on_an_outstanding_readiness_acquisition() {
    let mut c = Company::prepared_for_recovery(common::TestEnv::new());
    // The clock only brings the runner's wakes about: no held acquisition
    // is judged on it, and the fake says itself whether each is still open.
    let mut clock = OperatorClock::on(&mut c, "2030-01-01T00:00:00Z");
    let read_flag = |name: &str| c.env.path(&format!("t3-readiness/{name}"));
    let (hold_read, release_read) = (read_flag("hold-read"), read_flag("release-read"));
    let release_issue = c.runtime.state_dir().join("release-issue");
    let watched_read = c.runtime.state_dir().join("watch-read-at-start");
    let read_at_start = c.runtime.state_dir().join("read-at-start");
    let stage_at_history = |c: &Company, text: &str| {
        let before = c.sessions();
        let held = c.slack.held();
        c.slack.hold_reads();
        c.slack.mention("C1", "USTEVEN", text, None);
        wait("the question's context read", || c.slack.held() > held);
        before
    };
    // No snapshot yet, the token's issue held: the first acquisition at the
    // runner's start.
    c.runtime.flag("hold-issue", true);
    c.run();
    wait("the held token issue", || c.runtime.held_issue_running(1));
    let first = c.ask_and_wait_for_session("When do we ship?");
    assert!(c.runtime.held_issue_running(1), "the issue is still open");
    // Finish admitting the question up to Slack's context response before
    // opening a readiness request with a real-time deadline.
    let before = stage_at_history(&c, "<@UAGENT> Who reviews it?");
    // Still no snapshot, the first snapshot read held.
    std::fs::write(&hold_read, "").unwrap();
    c.runtime.flag("hold-issue", false);
    std::fs::write(&release_issue, "").unwrap();
    wait("the held first read", || {
        common::t3_readiness::held_read_open(&c.env, 1)
    });
    std::fs::write(&watched_read, "1").unwrap();
    c.slack.release_reads();
    let second = c.wait_for_new_session(&before);
    assert_eq!(
        std::fs::read_to_string(&read_at_start).unwrap(),
        "arrived",
        "the first read was still outstanding when T3 received start"
    );
    std::fs::remove_file(&hold_read).unwrap();
    std::fs::write(&release_read, "").unwrap();
    wait("the first snapshot", || {
        !common::t3_readiness::reads(&c.env).is_empty()
            && !common::t3_readiness::held_read_open(&c.env, 1)
    });
    // A snapshot retained, a later refresh's read held.
    let later = common::t3_readiness::held_reads(&c.env) + 1;
    std::fs::remove_file(&release_read).unwrap();
    std::fs::write(&hold_read, "").unwrap();
    let before = stage_at_history(&c, "<@UAGENT> And the release notes?");
    clock.advance(2);
    wait("the held later read", || {
        common::t3_readiness::held_read_open(&c.env, later)
    });
    std::fs::write(&watched_read, later.to_string()).unwrap();
    c.slack.release_reads();
    let third = c.wait_for_new_session(&before);
    assert_eq!(
        std::fs::read_to_string(&read_at_start).unwrap(),
        "arrived",
        "read {later} was still outstanding when T3 received start"
    );
    std::fs::remove_file(&hold_read).unwrap();
    std::fs::write(&release_read, "").unwrap();
    std::fs::remove_file(&watched_read).unwrap();
    // A snapshot retained, the renewal's issue held.
    let renewal = c.runtime.held_issues() + 1;
    std::fs::remove_file(&release_issue).unwrap();
    c.runtime.flag("hold-issue", true);
    clock.advance_seconds(270);
    wait("the held renewal", || c.runtime.held_issue_running(renewal));
    let fourth = c.ask_and_wait_for_session("And the date?");
    assert!(
        c.runtime.held_issue_running(renewal),
        "the renewal is still open"
    );
    std::fs::write(&release_issue, "").unwrap();

    // Observe native starts while each acquisition is held. Answer delivery
    // follows after release, outside the acquisition's real-time deadline.
    for (session, reply) in [
        (first, "On Friday."),
        (second, "Ada does."),
        (third, "They are ready."),
        (fourth, "The 20th."),
    ] {
        let posts = c.slack.posts().len();
        c.finish_turn(&session, 0, reply);
        wait("the answer", || c.slack.posts().len() == posts + 1);
    }
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn starts_issue_no_token() {
    let mut c = Company::new("");
    c.run();
    c.asked_and_answered("When do we ship?", "On Friday.");
    c.asked_and_answered("Who reviews it?", "Ada does.");
    assert_eq!(c.runtime.issued(), 1);
}

fn answer_in_member_thread(c: &mut Company) -> String {
    let thread = c
        .slack
        .mention("C1", "USTEVEN", "<@UAGENT> When do we ship?", None);
    let session = c.session();
    c.finish_turn(&session, 0, "On Friday.");
    wait("the member's Slack answer", || !c.slack.posts().is_empty());
    assert_eq!(destinations(c), [("C1".to_owned(), Some(thread))]);
    assert_eq!(c.slack.posts()[0].markdown.as_deref(), Some("On Friday."));
    session
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn a_renewed_token_keeps_the_snapshot_fresh() {
    let mut c = Company::new("");
    patch_company(&c.env, COMPANY, "profiles: [claude, codex]");
    let mut clock = OperatorClock::on(&mut c, "2030-01-01T00:00:00Z");
    c.run();
    let session = |n: u32| format!("00000000-0000-4000-8000-{n:012}");
    wait("the first snapshot", || {
        common::t3_readiness::reads(&c.env).last() == Some(&session(1))
    });
    // Past the renewal point, a minute before the token's five expire, the
    // next refresh reads with a new token, and the earlier one is revoked.
    clock.advance_seconds(270);
    wait("a read with the renewed token", || {
        common::t3_readiness::reads(&c.env).last() == Some(&session(2))
    });
    assert!(common::t3_readiness::revoked(&c.env, &session(1)));
    assert!(!common::t3_readiness::revoked(&c.env, &session(2)));
    // A revocation that fails is logged; the refresh still succeeds.
    c.runtime.flag("revoke-failure", true);
    clock.advance_seconds(270);
    wait("a read with the next token", || {
        common::t3_readiness::reads(&c.env).last() == Some(&session(3))
    });
    wait("the refresh to succeed", || !c.readiness_failing());
    std::fs::write(c.env.path("t3-readiness/hold-read"), "").unwrap();
    clock.advance(1);
    wait("the held refresh after renewal", || {
        common::t3_readiness::held_read_open(&c.env, 1)
    });
    uninstalled(&c, "claude");
    window_full(&c, "antigravity");
    common::t3_readiness::release_read(&c.env, 1);
    // The worker writes quota marks after publishing the snapshot. This
    // profile is outside the conversation's order, so its mark only signals
    // publication and cannot affect which profile the conversation selects.
    wait("the renewed snapshot's quota mark", || {
        !restricted_until(&c, "antigravity").is_empty()
    });
    let started = answer_in_member_thread(&mut c);
    assert_eq!(c.runtime.provider(&started), instance("codex"));
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn a_stopped_runner_revokes_its_token_and_leaves_no_readiness_row() {
    let mut c = Company::new("");
    let mut clock = OperatorClock::on(&mut c, "2030-01-01T00:00:00Z");
    c.run();
    let first = "00000000-0000-4000-8000-000000000001".to_owned();
    wait("the first snapshot", || {
        common::t3_readiness::reads(&c.env).last() == Some(&first)
    });
    // Its token cannot be renewed: the failed refresh shows while the runner
    // runs, and the first token is still the one kept.
    c.runtime.flag("issue-failure", true);
    clock.advance_seconds(270);
    wait("the failed refresh in status", || c.readiness_failing());
    assert!(!common::t3_readiness::revoked(&c.env, &first));
    // Stopped as a service manager stops it, the runner revokes its token
    // before it ends, and its readiness worker is no longer shown.
    c.stop_runner();
    assert!(common::t3_readiness::revoked(&c.env, &first));
    assert!(!c.readiness_failing());
}

/// T3 reports `profile`'s session window full, with no reset given, in
/// every snapshot read from now.
fn window_full(c: &Company, profile: &str) {
    let instance = instance(profile);
    common::t3_readiness::update(&c.env, |snapshot| {
        for provider in snapshot["providers"].as_array_mut().unwrap() {
            if provider["instanceId"] == instance.as_str() {
                provider["usageLimits"] = serde_json::json!({"checkedAt":"2030-01-01T00:00:00Z","windows":[{"id":"session","kind":"session","label":"Session","usedPercent":100}]});
            }
        }
    });
}

/// Until when `instrukt agents availability` restricts `profile`'s models,
/// one entry per model.
fn restricted_until(c: &Company, profile: &str) -> Vec<serde_json::Value> {
    let out = c
        .env
        .run(c.env.home(), ["agents", "availability"])
        .success();
    let marks: serde_json::Value = serde_json::from_slice(&out.get_output().stdout).unwrap();
    marks
        .as_array()
        .unwrap()
        .iter()
        .filter(|mark| mark["profile"] == profile && mark["status"] == "unavailable")
        .map(|mark| mark["until"].clone())
        .collect()
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn a_conversations_quota_restriction_expires_on_the_operator_clock() {
    let mut c = Company::new("");
    patch_company(&c.env, COMPANY, "profiles: [claude, codex]");
    window_full(&c, "claude");
    let mut clock = OperatorClock::on(&mut c, "2030-01-01T00:00:00Z");
    std::fs::write(c.env.path("t3-readiness/hold-read"), "").unwrap();
    c.run();
    wait("the held startup snapshot", || {
        common::t3_readiness::held_read_open(&c.env, 1)
    });
    common::t3_readiness::release_read(&c.env, 1);
    wait("the startup window ends at 01:00", || {
        let until = restricted_until(&c, "claude");
        !until.is_empty() && until.iter().all(|at| at == "2030-01-01T01:00:00Z")
    });
    wait(
        "the first wake's in-flight snapshot on the existing token",
        || common::t3_readiness::held_read_open(&c.env, 2),
    );
    // The queued startup wake already authenticated its read. Fail that
    // response as well as future token issues so neither can renew the mark.
    std::fs::write(c.env.path("t3-readiness/malformed"), "").unwrap();
    c.runtime.flag("issue-failure", true);
    clock.advance(61);
    common::t3_readiness::release_read(&c.env, 2);
    wait("the failed refresh in status", || c.readiness_failing());
    let started = c.asked_and_answered("When do we ship?", "On Friday.");
    assert_eq!(c.runtime.provider(&started), instance("claude"));
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn a_snapshot_is_observed_when_its_read_returns() {
    let mut c = Company::new("");
    patch_company(&c.env, COMPANY, "profiles: [claude, codex]");
    window_full(&c, "claude");
    let mut clock = OperatorClock::on(&mut c, "2030-01-01T00:00:00Z");
    // The first read arrives at t=0 and returns at t=30 min: the window's
    // default expiry is an hour after it returned.
    std::fs::write(c.env.path("t3-readiness/hold-read"), "").unwrap();
    c.run();
    wait("the held first read", || {
        common::t3_readiness::held_read_open(&c.env, 1)
    });
    clock.advance(30);
    common::t3_readiness::release_read(&c.env, 1);
    wait("the first profile's restriction", || {
        !restricted_until(&c, "claude").is_empty()
    });
    let until = restricted_until(&c, "claude");
    assert!(
        until.iter().all(|at| at == "2030-01-01T01:30:00Z"),
        "{until:?}"
    );
    answer_in_member_thread(&mut c);
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn a_stop_during_a_renewal_revokes_the_token_kept_then() {
    let mut c = Company::new("");
    let mut clock = OperatorClock::on(&mut c, "2030-01-01T00:00:00Z");
    c.run();
    let first = "00000000-0000-4000-8000-000000000001";
    let second = "00000000-0000-4000-8000-000000000002";
    wait("the first snapshot", || {
        common::t3_readiness::reads(&c.env)
            .last()
            .map(String::as_str)
            == Some(first)
    });
    // The renewal keeps the second token; the first one's revocation is
    // still running when the runner is stopped.
    c.runtime.flag("hold-next-revoke", true);
    clock.advance_seconds(270);
    wait("the earlier token's revocation", || {
        c.runtime.held_revoke_running(1)
    });
    assert_eq!(c.runtime.issued(), 2);
    c.stop_runner();
    // The stop revokes the token kept then, once, and not the earlier one
    // again.
    assert!(common::t3_readiness::revoked(&c.env, second));
    assert_eq!(c.runtime.revoke_attempts(second), 1);
    assert_eq!(c.runtime.revoke_attempts(first), 1);
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn a_slow_stop_revocation_is_bounded_and_leaves_no_child() {
    let mut c = Company::new("");
    // The operator's clock stands still: the stop's bound is real time.
    let _clock = OperatorClock::on(&mut c, "2030-01-01T00:00:00Z");
    c.run();
    let first = "00000000-0000-4000-8000-000000000001";
    wait("the first snapshot", || {
        !common::t3_readiness::reads(&c.env).is_empty()
    });
    // T3 does not answer the revocation the stop attempts.
    c.runtime.flag("hold-revoke", true);
    c.stop_runner();
    assert_eq!(c.runtime.held_revokes(), 1);
    assert!(
        !c.runtime.held_revoke_running(1),
        "the stop's revocation ended with the runner"
    );
    assert_eq!(c.runtime.revoke_attempts(first), 1);
    assert!(!c.readiness_failing());
    c.runtime.flag("release-revoke", true);
}

/// Whether no conversation holds an answer piece still to be recorded
/// posted: status lists a conversation only while it has work outstanding.
// Poll the same durable status the operator's CLI formats without launching
// a fresh command on each observation. The runner under test is still a
// separate process, and assertions about the operator surface use the CLI.
fn retained_conversations(c: &Company) -> instrukt_company::conversations::Status {
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());
    conversations::status(&home).unwrap()
}

fn answers_recorded(c: &Company) -> bool {
    retained_conversations(c)
        .conversations
        .iter()
        .all(|conversation| conversation.pending_answers == 0)
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV14
#[test]
fn an_answer_whose_acceptance_could_not_be_recorded_is_posted_once() {
    // History is off, so the conversation relay is the stand-in's only
    // source reader while the runner runs.
    let mut c = Company::on(common::TestEnv::new(), ", history: false", "", "");
    // The runner measures its waits on a clock the scenario moves.
    let clock = c.env.path("clock");
    let set_clock = |at: &jiff::Timestamp| {
        let next = clock.with_extension("next");
        std::fs::write(&next, at.to_string()).unwrap();
        std::fs::rename(&next, &clock).unwrap();
    };
    let start: jiff::Timestamp = "2026-11-18T12:00:00Z".parse().unwrap();
    set_clock(&start);
    c.env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    let gate = c.store.gate();
    c.run();
    c.slack
        .mention("C1", "USTEVEN", "<@UAGENT> When do we ship?", None);
    let session = c.session();
    // Slack accepts the post of the answer, and its answer is held.
    c.slack.hold_posts();
    c.finish_turn(&session, 0, "On Friday.");
    wait("the accepted post", || c.slack.held_posts() == 1);
    // While Slack's answer arrives, the state database is held by another
    // process and the runner's clock moves on, so the runner's wait to
    // record the acceptance ends unrecorded.
    let held = instrukt_core::fsx::LockGuard::try_acquire(
        &c.env.instrukt_home().join("state/writer.lock"),
    )
    .unwrap()
    .unwrap();
    gate.hold();
    let ticking = std::sync::atomic::AtomicBool::new(true);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut now = start;
            while ticking.load(std::sync::atomic::Ordering::Acquire) {
                now += jiff::SignedDuration::from_mins(10);
                set_clock(&now);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        });
        c.slack.release_posts();
        // The relay's next pass reads the source only after the pass that
        // posted has ended, the database still held.
        gate.held();
        ticking.store(false, std::sync::atomic::Ordering::Release);
    });
    drop(held);
    gate.open();
    wait("the answer's acceptance recorded", || answers_recorded(&c));
    assert_eq!(
        destinations(&c),
        [("C1".to_owned(), Some("1700000001.000100".to_owned()))]
    );
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV14
#[test]
fn an_answer_accepted_as_the_runner_stopped_is_posted_once() {
    // History is off, so the conversation relay is the stand-in's only
    // source reader while the runner runs.
    let mut c = Company::on(common::TestEnv::new(), ", history: false", "", "");
    c.run();
    c.slack
        .mention("C1", "USTEVEN", "<@UAGENT> When do we ship?", None);
    let session = c.session();
    // Slack accepts the post of the answer, and the runner dies before
    // Slack's answer reaches it.
    c.slack.hold_posts();
    c.finish_turn(&session, 0, "On Friday.");
    wait("the accepted post", || c.slack.held_posts() == 1);
    c.runner_dies();
    c.slack.release_posts();
    // The runner started again finds the answer among its own posts.
    c.run();
    wait("the answer's acceptance recorded", || answers_recorded(&c));
    assert_eq!(
        destinations(&c),
        [("C1".to_owned(), Some("1700000001.000100".to_owned()))]
    );
}

// spec: project/instrukt/spec/feature/company/conversations UC-CV1
#[test]
fn a_question_the_connector_takes_before_the_relay_reads_is_answered() {
    // History is off, so only the relay and readiness read T3's source
    // while the runner runs.
    let mut c = Company::on(common::TestEnv::new(), ", history: false", "", "");
    let gate = c.store.gate();
    // Every source read is held while the runner starts, readiness's and the
    // relay's alike: its connections start before the relay, which reads
    // nothing yet. The connector takes the member's question meanwhile.
    gate.hold();
    c.run();
    let question = c
        .slack
        .mention("C1", "USTEVEN", "<@UAGENT> When do we ship?", None);
    wait("the question retained", || {
        c.conversations()
            .conversations
            .iter()
            .any(|conversation| conversation.pending_messages > 0)
    });
    assert!(c.sessions().is_empty());
    // Once every held read passes, whichever reached the gate first, the
    // question reaches the agent and its answer is posted where it was asked.
    gate.open();
    let session = c.session();
    c.finish_turn(&session, 0, "On Friday.");
    wait("the answer", || c.slack.posts().len() == 1);
    assert_eq!(destinations(&c), [("C1".to_owned(), Some(question))]);
}

fn an_answer_whose_post_was_tried_before_a_restart_shows_the_unavailable_text() {
    let done_at: jiff::Timestamp = "2026-11-19T10:00:00Z".parse().unwrap();
    let (mut c, clock) = status_company(done_at);
    let channel = "C1";

    // The runner dies while a post of the answer is under way and not
    // accepted. The runner started next finds the answer tried and not
    // posted: the member sees the unavailable text, counted from the turn's
    // end, while its retry is under way, and nothing once it is posted.
    projected(&c);
    let lost = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Where is the invoice?", None);
    let session = started(&c, &[]);
    c.slack.hold_posts_to_fail();
    c.finish_turn(&session, 0, "In the finance folder.");
    wait("the answer's post under way", || {
        c.slack.held_failing_posts() == 1
    });
    c.runner_dies();
    c.slack.fail_held_posts();
    c.slack.hold_posts();
    c.slack.hold_reads_from(&lost);
    set_clock(&clock, done_at + jiff::SignedDuration::from_secs(30));
    let status_calls = c.slack.status_calls(channel, &lost);
    c.run();
    // Hold the provider read until the restarted runner sets status.
    // Slack can clear the display when it accepts the retry, even while
    // the response is still held and the receipt remains pending.
    wait(
        "the restarted runner sets unavailable before the retry",
        || {
            c.slack
                .status_texts(channel, &lost)
                .iter()
                .skip(status_calls)
                .any(|status| status == UNAVAILABLE)
        },
    );
    c.slack.release_reads();
    wait("the retry under way", || c.slack.held_posts() == 1);
    assert_eq!(interval(&c, &lost), Some(done_at.to_string()));
    // Shown while that first retry is still under way: no post of this
    // runner has failed, its own timeout included.
    assert_eq!(c.slack.held_posts(), 1);
    let status = c.conversations();
    assert!(
        status.conversations.iter().all(|x| x.failures.is_empty()),
        "{:?}",
        status.conversations
    );
    c.slack.release_posts();
    wait("the status cleared", || {
        shown(&c, channel, &lost).last().map(String::as_str) == Some("")
    });
    assert_eq!(interval(&c, &lost), None);
    assert_eq!(shown(&c, channel, &lost), [WORKING, UNAVAILABLE, ""]);

    // The runner dies during a post of the answer, and the turn's content
    // cannot be read once it is back. Slack refuses delivery until the
    // test releases it. The retained answer can post independently.
    let posts = c.slack.posts().len();
    let ended_at = done_at + jiff::SignedDuration::from_mins(10);
    set_clock(&clock, ended_at);
    let before = c.sessions();
    projected(&c);
    let unread = c
        .slack
        .mention(channel, "USTEVEN", "<@UAGENT> Who signed it?", None);
    let session = started(&c, &before);
    let held = c.slack.held_failing_posts();
    c.slack.hold_posts_to_fail();
    c.finish_turn(&session, 0, "Ada signed it.");
    wait("the answer's post under way", || {
        c.slack.held_failing_posts() > held
    });
    c.runner_dies();
    c.slack.fail_held_posts();
    c.store.hide_messages("turn-2-user");
    c.slack.fate("Ada signed it.", Fate::Refused, usize::MAX);
    set_clock(&clock, ended_at + jiff::SignedDuration::from_secs(30));
    c.run();
    wait(
        "the source remains unreadable and Slack still refuses the saved reply",
        || {
            retained_conversations(&c).conversations.iter().any(|x| {
                x.pending_answers == 1
                    && x.failures
                        .iter()
                        .any(|cause| cause.contains("completed-turn evidence unavailable"))
                    && x.failures
                        .iter()
                        .any(|cause| cause.contains("not_in_channel"))
            }) && c.slack.status(channel, &unread).as_deref() == Some(UNAVAILABLE)
        },
    );
    assert_eq!(interval(&c, &unread), Some(ended_at.to_string()));
    assert_eq!(c.slack.posts().len(), posts);
    c.slack.clear_fates();
    wait(
        "Slack accepts the retained answer while the content is still unreadable",
        || c.slack.posts().len() == posts + 1 && answers_recorded(&c),
    );
    wait("the status cleared", || {
        shown(&c, channel, &unread).last().map(String::as_str) == Some("")
    });
    assert_eq!(c.slack.posts_beginning("Ada signed it."), 1);
    assert_eq!(
        destinations(&c).last(),
        Some(&(channel.to_owned(), Some(unread.clone())))
    );
    assert_eq!(interval(&c, &unread), None);
    assert_eq!(shown(&c, channel, &unread), [WORKING, UNAVAILABLE, ""]);
}
