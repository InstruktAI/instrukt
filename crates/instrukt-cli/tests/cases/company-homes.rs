//! Company homes through the real binary: their documents, their own
//! secrets and the receiver resolving each company's signing value from its
//! own home alone.
#![allow(clippy::unwrap_used, reason = "test setup and assertions")]

use crate::common;
use crate::fixtures::event_transport::Computer;
use crate::fixtures::webhooks::{self as hooks, BODY, Receiver, SECRET, declare};
use common::TestEnv;
use common::redis::RedisServer;

fn stdout(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stdout).into_owned()
}

/// Creates the company home `name` through `config init --company` and
/// applies `settings` to it with a revision-checked patch.
fn company(env: &TestEnv, name: &str, settings: Option<&str>) {
    let here = env.path("");
    env.run(&here, ["config", "init", "--company", name])
        .success();
    if let Some(settings) = settings {
        let revision = stdout(
            &env.run(&here, ["config", "get", "--company", name, "--revision"])
                .success(),
        );
        env.run(
            &here,
            [
                "config",
                "patch",
                "--company",
                name,
                "--revision",
                revision.trim(),
                "--yaml",
                settings,
            ],
        )
        .success();
    }
}

fn secret(env: &TestEnv, company: &str, name: &str, value: &str) {
    env.instrukt(&env.path(""))
        .args(["secrets", "set", name, "--company", company])
        .write_stdin(value)
        .assert()
        .success();
}

// spec: project/instrukt/spec/feature/configuration/company-homes UC-CH1
#[test]
fn a_company_is_added_beside_the_others_and_a_misnamed_folder_is_no_company() {
    let env = TestEnv::new();
    let here = env.path("");
    std::fs::create_dir_all(env.instrukt_home()).unwrap();
    std::fs::write(
        env.instrukt_home().join("instrukt.yaml"),
        format!(
            "schema_version: {}\nmodules: {{events: true}}\n",
            instrukt_core::config::CURRENT_SCHEMA_VERSION
        ),
    )
    .unwrap();
    company(
        &env,
        "sunfield",
        Some("{webhooks: {erp: {secret: '${SUN_HOOK}'}}}"),
    );
    let sunfield = env.instrukt_home().join("companies/sunfield/instrukt.yaml");
    let before = std::fs::read(&sunfield).unwrap();

    company(
        &env,
        "acme",
        Some("{webhooks: {erp: {secret: '${ACME_HOOK}'}}}"),
    );
    // The company's settings read back by its name, its reference unexpanded.
    insta::assert_snapshot!(
        "company-settings",
        stdout(
            &env.run(&here, ["config", "get", "--company", "acme"])
                .success()
        )
    );
    env.run(&here, ["config", "validate", "--company", "acme"])
        .success();
    // A repeated init keeps the home as it is.
    let acme = env.instrukt_home().join("companies/acme/instrukt.yaml");
    let created = std::fs::read(&acme).unwrap();
    env.run(&here, ["config", "init", "--company", "acme"])
        .success();
    assert_eq!(std::fs::read(&acme).unwrap(), created);
    assert_eq!(std::fs::read(&sunfield).unwrap(), before);

    let misnamed = env.instrukt_home().join("companies/misnamed");
    std::fs::create_dir_all(&misnamed).unwrap();
    std::fs::write(
        misnamed.join("instrukt.yaml"),
        format!(
            "schema_version: {}\ncompany: acme\n",
            instrukt_core::config::CURRENT_SCHEMA_VERSION
        ),
    )
    .unwrap();
    let home = env.instrukt_home().display().to_string();
    let findings = env
        .run(&here, ["config", "validate", "--company", "misnamed"])
        .failure();
    insta::assert_snapshot!(
        "misnamed-company-findings",
        stdout(&findings).replace(&home, "[INSTRUKT_HOME]")
    );
    let read = env
        .run(&here, ["config", "get", "--company", "misnamed"])
        .failure();
    let refusal =
        String::from_utf8_lossy(&read.get_output().stderr).replace(&home, "[INSTRUKT_HOME]");
    insta::assert_snapshot!("misnamed-company-read", refusal);
}

// spec: project/instrukt/spec/feature/configuration/company-homes UC-CH2
#[test]
fn each_company_resolves_its_own_secret_and_never_the_machines_or_anothers() {
    let computer = Computer::new();
    let redis = RedisServer::new(&computer.env);
    // The machine's file holds HOOK_KEY too; the companies reuse the name.
    declare(&computer, "{erp: {secret: '${HOOK_KEY}'}}");
    let receiver = Receiver::start(&computer, &redis);
    let env = &computer.env;
    let values = [
        ("acme", "acme-signing-value"),
        ("sunfield", "sunfield-signing-value"),
    ];
    for (name, value) in values {
        env.run(&env.path(""), ["config", "init", "--company", name])
            .success();
        secret(env, name, "HOOK_KEY", value);
        company(
            env,
            name,
            Some("{webhooks: {erp: {secret: '${HOOK_KEY}'}}}"),
        );
    }
    // A home whose own file lacks the value gets none from the machine.
    company(env, "bare", None);
    secret(env, "bare", "OTHER", "unrelated-value");
    std::fs::write(
        env.instrukt_home().join("companies/bare/instrukt.yaml"),
        format!(
            "schema_version: {}\ncompany: bare\nwebhooks: {{erp: {{secret: '${{HOOK_KEY}}'}}}}\n",
            instrukt_core::config::CURRENT_SCHEMA_VERSION
        ),
    )
    .unwrap();

    let route = |name: &str| format!("/hooks/inbound/company/{name}/erp");
    assert_eq!(
        receiver.signed(&route("acme"), BODY, values[0].1, &[]).0,
        202
    );
    assert_eq!(
        receiver
            .signed(&route("sunfield"), BODY, values[1].1, &[])
            .0,
        202
    );
    for wrong in [values[1].1, SECRET] {
        assert_eq!(receiver.signed(&route("acme"), BODY, wrong, &[]).0, 401);
    }
    assert_eq!(receiver.signed(&route("bare"), BODY, SECRET, &[]).0, 500);
    assert_eq!(hooks::events(&redis).len(), 2);

    // Reads and the finding name references and homes, never a value.
    let home = env.instrukt_home().display().to_string();
    let mut output = String::new();
    for name in ["acme", "sunfield", "bare"] {
        output.push_str(&stdout(
            &env.run(
                &env.path(""),
                ["config", "get", "--company", name, "--json"],
            )
            .success(),
        ));
    }
    let validated = env
        .run(&env.path(""), ["config", "validate", "--company", "bare"])
        .failure();
    output.push_str(&stdout(&validated));
    output.push_str(&String::from_utf8_lossy(&validated.get_output().stderr));
    insta::assert_snapshot!(
        "company-secrets-undisclosed",
        output.replace(&home, "[INSTRUKT_HOME]")
    );
}
