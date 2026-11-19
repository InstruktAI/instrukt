//! Rendered agent instructions from `instrukt sync`
//! (`docs/project/spec/feature/publication/agent-entrypoints.md`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test: a failed step fails the test"
)]

use crate::common;
use assert_cmd::assert::Assert;
use common::{Repo, TestEnv};

/// A snippet of `taxonomy` (policy, principle or procedure) with every
/// required section, `extra` frontmatter lines and `reads` as its required
/// reads.
fn snippet(taxonomy: &str, title: &str, extra: &str, reads: &[&str]) -> String {
    let (suffix, sections): (&str, &[&str]) = match taxonomy {
        "policy" => (
            "Policy",
            &["Rules", "Rationale", "Scope", "Enforcement", "Exceptions"],
        ),
        "principle" => (
            "Principle",
            &["Principle", "Rationale", "Implications", "Tensions"],
        ),
        _ => (
            "Procedure",
            &["Goal", "Preconditions", "Steps", "Outputs", "Recovery"],
        ),
    };
    let mut text =
        format!("---\ndescription: About {title}.\n{extra}---\n\n# {title} — {suffix}\n");
    if !reads.is_empty() {
        text.push_str("\n## Required reads\n\n");
        for read in reads {
            text.push_str(&format!("- {read}\n"));
        }
    }
    for section in sections {
        text.push_str(&format!("\n## {section}\n\n{title}: {section}.\n"));
    }
    text
}

fn layer(slug: &str) -> String {
    format!("schema_version: 1\nproject: {slug}\n")
}

/// Emitted text with the test's temporary directory shown as `[TMP]`.
fn shown(env: &TestEnv, bytes: &[u8]) -> String {
    let base = env.path("").display().to_string();
    String::from_utf8_lossy(bytes).replace(base.trim_end_matches('/'), "[TMP]")
}

fn stdout(env: &TestEnv, assert: &Assert) -> String {
    shown(env, &assert.get_output().stdout)
}

