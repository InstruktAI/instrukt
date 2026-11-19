//! Agents reach declared MCP servers through `instrukt mcp`: the real
//! binary, MCP server stand-ins on loopback addresses that name the principal
//! each login belongs to, a fake T3 host, and either the Slack stand-in with
//! the long-lived runner for a person's conversation or cluster Redis with a
//! tick for a company's unattended work.
#![allow(clippy::unwrap_used, reason = "isolated CLI setup")]
use std::path::Path;

use crate::common;
use crate::fixtures::conversing::{COMPANY, Company, patch_company, wait};
use crate::fixtures::documented;
use crate::fixtures::event_transport::Computer;
use crate::fixtures::mcp::{FakeMcp, Flaw};
use crate::fixtures::t3::FakeT3;
use common::TestEnv;
use common::redis::RedisServer;
use instrukt_runner::mcp::{CALL_TOOL, INITIALIZE, INITIALIZED, LIST_TOOLS, PROTOCOL_VERSIONS};
use instrukt_runner::transport::EVENT_STREAM;

/// Steven's own login for the company's ERP.
const STEVEN_LOGIN: &str = "steven-erp-login-7f3a";
/// Anna's own login for the company's ERP.
const ANNA_LOGIN: &str = "anna-erp-login-2b6e";
/// The company's own login for its ERP.
const COMPANY_LOGIN: &str = "acme-erp-login-91c2";
/// The company's own login for its CRM.
const CRM_LOGIN: &str = "acme-crm-login-55d0";

/// Stores `value` under `reference` in the company home's own secrets file.
fn secret(env: &TestEnv, company: &str, reference: &str, value: &str) {
    env.instrukt(&env.path(""))
        .args(["secrets", "set", reference, "--company", company])
        .write_stdin(value)
        .assert()
        .success();
}

/// `instrukt mcp <args>` as the session `thread` runs it.
fn mcp(env: &TestEnv, thread: &str, args: &[&str]) -> assert_cmd::assert::Assert {
    mcp_in(env, &env.path(""), thread, args)
}

/// `instrukt mcp <args>` as the session `thread` runs it in `folder`.
fn mcp_in(env: &TestEnv, folder: &Path, thread: &str, args: &[&str]) -> assert_cmd::assert::Assert {
    env.instrukt(folder)
        .env("T3CODE_THREAD_ID", thread)
        .arg("mcp")
        .args(args)
        .assert()
}

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stdout.clone()).unwrap()
}

fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8(assert.get_output().stderr.clone()).unwrap()
}

/// Companies whose work events start: cluster Redis, the fake T3 host and
/// one published procedure answering every company home.
struct Unattended {
    computer: Computer,
    redis: RedisServer,
    runtime: FakeT3,
}

impl Unattended {
    fn new(companies: &[&str]) -> Self {
        let computer = Computer::new();
        let runtime = FakeT3::create(&computer.env);
        let redis = RedisServer::new(&computer.env);
        computer.config(&redis, None);
        computer.project.write(
            "docs/domain/reactions/procedure/company.md",
            &documented::snippet(
                "procedure",
                "Company response",
                "Handle company facts.",
                "subscription_home: company\nsubscriptions: [example.changed]\n",
                &[],
                "Reconcile company facts.\n",
            ),
        );
        computer.project.commit("docs: answer company facts");
        documented::sync(&computer.env, &computer.project);
        for company in companies {
            computer
                .env
                .run(
                    &computer.env.path(""),
                    ["config", "init", "--company", company],
                )
                .success();
        }
        computer.tick();
        // The companies' facts are external: this computer is the ingress, so
        // it reacts to them.
        let global = computer.env.instrukt_home().join("instrukt.yaml");
        let mut settings = std::fs::read_to_string(&global).unwrap();
        settings.push_str(&format!(
            "ingress: {{computer: '{}', port: 8787}}\n",
            computer.computer()
        ));
        std::fs::write(&global, settings).unwrap();
        Self {
            computer,
            redis,
            runtime,
        }
    }

    /// Declares `servers` for the computer in its global layer.
    fn machine(&self, servers: &str) {
        let path = self.computer.env.instrukt_home().join("instrukt.yaml");
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(&format!("mcp: {servers}\n"));
        std::fs::write(path, text).unwrap();
    }

