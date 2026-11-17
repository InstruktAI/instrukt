//! Agents reach declared MCP servers through `instrukt mcp`: the real
//! binary, MCP server stand-ins on loopback addresses, a fake T3 host, and
//! either the Slack stand-in with the long-lived runner for a person's
//! conversation or cluster Redis with a tick for a company's unattended work.
#![allow(clippy::unwrap_used, reason = "isolated CLI setup")]
use crate::common;
use crate::fixtures::conversing::{COMPANY, Company, patch_company, wait};
use crate::fixtures::documented;
use crate::fixtures::event_transport::Computer;
use crate::fixtures::mcp::FakeMcp;
use crate::fixtures::t3::FakeT3;
use common::TestEnv;
use common::redis::RedisServer;
use instrukt_runner::transport::EVENT_STREAM;

/// The speaker's own login for the company's ERP.
const STEVEN_LOGIN: &str = "steven-erp-login-7f3a";
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
    env.instrukt(&env.path(""))
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
    let erp = FakeMcp::start();
    let crm = FakeMcp::start();
    secret(&c.env, COMPANY, "STEVEN_ERP", STEVEN_LOGIN);
    secret(&c.env, COMPANY, "ACME_ERP", COMPANY_LOGIN);
    secret(&c.env, COMPANY, "ACME_CRM", CRM_LOGIN);
    patch_company(
        &c.env,
        COMPANY,
        &format!(
            "members: {{steven: {{name: Steven Sonneveld, email: Steven@Example.com, credentials: {{erpnext: '${{STEVEN_ERP}}'}}}}}}\nmcp:\n  erpnext: {{url: '{}', description: Operate the company's ERPNext records, credential: erpnext, login: '${{ACME_ERP}}'}}\n  crm: {{url: '{}', description: The company's customer records, credential: crm, login: '${{ACME_CRM}}'}}\n",
            erp.url, crm.url
        ),
    );
    c.run();
    // Steven asks; his turn is open while the agent works.
    c.slack
        .mention("C1", "USTEVEN", "<@UAGENT> Book the open invoice", None);
    let session = c.session();
    // The runner has recorded the session as the conversation's.
    let home = instrukt_core::home::Home::at_user_home(c.env.home().into());
    let caller = instrukt_core::ids::SessionRef::parse(&format!("t3:{session}")).unwrap();
    wait("the session's recorded binding", || {
        instrukt_runner::assignments::resolve(
            &home,
            &caller,
            instrukt_todos::work::recorded_assignment,
        )
        .unwrap()
        .is_some()
    });
    // The call reaches the server under Steven's own login, which the agent
    // never supplied; what the server returns reaches the agent with the
    // login named by its reference, never its value.
    let called = mcp(
        &c.env,
        &session,
        &["call", "erpnext", "whoami", "--json", "{}"],
    )
    .success();
    insta::assert_snapshot!("speaker-login-call", stdout(&called));
    // A failure the server reports reaches the agent with the server's cause
    // and a non-zero exit, the login again named by its reference.
    let failed = mcp(
        &c.env,
        &session,
        &["call", "erpnext", "fail", "--json", "{}"],
    )
    .failure();
    insta::assert_snapshot!("speaker-login-failure", stderr(&failed));
    assert_eq!(
        erp.call_logins(),
        [Some(STEVEN_LOGIN.to_owned()), Some(STEVEN_LOGIN.to_owned())]
    );
    // Steven has no login of his own for the CRM: the call is refused naming
    // him and the missing login, and no other login reaches the server.
    let refused = mcp(&c.env, &session, &["call", "crm", "whoami", "--json", "{}"]).failure();
    insta::assert_snapshot!("speaker-login-missing", stderr(&refused));
    assert_eq!(crm.received().len(), 0);
}

// spec: project/instrukt/spec/feature/runner/mcp-access UC-MA2
#[test]
fn unattended_company_work_carries_the_companys_own_login() {
    let u = Unattended::new(&[COMPANY]);
    let erp = FakeMcp::start();
    let crm = FakeMcp::start();
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
    mcp(
        &u.computer.env,
        &thread,
        &["call", "erpnext", "whoami", "--json", "{}"],
    )
    .success();
    assert_eq!(erp.call_logins(), [Some(COMPANY_LOGIN.to_owned())]);
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
    let erp = FakeMcp::start();
    let search = FakeMcp::start();
    let vault = FakeMcp::start();
    let replaced = FakeMcp::start();
    secret(&u.computer.env, COMPANY, "ACME_ERP", COMPANY_LOGIN);
    patch_company(
        &u.computer.env,
        COMPANY,
        &format!(
            "mcp:\n  erpnext: {{url: '{}', description: Operate the company's ERPNext records, credential: erpnext, login: '${{ACME_ERP}}'}}\n",
            erp.url
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
}

// spec: project/instrukt/spec/feature/runner/mcp-access UC-MA4
#[test]
fn each_companys_server_of_one_name_is_reached_at_its_own_address() {
    let u = Unattended::new(&[COMPANY, "beta"]);
    let acme = FakeMcp::start();
    let beta = FakeMcp::start();
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
    for company in [COMPANY, "beta"] {
        let thread = u.work_for(company);
        mcp(
            &u.computer.env,
            &thread,
            &["call", "erpnext", "whoami", "--json", "{}"],
        )
        .success();
    }
    assert_eq!(acme.call_logins().len(), 1);
    assert_eq!(beta.call_logins().len(), 1);
}