/// Every file below `dir` outside `.git`, with its bytes.
fn files_below(dir: &std::path::Path) -> std::collections::BTreeMap<std::path::PathBuf, Vec<u8>> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_entry(|e| e.file_name() != ".git")
        .map(Result::unwrap)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            (
                e.path().strip_prefix(dir).unwrap().to_path_buf(),
                std::fs::read(e.path()).unwrap(),
            )
        })
        .collect()
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE9
#[test]
fn ci_catches_instructions_a_commit_left_stale() {
    let env = TestEnv::new();
    let instrukt_home = env.instrukt_home();
    assert!(!instrukt_home.exists(), "the home holds no instrukt");
    let home_at_start = files_below(env.home());
    let repo = env.git_repo("demo");
    // Every `docs render` and `docs render --check` leaves the home as the
    // test found it: no instrukt home, no log, whatever the outcome.
    let render = |args: &[&str]| -> Assert {
        let mut all = vec!["docs", "render"];
        all.extend_from_slice(args);
        let run = env.run(repo.path(), all);
        assert_eq!(
            files_below(env.home()),
            home_at_start,
            "`docs render {}` wrote in the home",
            args.join(" ")
        );
        run
    };

    // An unfinished index that no rendering reads blocks nothing, with no
    // master and with one.
    repo.write("instrukt.yaml", &layer("demo"))
        .write("docs/domain/unselected/index.yaml", "[: no\n");
    let nothing = render(&[]).success();
    insta::assert_snapshot!(stdout(&env, &nothing), @"");
    assert!(!repo.path().join("AGENTS.md").exists());
    repo.write("AGENTS.master.md", "## Purpose\n\nThe demo project.\n")
        .write(
            "docs/project/policy/house.md",
            &snippet("policy", "House", "", &["sdlc/policy/tests"]),
        )
        .write(
            "docs/project/policy/style.md",
            &snippet("policy", "Style", "", &[]),
        )
        .write(
            "docs/project/procedure/release.md",
            &snippet(
                "procedure",
                "Release",
                "activation_trigger: When cutting a release.\n",
                &[],
            ),
        );

    // The developer renders the instruction files and commits them; a
    // project without an index holds its purpose and procedures.
    let rendered = render(&[]).success();
    insta::assert_snapshot!("render_without_project_index", stdout(&env, &rendered));
    repo.commit("instructions");

    // A commit made without instrukt's hook changes the project's index.
    repo.write(
        "docs/project/index.yaml",
        "expanded:\n  - project/demo/policy/house\nlisted:\n  - project/demo/policy/style\n",
    );
    repo.git(["add", "--all"]);
    repo.git(["commit", "--quiet", "--no-verify", "-m", "index"]);
    let tree_before = files_below(repo.path());

    // The check fails, names the stale file and writes nothing.
    let stale = render(&["--check"]).failure();
    insta::assert_snapshot!("check_names_the_stale_file", stdout(&env, &stale));
    assert_eq!(files_below(repo.path()), tree_before);

    // The developer renders and commits; the check passes.
    render(&[]).success();
    repo.commit("rendered");
    insta::assert_snapshot!("committed_root_agents_md", repo.read("AGENTS.md"));
    render(&["--check"]).success();

    // A root master that carries required reads is a finding the check names.
    let purpose = "## Purpose\n\nThe demo project.\n";
    repo.write(
        "AGENTS.master.md",
        &format!("{purpose}\n## Required reads\n\n- project/demo/policy/style\n"),
    );
    let finding = render(&["--check"]).failure();
    insta::assert_snapshot!(
        "check_names_the_root_master_finding",
        stdout(&env, &finding)
    );

    // A master carrying a directive the format does not support is a
    // finding: the file keeps what it held, and both commands fail.
    let committed = repo.read("AGENTS.md");
    for (name, directive) in [
        ("exec", "<!-- @exec: echo harmless -->"),
        ("include", "<!-- @include: notes.md -->"),
    ] {
        repo.write("AGENTS.master.md", &format!("{purpose}\n{directive}\n"));
        let refused = render(&[]).failure();
        let checked = render(&["--check"]).failure();
        assert_eq!(stdout(&env, &checked), stdout(&env, &refused));
        insta::assert_snapshot!(
            format!("unsupported_directive_{name}"),
            stdout(&env, &refused)
        );
        insta::allow_duplicates! {
            // The render meant to write and ends with one line saying so; the
            // check is a report, and its findings are its output.
            insta::assert_snapshot!(
                shown(&env, &refused.get_output().stderr),
                @"instrukt: the findings above leave their files as they were; fix them and run `instrukt docs render` again"
            );
            insta::assert_snapshot!(shown(&env, &checked.get_output().stderr), @"");
        }
        assert_eq!(repo.read("AGENTS.md"), committed);
    }
    assert!(!instrukt_home.exists());
}

/// A project publishing `sdlc` with a description, a triggered procedure
/// and a snippet the general owner's index selects.
fn engineering(env: &TestEnv) -> Repo {
    let repo = env.git_repo("engineering");
    repo.write("instrukt.yaml", &layer("engineering"))
        .write(
            "docs/domain/sdlc/index.yaml",
            "description: Writing, reviewing and shipping code\n",
        )
        .write(
            "docs/domain/sdlc/policy/tests.md",
            &snippet("policy", "Tests", "", &[]),
        )
        .write(
            "docs/domain/sdlc/procedure/ship.md",
            &snippet(
                "procedure",
                "Ship",
                "activation_trigger: When the human asks to ship.\n",
                &[],
            ),
        );
    repo.commit("sdlc");
    repo
}

/// The owner of `docs/general/`: its index selects a general snippet (with
/// its closure), an sdlc snippet and another general snippet in full, lists
/// one, maps an agent policy to `agents`, and it has a triggered procedure.
fn general_owner(env: &TestEnv, agents: &str) -> Repo {
    let repo = env.git_repo("platform");
    repo.write("instrukt.yaml", &layer("platform"))
        .write(
            "docs/general/index.yaml",
            &format!(
                "expanded:\n  - general/policy/care\n  - sdlc/policy/tests\n  - general/policy/kind\n\
                 listed:\n  - general/policy/style\n{agents}"
            ),
        )
        .write(
            "docs/general/policy/care.md",
            &snippet("policy", "Care", "", &["general/principle/attention"]),
        )
        .write(
            "docs/general/principle/attention.md",
            &snippet("principle", "Attention", "", &[]),
        )
        .write(
            "docs/general/policy/kind.md",
            &snippet("policy", "Kind", "", &[]),
        )
        .write(
            "docs/general/policy/style.md",
            &snippet("policy", "Style", "", &[]),
        )
        .write(
            "docs/general/policy/agents/claude.md",
            &snippet("policy", "Claude", "", &[]),
        )
        .write(
            "docs/general/procedure/onboard.md",
            &snippet(
                "procedure",
                "Onboard",
                "activation_trigger: When the human asks to onboard someone.\n",
                &[],
            ),
        );
    repo.commit("general");
    repo
}

