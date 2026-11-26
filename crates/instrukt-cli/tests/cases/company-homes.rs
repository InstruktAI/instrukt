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
            instrukt_core::config::GLOBAL_SCHEMA_VERSION
        ),
    )
    .unwrap();
    company(
        &env,
        "sunfield",
        Some("{webhooks: {erp: {secret: '${SUN_HOOK}'}}}"),
    );
    let sunfield = env.instrukt_home().join("companies/sunfield/company.yaml");
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
    let acme = env.instrukt_home().join("companies/acme/company.yaml");
    let created = std::fs::read(&acme).unwrap();
    env.run(&here, ["config", "init", "--company", "acme"])
        .success();
    assert_eq!(std::fs::read(&acme).unwrap(), created);
    assert_eq!(std::fs::read(&sunfield).unwrap(), before);

    let misnamed = env.instrukt_home().join("companies/misnamed");
    std::fs::create_dir_all(&misnamed).unwrap();
    std::fs::write(
        misnamed.join("company.yaml"),
        format!(
            "schema_version: {}\ncompany: acme\n",
            instrukt_core::config::COMPANY_SCHEMA_VERSION
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
        env.instrukt_home().join("companies/bare/company.yaml"),
        format!(
            "schema_version: {}\ncompany: bare\nwebhooks: {{erp: {{secret: '${{HOOK_KEY}}'}}}}\n",
            instrukt_core::config::COMPANY_SCHEMA_VERSION
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

// spec: project/instrukt/spec/feature/configuration/company-homes UC-CH3
#[test]
fn members_are_listed_with_their_addresses_and_logins_as_references_and_a_shared_address_is_refused()
 {
    let env = TestEnv::new();
    let here = env.path("");
    company(
        &env,
        "acme",
        Some(
            "{members: {steven: {name: Steven Sonneveld, email: steven@example.com, credentials: {erpnext: '${STEVEN_ERPNEXT_LOGIN}'}}, anna: {email: anna@example.com}}}",
        ),
    );
    env.run(&here, ["config", "validate", "--company", "acme"])
        .success();
    insta::assert_snapshot!(
        "company-members",
        stdout(
            &env.run(&here, ["config", "get", "--company", "acme", "members"])
                .success()
        )
    );
    // A second member listing the same address, written in other case, is
    // refused, naming both.
    let revision = stdout(
        &env.run(&here, ["config", "get", "--company", "acme", "--revision"])
            .success(),
    );
    let refused = env
        .run(
            &here,
            [
                "config",
                "patch",
                "--company",
                "acme",
                "--revision",
                revision.trim(),
                "--yaml",
                "{members: {bram: {email: Steven@Example.com, credentials: {erpnext: plain-password}}}}",
            ],
        )
        .failure();
    let home = env.instrukt_home().display().to_string();
    insta::assert_snapshot!(
        "company-members-refused",
        String::from_utf8_lossy(&refused.get_output().stderr).replace(&home, "[INSTRUKT_HOME]")
    );
}

// spec: project/instrukt/spec/feature/configuration/company-homes UC-CH4
#[test]
fn a_companys_secrets_stay_out_of_its_repository() {
    let env = TestEnv::new();
    company(&env, "acme", None);
    secret(&env, "acme", "ERP_TOKEN", "erp-token-value");
    let dir = env.instrukt_home().join("companies/acme");
    // A home set up before its ignore lines were written: an existing
    // settings document in a git repository whose .gitignore holds only the
    // operator's own line.
    std::fs::write(dir.join(".gitignore"), "*.log\n").unwrap();
    std::fs::create_dir_all(dir.join("projects/erp")).unwrap();
    std::fs::write(dir.join("projects/erp/README.md"), "a checkout\n").unwrap();
    std::fs::create_dir_all(dir.join(".instrukt")).unwrap();
    std::fs::write(dir.join(".instrukt/state"), "local\n").unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(dir.join("docs/index.yaml"), "expanded: []\n").unwrap();
    env.git(&dir, ["init", "--quiet", "--initial-branch=main"]);
    env.run(&env.path(""), ["config", "init", "--company", "acme"])
        .success();
    insta::assert_snapshot!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), @r"
    *.log
    /company.yaml
    /secrets.env
    /projects/
    /.instrukt/
    ");
    env.git(&dir, ["add", "-A"]);
    insta::assert_snapshot!(env.git(&dir, ["diff", "--cached", "--name-only"]), @r"
    .gitignore
    docs/index.yaml
    ");
    // The secrets file tracked anyway: validating the company names it.
    env.git(&dir, ["add", "-f", "secrets.env"]);
    let validated = env
        .run(&env.path(""), ["config", "validate", "--company", "acme"])
        .failure();
    let base = env.path("").display().to_string();
    insta::assert_snapshot!(
        stdout(&validated).replace(base.trim_end_matches('/'), "[TMP]"),
        @"[TMP]/home/.instrukt/companies/acme/secrets.env: repository: the home's repository tracks it; it stays on this computer, so untrack it with `git rm -r --cached` and commit"
    );
}

/// A computer whose fake GitHub serves the owning organisation `homes`, the
/// project App installed on it with repository administration, and
/// `acme-admin`, a person's account; git reaches the fake's repositories in
/// the test's `remotes/` folder; instrukt's hook sections find the built
/// binary.
fn company_computer() -> (TestEnv, common::github::FakeGitHub, std::path::PathBuf) {
    let mut env = TestEnv::new();
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_instrukt"))
        .parent()
        .unwrap();
    let path = format!("{}:{}", bin.display(), env.injected("PATH").unwrap());
    env.set_env("PATH", &path);
    let github = env.with_fake_github();
    github
        .set_installation_permissions(
            1,
            serde_json::json!({"issues":"write","contents":"write","pull_requests":"write","checks":"read","statuses":"read","organization_projects":"write","administration":"write"}),
        )
        .install_on_org("homes")
        .set_account("homes", "Organization")
        .set_account("acme-admin", "User")
        .set_account("ana", "User");
    let remotes = env.route_github_remotes(&github.url());
    github.serve_remotes(&remotes);
    (env, github, remotes)
}

/// Declares `org` as the owning organisation through a revision-checked
/// global patch.
fn declare_owner(env: &TestEnv, org: &str) {
    let here = env.path("");
    let revision = stdout(
        &env.run(&here, ["config", "get", "--global", "--revision"])
            .success(),
    );
    env.run(
        &here,
        [
            "config",
            "patch",
            "--global",
            "--revision",
            revision.trim(),
            "--yaml",
            &format!("{{github: {{org: {org}}}}}"),
        ],
    )
    .success();
}

/// The GitHub calls the fake recorded that change something.
fn writes(github: &common::github::FakeGitHub) -> Vec<String> {
    github
        .requests()
        .into_iter()
        .filter(|request| request.method != "GET" && !request.path.ends_with("/access_tokens"))
        .filter(|request| request.path != "/graphql")
        .map(|request| format!("{} {}", request.method, request.path))
        .collect()
}

/// Every file the commit `main` of the bare repository `remote` holds.
fn files_on_main(env: &TestEnv, remote: &std::path::Path) -> Vec<String> {
    env.git(remote, ["ls-tree", "-r", "--name-only", "main"])
        .lines()
        .map(ToOwned::to_owned)
        .collect()
}

/// The text `main` of the bare repository `remote` holds at `path`.
fn on_main(env: &TestEnv, remote: &std::path::Path, path: &str) -> String {
    env.git(remote, ["show", &format!("main:{path}")])
}

fn stderr(assert: &assert_cmd::assert::Assert) -> String {
    String::from_utf8_lossy(&assert.get_output().stderr).into_owned()
}

/// `text` with the test's own temporary folder, as written and as resolved,
/// replaced by one placeholder, so a snapshot holds no machine path.
fn scrubbed(env: &TestEnv, text: &str) -> String {
    let base = env.path("");
    let base = base.to_str().unwrap().trim_end_matches('/');
    let resolved = std::fs::canonicalize(base).unwrap();
    text.replace(resolved.to_str().unwrap(), "[TMP]")
        .replace(base, "[TMP]")
}

/// Every string in `value`, at any depth; other scalars are left out.
fn values(value: &serde_yaml_ng::Value, out: &mut Vec<String>) {
    match value {
        serde_yaml_ng::Value::Mapping(map) => map.values().for_each(|v| values(v, out)),
        serde_yaml_ng::Value::Sequence(items) => items.iter().for_each(|v| values(v, out)),
        serde_yaml_ng::Value::String(text) => out.push(text.clone()),
        _ => {}
    }
}

// spec: project/instrukt/spec/feature/configuration/company-homes UC-CH5
#[test]
fn adding_a_company_generates_its_home_the_same_as_every_other() {
    let (env, github, remotes) = company_computer();
    let add = [
        "init",
        "--company",
        "acme",
        "--name",
        "Acme",
        "--admin",
        "acme-admin",
    ];
    let acme_home = env.instrukt_home().join("companies/acme");

    // No organisation is declared: nothing is created, and the operator is
    // told the organisation is a prerequisite.
    let refused = env.run(&env.path(""), add).failure();
    insta::assert_snapshot!("no-owner-refused", scrubbed(&env, &stderr(&refused)));
    // The declared owner is a person's account: refused the same way.
    let machine = env.instrukt_home().join("instrukt.yaml");
    let declared = std::fs::read_to_string(&machine).unwrap();
    let mut layer: serde_yaml_ng::Value = serde_yaml_ng::from_str(&declared).unwrap();
    layer["github"]["org"] = "ana".into();
    std::fs::write(&machine, serde_yaml_ng::to_string(&layer).unwrap()).unwrap();
    let refused = env.run(&env.path(""), add).failure();
    insta::assert_snapshot!("personal-owner-refused", scrubbed(&env, &stderr(&refused)));
    std::fs::write(&machine, &declared).unwrap();
    assert_eq!(writes(&github), Vec::<String>::new());
    assert!(!acme_home.exists());
    assert!(!remotes.join("homes").exists());

    // The operator declares the owning organisation; their agent adds the
    // company from a working directory inside another company's home, in a
    // repository whose configuration cannot be read, which refuses no one.
    declare_owner(&env, "homes");
    let elsewhere = env
        .instrukt_home()
        .join("companies/globex-old/projects/tool");
    std::fs::create_dir_all(&elsewhere).unwrap();
    env.git(&elsewhere, ["init", "--quiet"]);
    std::fs::write(elsewhere.join("instrukt.yaml"), "schema_version: [\n").unwrap();
    let added = env.run(&elsewhere, add).success();
    insta::assert_snapshot!("company-created", scrubbed(&env, &stdout(&added)));

    // The organisation holds the company's private repository, named after
    // its key, holding the company skeleton.
    let remote = remotes.join("homes/company-acme.git");
    // Each file of the skeleton is committed on main as the company receives
    // it, and nothing else is.
    let committed = [
        ("skeleton-layer", "instrukt.yaml"),
        ("skeleton-ignore", ".gitignore"),
        ("skeleton-index", "docs/index.yaml"),
        ("skeleton-instructions", "AGENTS.md"),
        ("skeleton-skill", "skills/acme/SKILL.md"),
        ("skeleton-catalog", ".claude-plugin/marketplace.json"),
    ];
    for (snapshot, path) in committed {
        insta::assert_snapshot!(snapshot, on_main(&env, &remote, path));
    }
    let skeleton = files_on_main(&env, &remote);
    assert_eq!(skeleton.len(), committed.len());
    assert_eq!(github.is_private("homes/company-acme"), Some(true));
    // The company's GitHub administrator is an administrator of it, and its
    // default branch is set.
    insta::assert_debug_snapshot!(
        "company-created-github",
        (
            github.collaborators("homes/company-acme"),
            github.default_branch("homes/company-acme"),
        )
    );

    // The computer holds the home as the checkout of that repository, ready
    // as a project like any other with no step taken by hand.
    assert_eq!(
        env.git(&acme_home, ["rev-parse", "HEAD"]),
        env.git(&remote, ["rev-parse", "main"])
    );
    assert!(acme_home.join("company.yaml").is_file());
    assert!(env.git(&acme_home, ["status", "--porcelain"]).is_empty());
    env.run(&env.path(""), ["config", "validate", "--company", "acme"])
        .success();
    let hooks = stdout(&env.run(&acme_home, ["hooks", "status"]).success());
    insta::assert_snapshot!("home-hooks", scrubbed(&env, &hooks));
    assert!(
        env.instrukt_home()
            .join("state/publication/company-acme.yaml")
            .is_file()
    );
    // The company's knowledge is committed on main, a document under
    // `docs/project/` among it: the commit carries the AGENTS.md rendered
    // from it, and publishing the home puts none of it on the computer.
    let about =
        "---\ndescription: Who Acme is.\n---\n\n# About Acme\n\nAcme builds bicycles to order.\n";
    let notes =
        "---\ndescription: How Acme plans its week.\n---\n\n# Planning\n\nAcme plans on Mondays.\n";
    std::fs::create_dir_all(acme_home.join("docs/project")).unwrap();
    std::fs::write(acme_home.join("docs/about.md"), about).unwrap();
    std::fs::write(acme_home.join("docs/project/planning.md"), notes).unwrap();
    std::fs::write(
        acme_home.join("docs/index.yaml"),
        "expanded: [about.md]\nlisted: [project/planning.md]\n",
    )
    .unwrap();
    env.git(&acme_home, ["add", "docs"]);
    env.git(&acme_home, ["commit", "--quiet", "-m", "Who Acme is"]);
    env.finished_hook_run(&acme_home, instrukt_core::repository::Hook::PostCommit);
    insta::assert_snapshot!(
        "home-commit-instructions",
        env.git(&acme_home, ["show", "HEAD:AGENTS.md"])
    );
    env.run(&acme_home, ["sync"]).success();
    let docs = env.instrukt_home().join("docs");
    for entry in walkdir::WalkDir::new(&docs)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() {
            let bytes = std::fs::read(entry.path()).unwrap();
            assert!(
                bytes != about.as_bytes() && bytes != notes.as_bytes(),
                "{} holds a company document",
                entry.path().display()
            );
        }
    }
    assert!(!docs.join("project/company-acme").exists());
    let index: serde_yaml_ng::Value = serde_yaml_ng::from_str(
        &std::fs::read_to_string(env.instrukt_home().join("state/index.yaml")).unwrap_or_default(),
    )
    .unwrap_or(serde_yaml_ng::Value::Null);
    let mut named = Vec::new();
    values(&index, &mut named);
    let repository = instrukt_core::config::company_repository(
        &instrukt_core::ids::ProjectSlug::parse("acme").unwrap(),
    )
    .unwrap();
    assert!(
        !named.iter().any(|value| value == repository.as_str()),
        "{named:?}"
    );

    // A second company's home holds the same skeleton, differing only where
    // its identity enters it.
    env.run(
        &env.path(""),
        [
            "init",
            "--company",
            "globex",
            "--name",
            "Globex",
            "--admin",
            "acme-admin",
        ],
    )
    .success();
    let globex_remote = remotes.join("homes/company-globex.git");
    assert_eq!(github.is_private("homes/company-globex"), Some(true));
    let identity = |text: String| text.replace("acme", "globex").replace("Acme", "Globex");
    assert_eq!(
        files_on_main(&env, &globex_remote),
        skeleton
            .iter()
            .map(|file| identity(file.clone()))
            .collect::<Vec<_>>()
    );
    for file in &skeleton {
        assert_eq!(
            on_main(&env, &globex_remote, &identity(file.clone())),
            identity(on_main(&env, &remote, file)),
            "{file}"
        );
    }

    // The company already has a home: nothing of it is changed, and the
    // operator is told the home exists.
    let before = common::Snapshot::over(vec![], vec![acme_home.clone()]);
    let recorded = writes(&github).len();
    let refused = env.run(&env.path(""), add).failure();
    insta::assert_snapshot!("existing-home-refused", scrubbed(&env, &stderr(&refused)));
    assert_eq!(before.changed(), Vec::<std::path::PathBuf>::new());
    assert_eq!(writes(&github).len(), recorded);
}

// spec: project/instrukt/spec/feature/configuration/company-homes UC-CH6
#[test]
fn another_computer_gets_an_existing_companys_home() {
    // The company's repository exists in the owning organisation: another
    // computer created it.
    let (first, _first_github, first_remotes) = company_computer();
    declare_owner(&first, "homes");
    first
        .run(
            &first.path(""),
            [
                "init",
                "--company",
                "acme",
                "--name",
                "Acme",
                "--admin",
                "acme-admin",
            ],
        )
        .success();
    let (env, github, remotes) = company_computer();
    declare_owner(&env, "homes");
    std::fs::create_dir_all(remotes.join("homes")).unwrap();
    let copied = std::process::Command::new("cp")
        .arg("-R")
        .arg(first_remotes.join("homes/company-acme.git"))
        .arg(remotes.join("homes/"))
        .status()
        .unwrap();
    assert!(copied.success());
    let remote = remotes.join("homes/company-acme.git");
    let refs = env.git(&remote, ["for-each-ref"]);
    let home = env.instrukt_home().join("companies/acme");
    assert!(!home.exists());

    // Their agent adds the company by its key.
    let added = env
        .run(&env.path(""), ["init", "--company", "acme"])
        .success();
    insta::assert_snapshot!("company-checked-out", scrubbed(&env, &stdout(&added)));

    // The computer holds the home as the checkout of that repository, ready
    // as a project like any other.
    assert_eq!(
        env.git(&home, ["rev-parse", "HEAD"]),
        env.git(&remote, ["rev-parse", "main"])
    );
    assert!(home.join("company.yaml").is_file());
    env.run(&env.path(""), ["config", "validate", "--company", "acme"])
        .success();
    let hooks = stdout(&env.run(&home, ["hooks", "status"]).success());
    insta::assert_snapshot!("other-computer-hooks", scrubbed(&env, &hooks));
    assert!(
        env.instrukt_home()
            .join("state/publication/company-acme.yaml")
            .is_file()
    );
    // Nothing of the repository on GitHub is changed.
    assert_eq!(writes(&github), Vec::<String>::new());
    assert_eq!(env.git(&remote, ["for-each-ref"]), refs);

    // Another company's repository was made before generation: it holds a
    // README and no project layer.
    let made = env.path("made-before-generation");
    std::fs::create_dir_all(&made).unwrap();
    env.git(&made, ["init", "--quiet", "--initial-branch=main"]);
    std::fs::write(made.join("README.md"), "# Globex\n").unwrap();
    env.git(&made, ["add", "README.md"]);
    env.git(&made, ["commit", "--quiet", "-m", "Globex"]);
    let globex_remote = remotes.join("homes/company-globex.git");
    env.git(
        &made,
        [
            std::ffi::OsStr::new("clone"),
            std::ffi::OsStr::new("--quiet"),
            std::ffi::OsStr::new("--bare"),
            made.as_os_str(),
            globex_remote.as_os_str(),
        ],
    );
    let globex_refs = env.git(&globex_remote, ["for-each-ref"]);
    // Their agent adds that company by its key: the home is its checkout,
    // and the report names what does not apply there and the way to it.
    let added = env
        .run(&env.path(""), ["init", "--company", "globex"])
        .success();
    insta::assert_snapshot!(
        "company-without-project-layer",
        scrubbed(&env, &stdout(&added))
    );
    let globex = env.instrukt_home().join("companies/globex");
    assert_eq!(
        env.git(&globex, ["rev-parse", "HEAD"]),
        env.git(&globex_remote, ["rev-parse", "main"])
    );
    assert_eq!(writes(&github), Vec::<String>::new());
    assert_eq!(env.git(&globex_remote, ["for-each-ref"]), globex_refs);
    // The door it names publishes the home once the repository's default
    // branch holds a project layer declaring it a company's.
    std::fs::write(
        globex.join("instrukt.yaml"),
        "schema_version: 6\nproject: company-globex\ncompany_repository: true\n",
    )
    .unwrap();
    env.git(&globex, ["add", "instrukt.yaml"]);
    env.git(
        &globex,
        [
            "commit",
            "--quiet",
            "-m",
            "Declare the company's repository",
        ],
    );
    env.git(&globex, ["push", "--quiet", "origin", "main"]);
    env.run(&globex, ["sync"]).success();
    assert!(
        env.instrukt_home()
            .join("state/publication/company-globex.yaml")
            .is_file()
    );
}
