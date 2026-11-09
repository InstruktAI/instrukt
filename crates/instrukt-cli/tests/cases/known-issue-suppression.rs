#![allow(clippy::unwrap_used, reason = "test setup")]
use crate::common;
use common::TestEnv;

// spec: project/instrukt/spec/feature/work/known-issue-suppression UC-KS1
#[test]
fn retained_project_dispositions_are_inspectable_removable_and_fail_closed() {
    let env = TestEnv::new();
    let repo = env.git_repo("project");
    repo.write("instrukt.yaml", "schema_version: 1\nproject: project\n");
    env.run(
        repo.path(),
        [
            "bugs",
            "suppress",
            "known socket shutdown",
            "--reason",
            "expected at shutdown",
        ],
    )
    .success();
    let listed = env
        .run(repo.path(), ["bugs", "suppressions", "--json"])
        .success();
    let records: serde_json::Value = serde_json::from_slice(&listed.get_output().stdout).unwrap();
    assert_eq!(records.as_array().unwrap().len(), 1);
    let id = records[0]["id"].as_str().unwrap();
    let issues = instrukt_todos::work::suppressions::KnownIssues::new(
        &instrukt_core::home::Home::at_user_home(env.home().into()),
        &instrukt_core::ids::ProjectSlug::parse("project").unwrap(),
    );
    assert!(
        issues
            .disposition("known socket shutdown")
            .unwrap()
            .is_some()
    );
    assert!(issues.disposition("different failure").unwrap().is_none());
    env.run(repo.path(), ["bugs", "unsuppress", id]).success();
    assert!(
        issues
            .disposition("known socket shutdown")
            .unwrap()
            .is_none()
    );
    let other = env.git_repo("other");
    other.write("instrukt.yaml", "schema_version: 1\nproject: other\n");
    env.run(
        other.path(),
        [
            "bugs",
            "suppress",
            "known socket shutdown",
            "--reason",
            "other project",
        ],
    )
    .success();
    assert!(
        issues
            .disposition("known socket shutdown")
            .unwrap()
            .is_none()
    );
    let retained = env.instrukt_home().join("state/known-issues/project.json");
    std::fs::write(retained, "broken").unwrap();
    assert!(issues.disposition("known socket shutdown").is_err());
    env.run(repo.path(), ["bugs", "suppressions"]).failure();
}
