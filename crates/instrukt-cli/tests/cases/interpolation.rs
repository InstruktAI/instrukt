//! Read-time configuration references and explicit runtime caller context.
#![allow(clippy::unwrap_used, reason = "isolated CLI setup")]
use crate::common;
use crate::fixtures::documented;
use crate::fixtures::t3;
use common::TestEnv;
// spec: project/instrukt/spec/feature/configuration/interpolation UC-VI2
#[test]
fn configured_values_expand_without_rewriting_the_stored_layer() {
    let mut env = TestEnv::new();
    env.set_env("DESTINATION", "local");
    let repo = env.git_repo("demo");
    let source =
        "schema_version: 5\nproject: demo\ngithub: {repository: '{project}/${DESTINATION}'}\n";
    repo.write("instrukt.yaml", source);
    let out = env
        .run(repo.path(), ["config", "get", "github.repository"])
        .success();
    insta::assert_snapshot!(String::from_utf8_lossy(&out.get_output().stdout),@"demo/local");
    assert_eq!(repo.read("instrukt.yaml").as_bytes(), source.as_bytes());
}
// spec: project/instrukt/spec/feature/configuration/interpolation UC-VI3
#[test]
fn only_the_selected_adapters_native_caller_context_supplies_identity() {
    let mut env = TestEnv::new();
    let file = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, "schema_version: 5\nrunner: {runtime: t3}\n").unwrap();
    env.set_env("T3CODE_THREAD_ID", "native-thread:42");
    let cwd = env.path("");
    let out = env
        .run(&cwd, ["config", "get", "runtime.session_id"])
        .success();
    insta::assert_snapshot!(String::from_utf8_lossy(&out.get_output().stdout),@"native-thread:42");
    env.remove_env("T3CODE_THREAD_ID");
    let out = env
        .run(&cwd, ["config", "get", "runtime.session_id"])
        .failure();
    insta::assert_snapshot!(String::from_utf8_lossy(&out.get_output().stderr));
}

// spec: project/instrukt/spec/feature/configuration/interpolation UC-VI4
// spec: project/instrukt/spec/feature/configuration/layered-configuration UC-CL6
#[test]
fn runtime_action_directives_render_the_selected_global_entry_without_a_caller_or_execution() {
    let env = TestEnv::new();
    let fake = t3::FakeT3::create(&env);
    let global = env.instrukt_home().join("instrukt.yaml");
    std::fs::create_dir_all(global.parent().unwrap()).unwrap();
    std::fs::write(&global,"schema_version: 1\nrunner: {runtime: t3}\nruntimes:\n  t3:\n    dispatch: Delegate the supplied native assignment with shared isolation.\n    message_child: Continue the recorded native child.\n").unwrap();
    let body = "<!-- @config: runtime.dispatch -->\n\n<!-- @config: runtime.message_child -->\n";
    let repo = documented::project(
        &env,
        "demo",
        &[(
            "docs/domain/acme/procedure/delegate.md",
            documented::snippet(
                "procedure",
                "Delegate",
                "Delegate through the selected native action.",
                "",
                &[],
                body,
            ),
        )],
    );
    documented::sync(&env, &repo);
    repo.write("instrukt.yaml","schema_version: 1\nproject: demo\nruntimes:\n  t3:\n    dispatch: Do not use the machine operation.\n");
    env.run(repo.path(), ["config", "validate"]).failure();
    let read = env
        .run(repo.path(), ["docs", "get", "acme/procedure/delegate"])
        .success();
    let rendered = common::mask_published_commits(
        &String::from_utf8_lossy(&read.get_output().stdout).replace(
            env.path("").to_str().unwrap().trim_end_matches('/'),
            "[TMP]",
        ),
    );
    insta::assert_snapshot!("configured-native-actions", rendered);
    std::fs::write(&global,"schema_version: 1\nrunner: {runtime: t3}\nruntimes:\n  t3:\n    dispatch: Use the explicitly configured instrukt fallback for the missing native capability.\n").unwrap();
    let fallback = env
        .run(repo.path(), ["docs", "get", "acme/procedure/delegate"])
        .success();
    insta::assert_snapshot!(
        "configured-action-fallback",
        common::mask_published_commits(
            &String::from_utf8_lossy(&fallback.get_output().stdout).replace(
                env.path("").to_str().unwrap().trim_end_matches('/'),
                "[TMP]"
            )
        )
    );
    std::fs::write(&global, "schema_version: 1\n").unwrap();
    env.run(repo.path(), ["docs", "get", "acme/procedure/delegate"])
        .failure();
    assert_eq!(fake.threads().len(), 0);
    assert_eq!(fake.calls().len(), 0);
}