const CLAUDE_AGENTS: &str = "agents:\n  claude:\n    - general/policy/agents/claude\n";

#[test]
fn an_agents_snippet_ends_only_its_own_clis_global_file() {
    let env = TestEnv::new();
    let home = env.home();
    for dir in [".claude", ".codex"] {
        std::fs::create_dir_all(home.join(dir)).unwrap();
    }
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(&env, CLAUDE_AGENTS);

    env.run(platform.path(), ["sync"]).success();

    let read = |rel: &str| std::fs::read_to_string(home.join(rel)).unwrap();
    insta::assert_snapshot!("claude_global_file", read(".claude/CLAUDE.md"));
    insta::assert_snapshot!("codex_global_file", read(".codex/AGENTS.md"));
}

#[test]
fn mapping_a_snippet_to_an_unsupported_agent_cli_is_refused_naming_the_supported_ones() {
    let env = TestEnv::new();
    std::fs::create_dir_all(env.home().join(".claude")).unwrap();
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(
        &env,
        "agents:\n  cursor:\n    - general/policy/agents/claude\n",
    );

    let refused = env.run(platform.path(), ["sync"]).failure();

    insta::assert_snapshot!("unsupported_agent_cli", stdout(&env, &refused));
    assert!(!env.home().join(".claude/CLAUDE.md").exists());
}

