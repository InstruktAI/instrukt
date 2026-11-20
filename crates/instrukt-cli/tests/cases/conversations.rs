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
    c.env
        .run(c.env.home(), ["sessions", "resolve-start", &key, "--none"])
        .success();
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
    // Each names the person who wrote it, in written order. The third was
    // taken in before the second arrived, so what accompanies it holds the
    // second too.
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
        scope.spawn(the_status_holds_through_locks_held_past_their_wait_and_slow_failures);
        scope.spawn(the_status_follows_the_relays_failure_and_a_turn_that_cannot_be_observed);
        scope.spawn(a_lost_answer_response_defers_notices_until_read_back);
        scope.spawn(a_normalized_notice_is_reconciled_after_a_restart);
        scope.spawn(a_rejected_retry_keeps_reconciling_the_earlier_answer);
        scope.spawn(|| legacy_answer_attempt_releases_conversation(LegacyReceipt::Found));
        scope.spawn(|| legacy_answer_attempt_releases_conversation(LegacyReceipt::Missing));
        scope.spawn(|| legacy_answer_attempt_releases_conversation(LegacyReceipt::Late));
        scope.spawn(legacy_messages_keep_status_and_the_notice_anchor);
        scope.spawn(notice_epochs_and_reconciliation);
        scope.spawn(an_answer_can_post_while_its_notice_is_being_reconciled);
        scope.spawn(an_answer_whose_post_was_tried_before_a_restart_shows_the_unavailable_text);
        notice_and_status_for_one_member();
    });
}