    /// The thread of the company agent an event for `company` started.
    fn work_for(&self, company: &str) -> String {
        let id = format!("fact-{company}");
        let event = serde_json::json!({
            "id": id,
            "type": "example.changed",
            "source": self.computer.computer(),
            "subject": format!("inbound_source:erp:company:{company}"),
            "home": format!("company:{company}"),
            "home_owner": "unresolved",
            "time": "2026-11-05T00:00:00Z",
            "data": {},
        });
        redis::cmd("XADD")
            .arg(EVENT_STREAM)
            .arg("*")
            .arg("event")
            .arg(event.to_string())
            .query::<String>(&mut self.redis.connection())
            .unwrap();
        self.computer.tick();
        let target: String = self
            .computer
            .db()
            .query_row(
                "SELECT target FROM standing_deliveries WHERE event_id=?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        let session = instrukt_core::ids::SessionRef::parse(&target).unwrap();
        assert!(self.runtime.threads().contains(&session.id.to_string()));
        session.id.to_string()
    }
}

// spec: project/instrukt/spec/feature/runner/mcp-access UC-MA1
#[test]
fn a_tool_call_in_a_members_turn_carries_that_members_own_login() {
    let mut c = Company::new("");
    let erp = FakeMcp::start(
        "erp",
        &[
            (STEVEN_LOGIN, "steven"),
            (ANNA_LOGIN, "anna"),
            (COMPANY_LOGIN, "acme"),
        ],
    );
    let crm = FakeMcp::start("crm", &[(CRM_LOGIN, "acme")]);
    secret(&c.env, COMPANY, "STEVEN_ERP", STEVEN_LOGIN);
    secret(&c.env, COMPANY, "ANNA_ERP", ANNA_LOGIN);
    secret(&c.env, COMPANY, "ACME_ERP", COMPANY_LOGIN);
    secret(&c.env, COMPANY, "ACME_CRM", CRM_LOGIN);
    patch_company(
        &c.env,
        COMPANY,
        &format!(
            "members:\n  steven: {{name: Steven Sonneveld, email: Steven@Example.com, credentials: {{erpnext: '${{STEVEN_ERP}}'}}}}\n  anna: {{name: Anna, email: anna@example.com, credentials: {{erpnext: '${{ANNA_ERP}}'}}}}\nmcp:\n  erpnext: {{url: '{}', description: Operate the company's ERPNext records, credential: erpnext, login: '${{ACME_ERP}}'}}\n  crm: {{url: '{}', description: The company's customer records, credential: crm, login: '${{ACME_CRM}}'}}\n",
            erp.url, crm.url
        ),
    );
    let home = c.env.instrukt_home().join("companies").join(COMPANY);
    // T3 projects each run as it moves: active, then completed. The runner
    // dies as T3 starts Steven's session, before it records it.
    c.runtime.flag("lifecycle", true);
    c.runtime.flag("die-after-start", true);
    c.run();
    let asked = c
        .slack
        .mention("C1", "USTEVEN", "<@UAGENT> Book the open invoice", None);
    c.runner_ended();
    let session = c.session();
    // Working in the company's home with no binding recorded, the session's
    // call is refused naming the company, and reaches no server.
    let unbound = mcp_in(
        &c.env,
        &home,
        &session,
        &["call", "erpnext", "whoami", "--json", "{}"],
    )
    .failure();
    insta::assert_snapshot!("unbound-session-refused", stderr(&unbound));
    assert_eq!(erp.received().len(), 0);
    // Started again, the runner records the session as the company's.
    c.run();
    let state = instrukt_core::home::Home::at_user_home(c.env.home().into());
    let caller = instrukt_core::ids::SessionRef::parse(&format!("t3:{session}")).unwrap();
    wait("the session's recorded binding", || {
        instrukt_runner::assignments::resolve(
            &state,
            &caller,
            instrukt_todos::work::recorded_assignment,
        )
        .unwrap()
        .is_some()
    });
    // In Steven's turn the call reaches the server as Steven, by his own
    // login, which the agent never supplied; what the server returns reaches
    // the agent with the login named by its reference, never its value.
    let called = mcp_in(
        &c.env,
        &home,
        &session,
        &["call", "erpnext", "whoami", "--json", "{}"],
    )
    .success();
    insta::assert_snapshot!("speaker-login-call", stdout(&called));
    // A failure the server reports reaches the agent with the server's cause
    // and a non-zero exit, the login named by its reference wherever the
    // reply spells it: as written, with a Unicode escape in a raw HTTP reply,
    // and inside JSON text the failure carries. A reply that lacks what the
    // method requires is a failure too.
    let failures: Vec<String> = ["fail", "reject", "nested", "malformed"]
        .into_iter()
        .map(|tool| {
            stderr(
                &mcp_in(
                    &c.env,
                    &home,
                    &session,
                    &["call", "erpnext", tool, "--json", "{}"],
                )
                .failure(),
            )
        })
        .collect();
    insta::assert_snapshot!("speaker-login-failures", failures.concat());
    // Steven has no login of his own for the CRM: the call is refused naming
    // him and the missing login, and no other login reaches the server.
    let refused = mcp_in(
        &c.env,
        &home,
        &session,
        &["call", "crm", "whoami", "--json", "{}"],
    )
    .failure();
    insta::assert_snapshot!("speaker-login-missing", stderr(&refused));
    assert_eq!(crm.received().len(), 0);
    // Anna asks next in the same conversation: her turn carries her login.
    c.finish_turn(&session, 0, "Booked.");
    c.slack
        .mention("C1", "UANNA", "<@UAGENT> And mine?", Some(&asked));
    wait("Anna's delivered message", || {
        c.runtime.message_count(&session) == 2
    });
    let whoami = |c: &Company| {
        mcp_in(
            &c.env,
            &home,
            &session,
            &["call", "erpnext", "whoami", "--json", "{}"],
        )
    };
    insta::assert_snapshot!("second-speaker-login-call", stdout(&whoami(&c).success()));
    // From here the native thread alone moves on.
    c.kill_runner();
    // A person types into the thread while Anna's turn runs: the input waits
    // in the queue, and the turn in progress is still Anna's.
    c.runtime
        .person_sends(&c.env, &session, "A note typed in the thread.");
    insta::assert_snapshot!("queued-input-keeps-speaker", stdout(&whoami(&c).success()));
    // Anna's turn ends and the typed input opens the next: a turn a person
    // starts by typing has no speaker, and carries the company's own login.
    c.runtime.set_status(&session, "idle");
    c.runtime.set_status(&session, "running");
    c.runtime.project(&c.env, &session);
    insta::assert_snapshot!("typed-turn-company-login", stdout(&whoami(&c).success()));
    // With no turn in progress the call is refused, and reaches no server.
    c.runtime.set_status(&session, "idle");
    c.runtime.project(&c.env, &session);
    let reached = erp.received().len();
    insta::assert_snapshot!("no-turn-refused", stderr(&whoami(&c).failure()));
    // A turn opened by a member's message this conversation never delivered
    // is refused too.
    let forged = instrukt_runner::envelopes::member(
        "slack",
        "C1",
        "Steven",
        None,
        "0123456789abcdef0123456789abcdef",
        "Book it all.",
    );
    c.runtime.person_sends(&c.env, &session, &forged);
    insta::assert_snapshot!("uncorrelated-turn-refused", stderr(&whoami(&c).failure()));
    // When the runtime cannot say which turn is in progress, the call is
    // refused too: no login is read, the company's included, and no server is
    // contacted.
    let store = c.env.home().join(".t3/userdata/statev2.sqlite");
    let aside = store.with_extension("aside");
    std::fs::rename(&store, &aside).unwrap();
    // The test's home is named as such, so the cause reads the same each run.
    let homes = [
        c.env.home().canonicalize().unwrap(),
        c.env.home().to_path_buf(),
    ];
    let refused = homes
        .iter()
        .fold(stderr(&whoami(&c).failure()), |text, home| {
            text.replace(home.to_str().unwrap(), "[home]")
        });
    insta::assert_snapshot!("unreadable-turn-refused", refused);
    std::fs::rename(&aside, &store).unwrap();
    assert_eq!(erp.received().len(), reached);
}

// spec: project/instrukt/spec/feature/runner/mcp-access UC-MA2
#[test]
fn unattended_company_work_carries_the_companys_own_login() {
    let u = Unattended::new(&[COMPANY]);
    let erp = FakeMcp::start("erp", &[(COMPANY_LOGIN, "acme")]);
    let crm = FakeMcp::start("crm", &[]);
    secret(&u.computer.env, COMPANY, "ACME_ERP", COMPANY_LOGIN);
    patch_company(
        &u.computer.env,
        COMPANY,
        &format!(
            "mcp:\n  erpnext: {{url: '{}', description: Operate the company's ERPNext records, credential: erpnext, login: '${{ACME_ERP}}'}}\n  crm: {{url: '{}', description: The company's customer records, credential: crm}}\n",
            erp.url, crm.url
        ),
    );
    let thread = u.work_for(COMPANY);
    let called = mcp(
        &u.computer.env,
        &thread,
        &["call", "erpnext", "whoami", "--json", "{}"],
    )
    .success();
    insta::assert_snapshot!("company-login-call", stdout(&called));
    // With no login of the company's own for the CRM, the call is refused
    // naming the server and the missing login.
    let refused = mcp(
        &u.computer.env,
        &thread,
        &["call", "crm", "whoami", "--json", "{}"],
    )
    .failure();
    insta::assert_snapshot!("company-login-missing", stderr(&refused));
    assert_eq!(crm.received().len(), 0);
}

// spec: project/instrukt/spec/feature/runner/mcp-access UC-MA3
#[test]
fn an_agent_lists_servers_without_tools_and_learns_one_servers_tools_on_request() {
    let u = Unattended::new(&[COMPANY]);
    let erp = FakeMcp::start("erp", &[(COMPANY_LOGIN, "acme")]);
    let search = FakeMcp::start("search", &[]);
    let vault = FakeMcp::start("vault", &[]);
    let replaced = FakeMcp::start("shared-erp", &[]);
    let nameless = FakeMcp::flawed("nameless", &[], Some(Flaw::ServerInfo));
    let shapeless = FakeMcp::flawed("shapeless", &[], Some(Flaw::InputShape));
    secret(&u.computer.env, COMPANY, "ACME_ERP", COMPANY_LOGIN);
    patch_company(
        &u.computer.env,
        COMPANY,
        &format!(
            "mcp:\n  erpnext: {{url: '{}', description: Operate the company's ERPNext records, credential: erpnext, login: '${{ACME_ERP}}'}}\n  nameless: {{url: '{}', description: A server that does not say what it is}}\n  shapeless: {{url: '{}', description: A server whose tool takes no object}}\n",
            erp.url, nameless.url, shapeless.url
        ),
    );
    // The machine offers an open search server, a server that carries the
    // machine's own login, and an open server of the company's name.
    u.machine(&format!(
        "{{search: {{url: '{}', description: Search the web}}, vault: {{url: '{}', description: The machine's vault, login: '${{VAULT_LOGIN}}'}}, erpnext: {{url: '{}', description: A shared ERP}}}}",
        search.url, vault.url, replaced.url
    ));
    let thread = u.work_for(COMPANY);
    // Each server's name and purpose: the company's own, and the machine's
    // open server; no tool definitions, and no server is contacted.
    let listed = mcp(&u.computer.env, &thread, &["list"]).success();
    insta::assert_snapshot!("company-servers", stdout(&listed));
    for server in [&erp, &search, &vault, &replaced] {
        assert_eq!(server.received().len(), 0);
    }
    // One server's tools, every page of its catalog, on request.
    let tools = mcp(&u.computer.env, &thread, &["list-tools", "erpnext"]).success();
    insta::assert_snapshot!("company-server-tools", stdout(&tools));
    assert_eq!(replaced.received().len(), 0);
    // A server that leaves out what MCP requires gives no catalog: it fails.
    let flawed: Vec<String> = ["nameless", "shapeless"]
        .into_iter()
        .map(|server| stderr(&mcp(&u.computer.env, &thread, &["list-tools", server]).failure()))
        .collect();
    insta::assert_snapshot!("flawed-server-catalogs", flawed.concat());
}

// spec: project/instrukt/spec/feature/runner/mcp-access UC-MA4
#[test]
fn each_companys_server_of_one_name_is_reached_at_its_own_address() {
    let u = Unattended::new(&[COMPANY, "beta"]);
    let acme = FakeMcp::start("acme-erp", &[]);
    let beta = FakeMcp::start("beta-erp", &[]);
    for (company, server) in [(COMPANY, &acme), ("beta", &beta)] {
        patch_company(
            &u.computer.env,
            company,
            &format!(
                "mcp: {{erpnext: {{url: '{}', description: The company's ERP}}}}",
                server.url
            ),
        );
    }
    let answers: Vec<String> = [COMPANY, "beta"]
        .into_iter()
        .map(|company| {
            let thread = u.work_for(company);
            let called = mcp(
                &u.computer.env,
                &thread,
                &["call", "erpnext", "whoami", "--json", "{}"],
            )
            .success();
            format!("{company}: {}", stdout(&called))
        })
        .collect();
    insta::assert_snapshot!("each-company-own-server", answers.join("\n"));
}

// spec: project/instrukt/spec/feature/runner/mcp-access UC-MA11
#[test]
fn a_member_uses_a_server_that_answers_an_earlier_protocol_version() {
    let mut c = Company::new("");
    // The earliest version instrukt speaks.
    let earlier = PROTOCOL_VERSIONS[PROTOCOL_VERSIONS.len() - 1];
    let erp = FakeMcp::answering("erp", &[(STEVEN_LOGIN, "steven")], earlier);
    let future = FakeMcp::answering("future", &[], "2099-01-01");
    secret(&c.env, COMPANY, "STEVEN_ERP", STEVEN_LOGIN);
    patch_company(
        &c.env,
        COMPANY,
        &format!(
            "members:\n  steven: {{name: Steven Sonneveld, email: Steven@Example.com, credentials: {{erpnext: '${{STEVEN_ERP}}'}}}}\nmcp:\n  erpnext: {{url: '{}', description: Operate the company's ERPNext records, credential: erpnext}}\n  future: {{url: '{}', description: A server of a later protocol version}}\n",
            erp.url, future.url
        ),
    );
    let home = c.env.instrukt_home().join("companies").join(COMPANY);
    c.runtime.flag("lifecycle", true);
    c.run();
    c.slack
        .mention("C1", "USTEVEN", "<@UAGENT> Book the open invoice", None);
    let session = c.session();
    let state = instrukt_core::home::Home::at_user_home(c.env.home().into());
    let caller = instrukt_core::ids::SessionRef::parse(&format!("t3:{session}")).unwrap();
    wait("the session's recorded binding", || {
        instrukt_runner::assignments::resolve(
            &state,
            &caller,
            instrukt_todos::work::recorded_assignment,
        )
        .unwrap()
        .is_some()
    });
    // A server answering an earlier version instrukt speaks gives its
    // catalog and its tool's result.
    let tools = mcp_in(&c.env, &home, &session, &["list-tools", "erpnext"]).success();
    insta::assert_snapshot!("earlier-version-tools", stdout(&tools));
    let called = mcp_in(
        &c.env,
        &home,
        &session,
        &["call", "erpnext", "whoami", "--json", "{}"],
    )
    .success();
    insta::assert_snapshot!("earlier-version-call", stdout(&called));
    // Every request of a session after its initialization names the version
    // the server answered.
    let seen = |server: &FakeMcp| {
        server
            .received()
            .into_iter()
            .map(|r| (r.method, r.protocol_version))
            .collect::<Vec<_>>()
    };
    let rpc =
        |method: &str, version: Option<&str>| (Some(method.to_owned()), version.map(str::to_owned));
    let end = |version: &str| (None, Some(version.to_owned()));
    assert_eq!(
        seen(&erp),
        vec![
            rpc(INITIALIZE, None),
            rpc(INITIALIZED, Some(earlier)),
            rpc(LIST_TOOLS, Some(earlier)),
            rpc(LIST_TOOLS, Some(earlier)),
            end(earlier),
            rpc(INITIALIZE, None),
            rpc(INITIALIZED, Some(earlier)),
            rpc(CALL_TOOL, Some(earlier)),
            end(earlier),
        ]
    );
    // A server answering a version instrukt does not speak is refused,
    // naming both; nothing follows its initialization but the session's end,
    // which carries the version instrukt offered.
    let refused = mcp_in(&c.env, &home, &session, &["list-tools", "future"]).failure();
    insta::assert_snapshot!("unspoken-version-refused", stderr(&refused));
    assert_eq!(
        seen(&future),
        vec![rpc(INITIALIZE, None), end(PROTOCOL_VERSIONS[0])]
    );
}