#[test]
fn a_global_file_keeping_a_reference_is_written_and_named() {
    let env = TestEnv::new();
    let home = env.home();
    std::fs::create_dir_all(home.join(".claude")).unwrap();
    // Another project publishes `ops`; its published snippet then reads as
    // an earlier release left it, with a visible reference validation
    // refuses today.
    let ops = env.git_repo("ops");
    ops.write("instrukt.yaml", &layer("ops"))
        .write("docs/domain/ops/index.yaml", "description: Operations\n")
        .write(
            "docs/domain/ops/policy/house.md",
            &snippet("policy", "House", "", &[]),
        );
    ops.commit("ops");
    env.run(ops.path(), ["sync"]).success();
    let published = env.instrukt_home().join("docs/ops/policy/house.md");
    let text = std::fs::read_to_string(&published).unwrap();
    std::fs::write(
        &published,
        text.replace("House: Rules.", "Read it (@notes/house.md) first."),
    )
    .unwrap();
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(&env, "");
    platform
        .write(
            "docs/general/index.yaml",
            "expanded:\n  - general/policy/care\n  - ops/policy/house\n",
        )
        .commit("select ops");

    let synced = env.run(platform.path(), ["sync"]).success();

    insta::assert_snapshot!(
        "sync_keeping_a_reference",
        stdout(&env, &synced).replace(&platform.head(), "[SHA]")
    );
    insta::assert_snapshot!(
        "global_file_keeping_a_reference",
        std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap()
    );
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE5
#[test]
fn a_domain_without_an_index_is_published_but_absent_from_the_global_files() {
    let env = TestEnv::new();
    std::fs::create_dir_all(env.home().join(".claude")).unwrap();
    env.run(engineering(&env).path(), ["sync"]).success();
    let platform = general_owner(&env, "");
    platform
        .write(
            "docs/domain/ops/policy/oncall.md",
            &snippet("policy", "Oncall", "", &[]),
        )
        .commit("ops without index");

    let synced = env.run(platform.path(), ["sync"]).success();

    insta::assert_snapshot!(
        "sync_domain_without_index",
        stdout(&env, &synced).replace(&platform.head(), "[SHA]")
    );
    insta::assert_snapshot!(
        "global_file_without_unindexed_domain",
        std::fs::read_to_string(env.home().join(".claude/CLAUDE.md")).unwrap()
    );
    let listed = env
        .run(platform.path(), ["docs", "index", "--domain", "ops"])
        .success();
    insta::assert_snapshot!(
        "unindexed_domain_listing",
        common::mask_published_commits(&stdout(&env, &listed))
    );
}

/// A test machine whose `PATH` holds the built `instrukt`, as an installed
/// one would: a repository's hook sections call it by name.
fn hooked_machine() -> TestEnv {
    let mut env = TestEnv::new();
    let bin = std::path::Path::new(env!("CARGO_BIN_EXE_instrukt"))
        .parent()
        .unwrap();
    let path = format!("{}:{}", bin.display(), env.injected("PATH").unwrap());
    env.set_env("PATH", &path);
    env
}

/// Commits everything through the hooks and waits for the post-commit
/// run's sync.
fn commit_through_hook(env: &TestEnv, repo: &Repo, message: &str) {
    repo.commit(message);
    env.finished_hook_run(repo.path(), instrukt_core::repository::Hook::PostCommit);
}

/// A project whose index expands a policy that requires `sdlc`'s tests
/// policy, which another repository publishes; adopted with `init`.
fn selecting_project(env: &TestEnv) -> Repo {
    let repo = env.git_repo("demo");
    repo.write("README.md", "# demo\n");
    repo.commit("the developer's first commit");
    env.run(repo.path(), ["init"]).success();
    repo.write(
        "docs/project/policy/house.md",
        &snippet("policy", "House", "", &["sdlc/policy/tests"]),
    )
    .write(
        "docs/project/index.yaml",
        "expanded:\n  - project/demo/policy/house\n",
    );
    repo
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE10
#[test]
fn two_computers_commit_the_same_instructions() {
    // Two computers that publish different doctrine and hold different
    // configuration.
    let first = hooked_machine();
    let second = hooked_machine();
    env_doctrine(&first, "Test first.");
    env_doctrine(&second, "Ship daily.");
    second
        .run(second.home(), ["config", "init", "--global"])
        .success();
    let first_repo = selecting_project(&first);
    let second_repo = selecting_project(&second);

    // Each commits the same content through the hook.
    commit_through_hook(&first, &first_repo, "instructions");
    commit_through_hook(&second, &second_repo, "instructions");
    let blob = first_repo.git(["rev-parse", "HEAD:AGENTS.md"]);
    assert_eq!(second_repo.git(["rev-parse", "HEAD:AGENTS.md"]), blob);

    // A selected snippet that reads a configured value is a finding: the
    // commit carries the instructions as they were.
    first_repo.write(
        "docs/project/policy/house.md",
        &snippet("policy", "House", "", &["sdlc/policy/tests"])
            .replace("House: Rules.", "<!-- @config: project -->"),
    );
    commit_through_hook(&first, &first_repo, "a configured value");
    assert_eq!(first_repo.git(["rev-parse", "HEAD:AGENTS.md"]), blob);
    let status = first.run(first_repo.path(), ["hooks", "status"]).success();
    insta::assert_snapshot!(
        "status_naming_a_configured_value",
        stdout(&first, &status).replace(&first_repo.head(), "[SHA]")
    );
}

/// Publishes `sdlc` on `env`'s computer with its tests policy ruling `rule`.
fn env_doctrine(env: &TestEnv, rule: &str) {
    let repo = env.git_repo("engineering");
    repo.write("instrukt.yaml", &layer("engineering"))
        .write("docs/domain/sdlc/index.yaml", "description: Code work\n")
        .write(
            "docs/domain/sdlc/policy/tests.md",
            &snippet("policy", "Tests", "", &[]).replace("Tests: Rules.", rule),
        );
    repo.commit("sdlc");
    env.run(repo.path(), ["sync"]).success();
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE1
#[test]
fn an_agent_that_checks_out_the_project_starts_with_what_it_selected() {
    let env = hooked_machine();
    // Another repository publishes `general` with a distinctive sentence.
    let platform = env.git_repo("platform");
    platform
        .write("instrukt.yaml", &layer("platform"))
        .write(
            "docs/general/policy/care.md",
            &snippet("policy", "Care", "", &[])
                .replace("Care: Rules.", "Only the platform says this sentence."),
        )
        .write(
            "docs/general/index.yaml",
            "expanded:\n  - general/policy/care\n",
        );
    platform.commit("general");
    env.run(platform.path(), ["sync"]).success();

    // The project selects a snippet that requires it, lists another, has a
    // triggered procedure, and a nested master requiring a held snippet and
    // the unheld one.
    let repo = env.git_repo("demo");
    repo.write("README.md", "# demo\n");
    repo.commit("the developer's first commit");
    env.run(repo.path(), ["init"]).success();
    repo.write(
        "docs/project/policy/house.md",
        &snippet("policy", "House", "", &["general/policy/care"]),
    )
    .write(
        "docs/project/policy/style.md",
        &snippet("policy", "Style", "", &[]),
    )
    .write(
        "docs/project/procedure/release.md",
        &snippet(
            "procedure",
            "Release",
            "activation_trigger: When cutting a release.\n",
            &[],
        ),
    )
    .write(
        "docs/project/index.yaml",
        "expanded:\n  - project/demo/policy/house\nlisted:\n  - project/demo/policy/style\n",
    )
    .write(
        "tools/AGENTS.master.md",
        "## Tools\n\nOnly the tools.\n\n## Required reads\n\n- project/demo/policy/style\n- general/policy/care\n",
    );

    // The commit right after init carries both renderings.
    commit_through_hook(&env, &repo, "adopt instrukt");
    let root = repo.git(["show", "HEAD:AGENTS.md"]);
    let tools = repo.git(["show", "HEAD:tools/AGENTS.md"]);
    insta::assert_snapshot!("committed_root_instructions", root);
    insta::assert_snapshot!("committed_nested_instructions", tools);

    // A clone on a computer without instrukt holds the same files.
    let clone = env.path("clone");
    let empty_home = env.path("empty-home");
    std::fs::create_dir_all(&empty_home).unwrap();
    let cloned = std::process::Command::new("git")
        .args(["clone", "--quiet"])
        .arg(repo.path())
        .arg(&clone)
        .env_clear()
        .env("HOME", &empty_home)
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(cloned.success());
    assert_eq!(
        std::fs::read_to_string(clone.join("AGENTS.md"))
            .unwrap()
            .trim_end(),
        root
    );
    assert_eq!(
        std::fs::read_to_string(clone.join("tools/AGENTS.md"))
            .unwrap()
            .trim_end(),
        tools
    );

    // A checkout renders nothing: a branch whose committed file is no
    // rendering keeps it, in a linked worktree and in the primary checkout.
    env.run(repo.path(), ["sync"]).success();
    repo.git(["checkout", "--quiet", "-b", "hand-edited"]);
    repo.write("AGENTS.md", "# Edited by hand on this branch\n");
    repo.git(["add", "AGENTS.md"]);
    repo.git(["commit", "--quiet", "--no-verify", "-m", "hand edit"]);
    let on_branch = repo.git(["show", "hand-edited:AGENTS.md"]);
    repo.git(["checkout", "--quiet", "main"]);
    let linked = env.path("linked");
    repo.git([
        "worktree",
        "add",
        "--quiet",
        linked.to_str().unwrap(),
        "hand-edited",
    ]);
    assert_eq!(
        std::fs::read_to_string(linked.join("AGENTS.md"))
            .unwrap()
            .trim_end(),
        on_branch
    );
    assert_eq!(env.git(&linked, ["status", "--porcelain"]), "");
    repo.git(["worktree", "remove", "--force", linked.to_str().unwrap()]);
    repo.git(["checkout", "--quiet", "hand-edited"]);
    env.finished_hook_run(repo.path(), instrukt_core::repository::Hook::PostCheckout);
    assert_eq!(repo.read("AGENTS.md").trim_end(), on_branch);
    assert_eq!(repo.git(["status", "--porcelain"]), "");
}

// spec: project/instrukt/spec/feature/publication/agent-entrypoints UC-AE8
#[test]
fn a_companys_knowledge_reaches_every_agent_from_its_repository() {
    let env = hooked_machine();
    env.run(&env.path(""), ["config", "init", "--company", "acme"])
        .success();
    // The company's home is the checkout of its repository; its settings
    // document is there and ignored.
    let dir = env.instrukt_home().join("companies/acme");
    let relative = dir
        .strip_prefix(env.path(""))
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let home = env.git_repo(&relative);
    home.write(
        "AGENTS.master.md",
        "# Acme\n\nAcme builds bicycles to order.\n\n## Required reads\n\n- docs/people.md\n",
    )
    .write(
        "docs/people.md",
        "# People\n\nWho works at Acme.\n\n## Required reads\n\n- roles.md\n",
    )
    .write("docs/roles.md", "# Roles\n\nWhat each role at Acme does.\n");
    env.run(home.path(), ["hooks", "install"]).success();
    commit_through_hook(&env, &home, "the company's knowledge");
    let committed = home.git(["show", "HEAD:AGENTS.md"]);
    insta::assert_snapshot!("company_rendering", committed);
    assert_eq!(home.git(["status", "--porcelain"]), "");

    // A clone of the company's repository holds no settings document: it
    // renders the same bytes, and the check passes.
    let remote = env.bare_remote("acme");
    home.git(["push", "--quiet", remote.to_str().unwrap(), "main"]);
    let clone = env.clone_repo(&remote, "acme-clone");
    assert!(!clone.path().join("instrukt.yaml").exists());
    env.run(clone.path(), ["docs", "render", "--check"])
        .success();
    std::fs::remove_file(clone.path().join("AGENTS.md")).unwrap();
    env.run(clone.path(), ["docs", "render"]).success();
    // The home's file is the committed one: its status is clean.
    assert_eq!(clone.read("AGENTS.md"), home.read("AGENTS.md"));

    // The master gains a required read of a snippet: the commit is made, its
    // AGENTS.md is the one before, and both commands name the master and
    // the read.
    let head = home.git(["rev-parse", "HEAD"]);
    home.write(
        "AGENTS.master.md",
        "# Acme\n\nAcme builds bicycles to order.\n\n## Required reads\n\n- docs/people.md\n- general/policy/commits\n",
    );
    commit_through_hook(&env, &home, "doctrine in the master");
    assert_ne!(home.git(["rev-parse", "HEAD"]), head);
    assert_eq!(home.git(["show", "HEAD:AGENTS.md"]), committed);
    let validated = env
        .run(&env.path(""), ["config", "validate", "--company", "acme"])
        .failure();
    let checked = env
        .run(home.path(), ["docs", "render", "--check"])
        .failure();
    insta::assert_snapshot!(
        "company_rendering_names_the_read",
        format!(
            "config validate --company acme:\n{}docs render --check:\n{}{}",
            stdout(&env, &validated),
            stdout(&env, &checked),
            shown(&env, &checked.get_output().stderr)
        )
    );

    // A required read naming a symbolic link is never read: in the working
    // tree and in the index the commit is made from, the finding names it as
    // a link.
    home.write(
        "AGENTS.master.md",
        "# Acme\n\nAcme builds bicycles to order.\n\n## Required reads\n\n- docs/team.md\n",
    );
    std::os::unix::fs::symlink("roles.md", home.path().join("docs/team.md")).unwrap();
    let linked = env
        .run(home.path(), ["docs", "render", "--check"])
        .failure();
    commit_through_hook(&env, &home, "a linked document");
    let record = env.finished_hook_run(home.path(), instrukt_core::repository::Hook::PreCommit);
    let instrukt_core::repository::StagingOutcome::Unchanged { findings } = record.staging.unwrap()
    else {
        panic!("the commit staged a rendering of a linked read");
    };
    insta::assert_snapshot!(
        "company_rendering_names_a_link",
        format!(
            "working tree:\n{}{}index:\n{}\n",
            stdout(&env, &linked),
            shown(&env, &linked.get_output().stderr),
            findings.join("\n")
        )
    );
    assert_eq!(home.git(["show", "HEAD:AGENTS.md"]), committed);

    // Each of these leaves the home's rendering as it is, and the render
    // names why.
    let refused = |step: &str| {
        let before = home.read("AGENTS.md");
        let output = env.run(home.path(), ["docs", "render"]).failure();
        assert_eq!(home.read("AGENTS.md"), before);
        format!(
            "{step}:\n{}{}",
            stdout(&env, &output),
            shown(&env, &output.get_output().stderr)
        )
    };
    let mut refusals = Vec::new();

    // A read through a linked folder is never read, though the index holds
    // the file below it.
    home.write(
        "AGENTS.master.md",
        "# Acme\n\n## Required reads\n\n- docs/team/people.md\n",
    )
    .write("docs/team/people.md", "# Team\n");
    home.git(["add", "AGENTS.master.md", "docs/team/people.md"]);
    std::fs::remove_dir_all(home.path().join("docs/team")).unwrap();
    home.write("outside/people.md", "# Outside\n\nNot the company's.\n");
    std::os::unix::fs::symlink("../outside", home.path().join("docs/team")).unwrap();
    refusals.push(refused("a linked folder"));
    std::fs::remove_file(home.path().join("docs/team")).unwrap();
    home.git(["reset", "--quiet"]);

    // Nor through a linked `docs/`.
    home.write(
        "AGENTS.master.md",
        "# Acme\n\n## Required reads\n\n- docs/people.md\n",
    );
    std::fs::rename(home.path().join("docs"), home.path().join("shelf")).unwrap();
    std::os::unix::fs::symlink("shelf", home.path().join("docs")).unwrap();
    refusals.push(refused("a linked docs folder"));
    std::fs::remove_file(home.path().join("docs")).unwrap();
    std::fs::rename(home.path().join("shelf"), home.path().join("docs")).unwrap();

    // Nor when the index holds `docs` itself as the link.
    std::fs::rename(home.path().join("docs"), home.path().join("shelf")).unwrap();
    std::os::unix::fs::symlink("outside", home.path().join("docs")).unwrap();
    home.git(["rm", "--cached", "-r", "--quiet", "docs"]);
    home.git(["add", "docs"]);
    refusals.push(refused("a staged docs link"));
    std::fs::remove_file(home.path().join("docs")).unwrap();
    std::fs::rename(home.path().join("shelf"), home.path().join("docs")).unwrap();
    home.git(["reset", "--quiet"]);

    // A configured value among the required reads is a directive all the
    // same; configuration's validation names it too.
    home.write(
        "AGENTS.master.md",
        "# Acme\n\n## Required reads\n\n<!-- @config: company -->\n- docs/people.md\n",
    );
    refusals.push(refused("a configured value among the required reads"));
    let validated = env
        .run(&env.path(""), ["config", "validate", "--company", "acme"])
        .failure();
    refusals.push(format!(
        "config validate --company acme:\n{}",
        stdout(&env, &validated)
    ));

    // Documents requiring each other have no order.
    home.write(
        "AGENTS.master.md",
        "# Acme\n\n## Required reads\n\n- docs/a.md\n",
    )
    .write("docs/a.md", "# A\n\n## Required reads\n\n- b.md\n")
    .write("docs/b.md", "# B\n\n## Required reads\n\n- a.md\n");
    refusals.push(refused("a cycle"));

    // Every reference form in the text is held to the company's own
    // documents; a reference to one of them stays.
    home.write(
        "AGENTS.master.md",
        "# Acme\n\nSee @general/policy/commits for details.\nSee @general for details.\nRead [Outside](../secret.md).\nSee @docs/people.md for details.\n\n## See Also\n\n- general/policy/commits\n",
    );
    refusals.push(refused("references in the text"));
    insta::assert_snapshot!("company_rendering_refusals", refusals.join("\n"));

    // A bare `@word` mentions someone: it is prose, and the home renders.
    home.write(
        "AGENTS.master.md",
        "# Acme\n\nAsk @maurice about frames.\n\n## Required reads\n\n- docs/people.md\n",
    );
    let mentioned = env.run(home.path(), ["docs", "render"]).success();
    insta::assert_snapshot!(
        "company_rendering_keeps_a_mention",
        format!(
            "{}{}{}",
            stdout(&env, &mentioned),
            shown(&env, &mentioned.get_output().stderr),
            home.read("AGENTS.md")
        )
    );
}