fn notice_and_status_for_one_member() {
    let mut c = Company::on(common::TestEnv::new(), ", history: false", "", "");
    // The operator's clock, moved by the test alone.
    let clock = c.env.path("clock");
    std::fs::write(&clock, "2026-11-19T10:00:00Z").unwrap();
    c.env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    c.run();
    let channel = "C1";

    // The status tells the state. While the message waits for its start the
    // member sees the working text; once the start's listing is refused, the
    // unavailable text; once the runtime accepts the message, the working
    // text again; while its turn runs, the newest message the agent has
    // written in it, as written; and nothing once the answer is posted.
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());
    let dispatch = || {
        instrukt_core::fsx::LockGuard::acquire(
            &home.state_dir().join("runtime-dispatch.lock"),
            std::time::Duration::from_secs(5),
        )
        .unwrap()
    };
    let held = dispatch();
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
    wait("the slow start's answer", || c.slack.posts().len() == 2);
    wait("the status cleared", || {
        shown(&c, channel, &slow).last().map(String::as_str) == Some("")
    });
    assert_eq!(shown(&c, channel, &slow), [WORKING, ""]);

    // A message waits on the runtime's dispatch lock past the wait: the
    // working text, then the unavailable text; once the lock is had, the
    // message is started and answered.
    let held = dispatch();
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
    wait("the locked message's answer", || answers(&c) == 3);
    wait("the status cleared", || {
        shown(&c, channel, &locked).last().map(String::as_str) == Some("")
    });
    assert_eq!(
        shown(&c, channel, &locked),
        [WORKING, UNAVAILABLE, WORKING, ""]
    );

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
    wait("the answer after the outage", || answers(&c) == 4);
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
        || notices(&c, "USTEVEN") == 2,
    );
    assert_eq!(answers(&c), 4);
    c.slack.clear_fates();
    wait("the answer once posts are taken", || answers(&c) == 5);
    wait("the status cleared", || {
        shown(&c, channel, &unposted).last().map(String::as_str) == Some("")
    });
    assert_eq!(shown(&c, channel, &unposted), [WORKING, UNAVAILABLE, ""]);

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
    assert_eq!(answers(&c), 5);
    assert_eq!(notices(&c, "USTEVEN"), 2);

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
        c.conversations()
            .conversations
            .iter()
            .any(|conversation| conversation.pending_messages == 2)
    });
    set_clock(
        &clock,
        interval_clock(&clock) + jiff::SignedDuration::from_secs(90),
    );
    assert_eq!(notices(&c, "USTEVEN"), 2);
    c.finish_turn(&session, 0, "The release.");
    wait("the second message handed over", || {
        c.runtime.message_count(&session) == 2
    });
    c.finish_turn(&session, 1, "The roadmap.");
    wait("both answers", || answers(&c) == 7);
    wait("the status cleared", || {
        shown(&c, channel, &first).last().map(String::as_str) == Some("")
    });
    assert!(!shown(&c, channel, &first).contains(&UNAVAILABLE.to_owned()));
    assert_eq!(notices(&c, "USTEVEN"), 2);
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
    c.runtime.flag("list-failure", true);
    let first = c
        .slack
        .mention(channel, sender, "<@UAGENT> When do we ship?", None);
    let conversation = format!("{channel}/{first}");
    c.slack
        .mention(channel, other, "<@UAGENT> What do I review?", Some(&first));
    let first_at = interval_at(&c, &first);
    wait("both members' messages admitted", || {
        c.conversations()
            .conversations
            .iter()
            .any(|row| row.pending_messages == 2)
    });
    c.runner_dies();
    let database = c.store.file.clone();
    let away = database.with_extension("away");
    std::fs::rename(&database, &away).unwrap();
    std::fs::create_dir(&database).unwrap();
    c.run();
    wait("the relay unavailable for both members", || {
        c.slack.status(channel, &first).as_deref() == Some(UNAVAILABLE)
    });
    let again = c
        .slack
        .mention(channel, sender, "<@UAGENT> And what time?", Some(&first));
    wait(
        "the repeated message admitted while the relay is down",
        || {
            c.conversations()
                .conversations
                .iter()
                .any(|row| row.pending_messages == 3)
        },
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
    c.runtime.flag("list-failure", false);
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
    // A new epoch can owe another notice. A post still in flight has time
    // to become visible without sending a second copy.
    set_clock(
        &clock,
        interval_clock(&clock) + jiff::SignedDuration::from_secs(1),
    );
    c.runtime.flag("send-failure", true);
    let next = c.slack.mention(
        channel,
        sender,
        "<@UAGENT> Where are the notes?",
        Some(&first),
    );
    let next_at = interval_at(&c, &next);
    c.slack.fate(&notice_text(sender), Fate::Late, 1);
    set_clock(&clock, next_at + minute());
    wait("the following epoch's notice sent", || {
        notice_state(&c, &conversation, sender).as_deref() == Some("posting")
    });
    let reads = c.slack.reads();
    wait(
        "a reconciliation read while the notice is in flight",
        || c.slack.reads() > reads,
    );
    assert_eq!(notices(&c, sender), 2);
    c.runtime.flag("send-failure", false);
    wait("the new epoch's input accepted", || {
        c.runtime.message_count(&session) == 4
    });
    c.finish_turn(&session, 3, "NOOP");
    wait("the completed message leaves no pending message", || {
        c.conversations()
            .conversations
            .iter()
            .all(|row| row.pending_messages == 0)
    });
    // NOOP completes one message and leaves the person's epoch and its
    // reconciliation range in place for the next message.
    c.runtime.flag("send-failure", true);
    let after_quiet = c
        .slack
        .mention(channel, sender, "<@UAGENT> And the contract?", Some(&first));
    interval_at(&c, &after_quiet);
    c.slack.commit_late();
    wait("the new epoch's notice visible", || {
        notices(&c, sender) == 3
    });
    set_clock(&clock, next_at + jiff::SignedDuration::from_mins(3));
    wait("the late notice reconciled", || {
        notice_state(&c, &conversation, sender).as_deref() == Some("posted")
    });
    assert_eq!(notices(&c, sender), 3);
    c.runtime.flag("send-failure", false);
    wait("the next input accepted", || {
        c.runtime.message_count(&session) == 5
    });
    c.finish_turn(&session, 4, "In the wiki.");
    wait("the new epoch's answer", || {
        c.slack.posts_beginning("In the wiki.") == 1
    });
    for post in c.slack.posts() {
        assert_eq!(post.channel, "C1");
        assert_eq!(post.thread.as_deref(), Some(first.as_str()));
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

fn a_lost_answer_response_defers_notices_until_read_back() {
    let (mut c, clock) = status_company("2026-11-19T10:00:00Z".parse().unwrap());
    projected(&c);
    let asked = c.slack.direct("D13", "USTEVEN", "Who signed?");
    let session = started(&c, &[]);
    c.slack.fate("Ada signed.", Fate::Lost, 1);
    c.slack.hold_reads_from(&asked);
    c.finish_turn(&session, 0, "Ada signed.");
    wait("Slack displays the answer before reconciliation", || {
        c.slack.posts_beginning("Ada signed.") == 1 && c.slack.held() > 0
    });
    c.runner_dies();
    let opened = interval_at(&c, &asked);
    set_clock(&clock, opened + minute());
    let held = c.slack.held();
    c.run();
    wait("the restarted relay reading back the answer", || {
        c.slack.held() > held
    });
    // Wait through two eligibility passes while the provider read stays held.
    // Slack's request timeout is 30 seconds.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(11);
    while std::time::Instant::now() < until {
        assert_eq!(notices(&c, "USTEVEN"), 0, "the answer is already visible");
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    c.slack.release_reads();
    wait("the answer reconciled", || answers_recorded(&c));
    assert_eq!(c.slack.posts_beginning("Ada signed."), 1);
    assert_eq!(notices(&c, "USTEVEN"), 0);
}

fn a_rejected_retry_keeps_reconciling_the_earlier_answer() {
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
    c.slack.release_reads();
    wait("the retry rejected after an empty read-back", || {
        c.conversations().conversations.iter().any(|row| {
            row.failures
                .iter()
                .any(|cause| cause.contains("not_in_channel"))
        })
    });
    assert!(c.slack.posts().is_empty());
    let opened = interval_at(&c, &asked);
    c.slack.hold_reads_from(&asked);
    c.slack.commit_late();
    assert_eq!(c.slack.posts_beginning(answer), 1);
    set_clock(&clock, opened + minute());
    // Keep read-back blocked through two notice passes. The answer has
    // arrived, but the runner still needs to reconcile its earlier attempt.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(11);
    while std::time::Instant::now() < until {
        assert_eq!(
            notices(&c, "USTEVEN"),
            0,
            "the earlier answer already landed"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    c.slack.release_reads();
    wait(
        "the earlier answer reconciled after a rejected retry",
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
}

fn legacy_answer_attempt_releases_conversation(receipt: LegacyReceipt) {
    let at = jiff::Timestamp::from_second(1_700_000_010).unwrap();
    let (mut c, clock) = status_company(at);
    c.runtime.flag("separate-completion", true);
    let channel = "C1";
    let member = "USTEVEN";
    projected(&c);
    let asked = c
        .slack
        .mention(channel, member, "<@UAGENT> Where are the notes?", None);
    let conversation = format!("{channel}/{asked}");
    let session = started(&c, &[]);
    let legacy = "The legacy answer.";
    c.slack.hold_posts_to_fail();
    c.finish_turn(&session, 0, legacy);
    wait("the original answer attempt is retained", || {
        c.slack.held_failing_posts() == 1
    });
    c.runner_dies();
    c.slack.fail_held_posts();
    let state = instrukt_core::state_db::rusqlite::Connection::open(
        c.env.instrukt_home().join("state/instrukt.db"),
    )
    .unwrap();
    // The migration test covers the schema transition; start the real CLI
    // with the same id-less attempt it leaves for the relay.
    assert_eq!(state.execute(
        "UPDATE conversation_answers SET post_id=NULL,latest_attempted_at=NULL WHERE conversation=?1 AND attempted_at IS NOT NULL",
        [&conversation],
    ).unwrap(), 1);
    if receipt != LegacyReceipt::Missing {
        c.slack.say_at(
            "1700000011.000100",
            channel,
            crate::fixtures::slack::BOT_USER,
            legacy,
            Some(&asked),
        );
    } else {
        // A same-text post by the bot before the attempt and another person's
        // message after it cannot establish that this piece was delivered.
        c.slack.say_at(
            "1700000009.000100",
            channel,
            crate::fixtures::slack::BOT_USER,
            legacy,
            Some(&asked),
        );
        c.slack
            .say_at("1700000011.000100", channel, member, legacy, Some(&asked));
    }
    let legacy_state = || -> (Option<String>, Option<String>, Option<String>) {
        state.query_row(
            "SELECT posted_at,legacy_read_at,delivery_unknown_at FROM conversation_answers WHERE conversation=?1 AND turn='turn-1' AND piece=0",
            [&conversation],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        ).unwrap()
    };
    if receipt == LegacyReceipt::Late {
        // Keep an already delivered notice from this epoch in both systems.
        state.execute(
            "INSERT INTO conversation_refusals(company,connector,conversation,sender,reason,received_at,state,replied_at) VALUES (?1,'slack',?2,?3,'unavailable',?4,'posted',?4)",
            instrukt_core::state_db::rusqlite::params![COMPANY, conversation, member, at.to_string()],
        ).unwrap();
        c.slack.say_at(
            "1700000010.500000",
            channel,
            crate::fixtures::slack::BOT_USER,
            &notice_text(member),
            Some(&asked),
        );
        set_clock(&clock, at + jiff::SignedDuration::from_secs(119));
        c.slack.hold_reads_from("1700000010.000000");
    }
    c.run();
    wait("the id-less attempt read back once", || {
        legacy_state().1.is_some()
    });
    if receipt == LegacyReceipt::Late {
        wait("the legacy read-back held inside the window", || {
            c.slack.held() > 0
        });
        assert_eq!(
            legacy_state().1,
            Some((at + jiff::SignedDuration::from_secs(119)).to_string())
        );
        set_clock(&clock, at + jiff::SignedDuration::from_secs(121));
        common::wait_until(
            "expiry while the matching read-back is held",
            std::time::Duration::from_secs(12),
            || legacy_state().2.is_some(),
        );
        assert!(legacy_state().0.is_none());
        assert_eq!(
            notice_state(&c, &conversation, member).as_deref(),
            Some("posted")
        );
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
        assert_eq!(
            notice_state(&c, &conversation, member).as_deref(),
            Some("posted")
        );
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
        assert!(notice_state(&c, &conversation, member).is_none());
    } else {
        assert!(legacy_state().0.is_none());
        assert!(legacy_state().2.is_none());
        c.runner_dies();
        std::fs::rename(&database, &hidden_database).unwrap();
        std::fs::create_dir(&database).unwrap();
        set_clock(&clock, at + jiff::SignedDuration::from_mins(3));
        c.run();
        wait("T3's source unavailable", || {
            c.conversations().relay_failure.is_some()
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
    assert!(c.slack.posts().is_empty());

    if receipt == LegacyReceipt::Late {
        c.runtime.flag("send-failure", true);
    }
    let later = c
        .slack
        .mention(channel, member, "<@UAGENT> And the contract?", Some(&asked));
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
    wait("the next message goes to the same conversation", || {
        c.runtime.message_count(&session) == 2
    });
    c.finish_turn(&session, 1, "In the shared drive.");
    wait("the next answer posted and recorded", || {
        c.slack.posts_beginning("In the shared drive.") == 1 && answers_recorded(&c)
    });
    let expected_notices = usize::from(receipt != LegacyReceipt::Found);
    assert_eq!(c.slack.posts().len(), 1 + expected_notices);
    assert_eq!(notices(&c, member), expected_notices);
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
    let later_at = interval_at(&c, &later);
    set_clock(&clock, later_at + jiff::SignedDuration::from_mins(3));
    c.slack.release_reads();
    wait(
        "the original notice reconciled after its legacy trigger completes",
        || notice_state(&c, channel, sender).as_deref() == Some("posted"),
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
        || {
            c.slack.posts_beginning("Ada signed.") == 1
                && notice_state(&c, channel, sender).is_none()
        },
    );
    c.slack.release_reads();
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

/// A company whose conversations run on a runner under the operator's clock
/// in the file it returns, set to `at`.
fn status_company(at: jiff::Timestamp) -> (Company, std::path::PathBuf) {
    let mut c = Company::on(common::TestEnv::new(), ", history: false", "", "");
    let clock = c.env.path("clock");
    set_clock(&clock, at);
    c.env
        .set_env("INSTRUKT_NOW", &format!("@{}", clock.display()));
    c.run();
    (c, clock)
}

fn the_status_holds_through_locks_held_past_their_wait_and_slow_failures() {
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
    let now = running.stop();
    let session = started(&c, &before);
    c.finish_turn(&session, 0, "Linus is on call.");
    wait("the answer beside the held lock", || {
        c.slack.posts().len() == posts + 2
    });
    assert_eq!(
        c.slack.posts().last().unwrap().thread.as_deref(),
        Some(apart.as_str())
    );
    drop(other_lock);

    // A start under way a minute and a half, then refused: the message's
    // interval opens when the refusal is established, on the clock then.
    let posts = c.slack.posts().len();
    let asked_at = now + jiff::SignedDuration::from_mins(10);
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

    // A turn completes while its answer's post is slow and then fails. While
    // the post is under way nothing keeps the answer from the member: the
    // working text shows and no interval is open. Once the post fails, the
    // status says the assistant is unavailable, counted from the turn's end.
    let posts = c.slack.posts().len();
    let done_at = refused_at + jiff::SignedDuration::from_mins(10);
    set_clock(&clock, done_at);
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
    let key = conversations::key(&c.company(), SLACK, direct);
    let held = instrukt_core::fsx::LockGuard::acquire(
        &home.locks_dir().join(format!(
            "conversation-{}.lock",
            hex::encode(&sha2::Sha256::digest(key.as_bytes())[..8])
        )),
        std::time::Duration::from_secs(5),
    )
    .unwrap();
    let running = RunningClock::start(
        clock.clone(),
        done_at + jiff::SignedDuration::from_mins(10),
        30,
    );
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
    an_answer_to_one_member_does_not_restart_anothers_minute();
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
        c.conversations()
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

fn the_status_follows_the_relays_failure_and_a_turn_that_cannot_be_observed() {
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
        c.conversations()
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

    assert_eq!(notices(&c, "USTEVEN"), 1);
    #[cfg(debug_assertions)]
    a_completed_turn_is_read_again_after_a_locked_read();
}

#[cfg(debug_assertions)]
fn a_completed_turn_is_read_again_after_a_locked_read() {
    use sha2::Digest as _;
    let mut c = Company::on(common::TestEnv::new(), ", history: false", "", "");
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
        || c.conversations().conversations.is_empty(),
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
    let mut c = Company::new("");
    // The clock only brings the runner's wakes about: no held acquisition
    // is judged on it, and the fake says itself whether each is still open.
    let mut clock = OperatorClock::on(&mut c, "2030-01-01T00:00:00Z");
    let read_flag = |name: &str| c.env.path(&format!("t3-readiness/{name}"));
    let (hold_read, release_read) = (read_flag("hold-read"), read_flag("release-read"));
    let release_issue = c.runtime.state_dir().join("release-issue");
    // No snapshot yet, the token's issue held: the first acquisition at the
    // runner's start.
    c.runtime.flag("hold-issue", true);
    c.run();
    wait("the held token issue", || c.runtime.held_issue_running(1));
    c.asked_and_answered("When do we ship?", "On Friday.");
    assert!(c.runtime.held_issue_running(1), "the issue is still open");
    // Still no snapshot, the first snapshot read held.
    std::fs::write(&hold_read, "").unwrap();
    c.runtime.flag("hold-issue", false);
    std::fs::write(&release_issue, "").unwrap();
    wait("the held first read", || {
        common::t3_readiness::held_read_open(&c.env, 1)
    });
    c.asked_and_answered("Who reviews it?", "Ada does.");
    assert!(
        common::t3_readiness::held_read_open(&c.env, 1),
        "the first read is still open"
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
    clock.advance(2);
    wait("the held later read", || {
        common::t3_readiness::held_read_open(&c.env, later)
    });
    c.asked_and_answered("And the release notes?", "They are ready.");
    assert!(
        common::t3_readiness::held_read_open(&c.env, later),
        "the later read is still open"
    );
    std::fs::remove_file(&hold_read).unwrap();
    std::fs::write(&release_read, "").unwrap();
    // A snapshot retained, the renewal's issue held.
    let renewal = c.runtime.held_issues() + 1;
    std::fs::remove_file(&release_issue).unwrap();
    c.runtime.flag("hold-issue", true);
    clock.advance_seconds(270);
    wait("the held renewal", || c.runtime.held_issue_running(renewal));
    c.asked_and_answered("And the date?", "The 20th.");
    assert!(
        c.runtime.held_issue_running(renewal),
        "the renewal is still open"
    );
    std::fs::write(&release_issue, "").unwrap();
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
fn answers_recorded(c: &Company) -> bool {
    c.conversations()
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
    // cannot be read once it is back: the answer stays unposted, and the
    // member sees the unavailable text from the turn's end until the content
    // reads again and the answer is posted.
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
    set_clock(&clock, ended_at + jiff::SignedDuration::from_secs(30));
    c.run();
    wait("the turn's content unread", || {
        c.conversations()
            .conversations
            .iter()
            .any(|x| !x.failures.is_empty() && x.pending_answers == 1)
    });
    assert_eq!(c.conversations().relay_failure, None);
    wait("the unavailable text while the content is unread", || {
        c.slack.status(channel, &unread).as_deref() == Some(UNAVAILABLE)
    });
    assert_eq!(interval(&c, &unread), Some(ended_at.to_string()));
    c.store.show_messages();
    wait("the answer once the content reads", || {
        c.slack.posts().len() == posts + 1
    });
    wait("the status cleared", || {
        shown(&c, channel, &unread).last().map(String::as_str) == Some("")
    });
    assert_eq!(interval(&c, &unread), None);
    assert_eq!(shown(&c, channel, &unread), [WORKING, UNAVAILABLE, ""]);
}
