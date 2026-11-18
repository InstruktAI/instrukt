//! Conversations through the real runner: a company member addresses the
//! company's agent through the Slack stand-in, the agent's session runs in the
//! fake T3 host, and its answer is posted back.
#![allow(clippy::unwrap_used, reason = "isolated feature scenarios")]

use crate::common;
use crate::fixtures::conversing::{COMPANY, Company, patch_company, wait};
use crate::fixtures::documented;
use instrukt_runner::connectors::{SLACK, TURN_AWAY, WORKING};
use instrukt_runner::conversations::{self, SLACK_MESSAGE_LIMIT};
use instrukt_runner::transport::EVENT_STREAM;

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
    // remains unresolved, its key and the command that resolves it. Its
    // asker is admitted, so Slack shows the agent working on the thread.
    c.run();
    wait("the working status on the thread", || {
        c.slack.status(channel, &question).as_deref() == Some(WORKING)
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
    gate.hold();
    c.run();
    gate.admit();
    elsewhere.nudge();
    gate.held();
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
    c.run();
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
    elsewhere.stream();
    gate.hold();
    gate.admit();
    gate.held();
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
    wait("the first answer", || c.slack.posts().len() == 1);
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
    wait("the retained answer", || c.slack.posts().len() == 2);
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
    wait("the second answer", || c.slack.posts().len() == 3);
    // The machine layer is broken by hand as well: status still says why it
    // is not admitted, the runner keeps its generation, and the member is
    // answered.
    let machine = c.env.instrukt_home().join("instrukt.yaml");
    let admitted = std::fs::read_to_string(&machine).unwrap();
    std::fs::write(&machine, format!("{admitted}modules: [\n")).unwrap();
    next_wake(&c, "kept");
    assert!(
        c.status()["admission"]["error"]
            .as_str()
            .is_some_and(|error| !error.is_empty())
    );
    c.slack
        .mention(channel, "USTEVEN", "<@UAGENT> And the date?", None);
    wait("the third session", || c.sessions().len() == 4);
    let third = c
        .sessions()
        .into_iter()
        .find(|session| ![&first, &retained, &second].contains(&session))
        .unwrap();
    c.finish_turn(&third, 0, "Friday the 20th.");
    wait("the third answer", || c.slack.posts().len() == 4);
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

/// Waits until the runner has read the provider snapshot again: one read
/// more than `before`.
fn read_again(c: &Company, before: usize) {
    wait("the next readiness read", || {
        common::t3_readiness::reads(&c.env).len() > before
    });
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
    // What T3 reports after the renewal is what the next start sees.
    common::t3_readiness::update(&c.env, |snapshot| {
        for provider in snapshot["providers"].as_array_mut().unwrap() {
            if provider["instanceId"] == instance("claude").as_str() {
                provider["installed"] = false.into();
            }
        }
    });
    let reads = common::t3_readiness::reads(&c.env).len();
    clock.advance(1);
    read_again(&c, reads);
    let started = c.asked_and_answered("When do we ship?", "On Friday.");
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
    c.run();
    wait("the first profile's restriction", || {
        !restricted_until(&c, "claude").is_empty()
    });
    // Every later refresh fails, so the restriction of t=0 stands: it ends at
    // t=60 min on the runner's clock.
    c.runtime.flag("issue-failure", true);
    clock.advance(61);
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
    std::fs::write(c.env.path("t3-readiness/release-read"), "").unwrap();
    wait("the first profile's restriction", || {
        !restricted_until(&c, "claude").is_empty()
    });
    let until = restricted_until(&c, "claude");
    assert!(
        until.iter().all(|at| at == "2030-01-01T01:30:00Z"),
        "{until:?}"
    );
}
