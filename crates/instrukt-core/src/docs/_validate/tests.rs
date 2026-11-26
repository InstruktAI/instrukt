use std::path::Path;
use std::sync::Arc;

use super::*;
use crate::docs::SourceFile;
use crate::ids::{ProjectSlug, RoleName};
use crate::interfaces::{
    FlavorDeclarations, FlavorFinding, FlavorMembership, FlavorRoles, FlavorSchema, InterfaceError,
    TodoExistence,
};

struct Todos(Vec<&'static str>);

impl TodoExistence for Todos {
    fn todo_exists(&self, _: &ProjectSlug, slug: &TodoSlug) -> Result<bool, InterfaceError> {
        Ok(self.0.contains(&slug.as_str()))
    }
}

struct Roles {
    roles: Vec<&'static str>,
}

impl FlavorRoles for Roles {
    fn declared(&self, _: &Path) -> Result<FlavorMembership, InterfaceError> {
        Ok(FlavorMembership {
            roles: self
                .roles
                .iter()
                .filter_map(|r| RoleName::parse(r).ok())
                .collect(),
        })
    }
}

struct Schema(FlavorCheck);

impl FlavorSchema for Schema {
    fn check(&self, _: &Path) -> Result<FlavorCheck, InterfaceError> {
        Ok(self.0.clone())
    }
}

fn corpus_at(home: &Path, files: &[(&str, &str)]) -> Corpus {
    Corpus::from_sources(
        ProjectSlug::parse("demo").ok(),
        Some(&Home::at_user_home(home.to_path_buf())),
        files.iter().map(|(path, text)| SourceFile {
            path: (*path).to_owned(),
            text: (*text).to_owned(),
        }),
    )
}

fn corpus(files: &[(&str, &str)]) -> Corpus {
    corpus_at(Path::new("/nonexistent-home"), files)
}

fn run(corpus: &Corpus, wiring: &CoreWiring, agents: &[&str]) -> Vec<Finding> {
    run_at(Path::new("/nonexistent-repo"), corpus, wiring, agents)
}

fn run_at(root: &Path, corpus: &Corpus, wiring: &CoreWiring, agents: &[&str]) -> Vec<Finding> {
    let _ = agents;
    check(
        corpus,
        &CheckEnv {
            repo_root: root,
            wiring,
            schemas: &Schemas::default(),
        },
    )
    .unwrap()
}

fn codes(findings: &[Finding]) -> Vec<FindingCode> {
    findings.iter().map(|f| f.code).collect()
}

const SECTIONS: &str = "## Goal\n\n## Preconditions\n\n## Steps\n\n## Outputs\n\n## Recovery\n";

fn procedure(front: &str) -> String {
    format!("---\ndescription: d\n{front}---\n# P — Procedure\n\n{SECTIONS}")
}

const PENDING_BUILDER: &str =
    "role: builder\nactivation_trigger: t\ndelivered_by: [known, unknown]\n";

#[test]
fn with_todos_off_neither_existence_nor_roles_are_asked() {
    let c = corpus(&[("docs/project/procedure/p.md", &procedure(PENDING_BUILDER))]);
    assert!(run(&c, &CoreWiring::default(), &[]).is_empty());
}

#[test]
fn with_todos_on_unknown_slugs_and_undeclared_roles_are_findings() {
    let c = corpus(&[
        ("docs/project/procedure/p.md", &procedure(PENDING_BUILDER)),
        (
            "docs/general/procedure/company-assistance.md",
            &procedure("role: company-agent\n"),
        ),
    ]);
    let wiring = CoreWiring {
        todo_existence: Some(Arc::new(Todos(vec!["known"]))),
        flavor_roles: Some(Arc::new(Roles {
            roles: vec!["reviewer"],
        })),
        ..CoreWiring::default()
    };
    assert_eq!(
        codes(&run(&c, &wiring, &[])),
        [FindingCode::RoleUndeclared, FindingCode::PendingSlugUnknown]
    );
    let declared = CoreWiring {
        flavor_roles: Some(Arc::new(Roles {
            roles: vec!["builder"],
        })),
        ..CoreWiring::default()
    };
    assert!(run(&c, &declared, &[]).is_empty());
}

const ADR: &str = "---\ndescription: d\ndate: 2026-01-02\nnumber: 1\n{extra}---\n# A — ADR\n\n\
                       ## Context\n\n{body}## Decision\n\n## Consequences\n";

#[test]
fn adrs_carry_no_pending_material() {
    let adr = ADR.replace("{extra}", "delivered_by: [later]\n").replace(
        "{body}",
        "<!-- planned:later -->\nx\n<!-- /planned:later -->\n\n",
    );
    let clean = ADR.replace("{extra}", "").replace("{body}", "");
    let c = corpus(&[
        ("docs/project/adr/0001-a.md", &adr),
        (
            "docs/project/adr/0002-b.md",
            &clean.replace("number: 1", "number: 2"),
        ),
        ("docs/project/adr/0002-b.later.md", ""),
    ]);
    let found = run(&c, &CoreWiring::default(), &[]);
    let carriers: Vec<&Path> = found
        .iter()
        .filter(|f| f.code == FindingCode::AdrPendingCarrier)
        .map(|f| f.path.as_path())
        .collect();
    assert_eq!(
        carriers,
        [
            Path::new("docs/project/adr/0001-a.md"),
            Path::new("docs/project/adr/0001-a.md"),
            Path::new("docs/project/adr/0002-b.later.md"),
        ]
    );
}

#[test]
fn third_party_docs_author_only_a_description() {
    let c = corpus(&[(
        "docs/third-party/tool/usage.md",
        "---\ndescription: d\nid: x\nrole: r\ndelivered_by: [a]\n---\n# Tool\n",
    )]);
    let found = run(&c, &CoreWiring::default(), &[]);
    assert_eq!(codes(&found), [FindingCode::ThirdPartyFieldNotAllowed; 3]);
}

fn source_findings(entry: &str) -> Vec<FindingCode> {
    let doc = format!("---\ndescription: d\n---\n# Tool\n\n## Sources\n\n- {entry}\n");
    codes(&run(
        &corpus(&[("docs/third-party/tool/usage.md", &doc)]),
        &CoreWiring::default(),
        &[],
    ))
}

#[test]
fn a_source_written_as_an_autolink_is_a_web_link() {
    assert!(source_findings("<https://tool.example/docs>").is_empty());
}

#[test]
fn an_autolink_to_something_other_than_the_web_is_refused() {
    assert_eq!(
        source_findings("<mailto:someone@tool.example>"),
        [FindingCode::SourceInvalid]
    );
}

#[test]
fn a_source_that_is_not_a_link_is_refused() {
    assert_eq!(
        source_findings("somewhere on the web"),
        [FindingCode::SourceInvalid]
    );
}

#[test]
fn a_manifest_naming_a_bundle_cannot_also_name_what_it_carries() {
    let concept = "---\ndescription: d\n---\n# C — Concept\n\n## What\n\n## Why\n";
    let c = corpus(&[
        ("docs/domain/acme/concept/a.md", concept),
        (
            "docs/domain/acme/baseline.md",
            "@~/.instrukt/docs/acme/concept/a.md\n",
        ),
        (
            "docs/project/baseline.md",
            "@~/.instrukt/docs/acme/bundle.md\n@~/.instrukt/docs/acme/concept/a.md\n",
        ),
    ]);
    let found = run(&c, &CoreWiring::default(), &[]);
    assert_eq!(codes(&found), [FindingCode::BaselineCarried]);
    assert_eq!(found[0].path, Path::new("docs/project/baseline.md"));
}

const CONCEPT: &str = "---\ndescription: d\n---\n# C — Concept\n\n## What\n\n## Why\n";

fn local_manifest_findings(files: &[(&str, &str)]) -> Vec<(FindingCode, PathBuf)> {
    let mut all = vec![
        ("docs/domain/acme/concept/a.md", CONCEPT),
        (
            "docs/domain/acme/baseline.md",
            "@~/.instrukt/docs/acme/concept/a.md\n",
        ),
        (
            "docs/project/baseline.md",
            "@~/.instrukt/docs/outer/bundle.md\n@~/.instrukt/docs/acme/concept/a.md\n",
        ),
    ];
    all.extend_from_slice(files);
    run(&corpus(&all), &CoreWiring::default(), &[])
        .into_iter()
        .map(|f| (f.code, f.path))
        .collect()
}

#[test]
fn a_bundle_naming_another_bundle_carries_what_that_bundle_carries() {
    let carried = vec![(
        FindingCode::BaselineCarried,
        PathBuf::from("docs/project/baseline.md"),
    )];
    let nested = local_manifest_findings(&[(
        "docs/domain/outer/baseline.md",
        "@~/.instrukt/docs/acme/bundle.md\n",
    )]);
    assert_eq!(nested, carried);
}

#[test]
fn bundles_naming_each_other_end_the_expansion() {
    let c = corpus(&[
        ("docs/domain/acme/concept/a.md", CONCEPT),
        (
            "docs/domain/acme/baseline.md",
            "@~/.instrukt/docs/acme/concept/a.md\n@~/.instrukt/docs/outer/bundle.md\n",
        ),
        (
            "docs/domain/outer/baseline.md",
            "@~/.instrukt/docs/acme/bundle.md\n",
        ),
        (
            "docs/project/baseline.md",
            "@~/.instrukt/docs/outer/bundle.md\n",
        ),
    ]);
    // The cycle ends; acme's own entry is carried back to it through
    // outer's bundle, and nothing else is reported.
    let found: Vec<(FindingCode, PathBuf)> = run(&c, &CoreWiring::default(), &[])
        .into_iter()
        .map(|f| (f.code, f.path))
        .collect();
    assert_eq!(
        found,
        [(
            FindingCode::BaselineCarried,
            PathBuf::from("docs/domain/acme/baseline.md")
        )]
    );
}

#[cfg(unix)]
#[test]
fn an_unreadable_published_procedure_fails_the_run() {
    use std::os::unix::fs::PermissionsExt;
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".instrukt/docs/other/procedure");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("p.md");
    std::fs::write(&file, procedure("role: builder\nactivation_trigger: t\n")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::read_to_string(&file).is_ok() {
        return; // running with privileges that ignore file modes
    }
    let c = corpus_at(home.path(), &[]);
    let result = check(
        &c,
        &CheckEnv {
            repo_root: Path::new("/nonexistent-repo"),
            wiring: &CoreWiring::default(),
            schemas: &Schemas::default(),
        },
    );
    assert!(matches!(result, Err(DocsError::Read { path, .. }) if path == file));
}

fn repo_with_workflow() -> tempfile::TempDir {
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo.path().join("docs/domain/acme/workflow")).unwrap();
    std::fs::write(
        repo.path().join("docs/domain/acme/workflow/workflow.yaml"),
        "",
    )
    .unwrap();
    repo
}

fn schema(check: FlavorCheck) -> CoreWiring {
    CoreWiring {
        flavor_schema: Some(Arc::new(Schema(check))),
        ..CoreWiring::default()
    }
}

#[test]
fn workflow_schema_findings_are_reported_in_their_file() {
    let repo = repo_with_workflow();
    let wiring = schema(FlavorCheck::Invalid(vec![FlavorFinding {
        file: PathBuf::from("workflow.yaml"),
        location: "roles".into(),
        message: "is empty".into(),
    }]));
    let found = run_at(repo.path(), &corpus(&[]), &wiring, &[]);
    assert_eq!(codes(&found), [FindingCode::WorkflowSchemaInvalid]);
    assert_eq!(
        found[0].path,
        Path::new("docs/domain/acme/workflow/workflow.yaml")
    );
}

#[test]
fn a_valid_flavor_is_checked_against_the_procedures() {
    let repo = repo_with_workflow();
    let role = |r: &str| RoleName::parse(r).unwrap();
    let wiring = schema(FlavorCheck::Valid(FlavorDeclarations {
        roles: vec![role("worker"), role("lead")],
        orchestrators: vec![("workflow".into(), role("lead"))],
    }));
    let c = corpus(&[
        (
            "docs/domain/acme/procedure/build.md",
            &procedure("role: worker\nactivation_trigger: t\n"),
        ),
        (
            "docs/domain/acme/procedure/rogue.md",
            &procedure("role: rogue\nactivation_trigger: t\n"),
        ),
    ]);
    let found = run_at(repo.path(), &c, &wiring, &[]);
    let got: Vec<(FindingCode, &Path)> = found.iter().map(|f| (f.code, f.path.as_path())).collect();
    assert_eq!(
        got,
        [
            (
                FindingCode::OrchestratorWithoutProcedure,
                Path::new("docs/domain/acme/workflow/workflow.yaml")
            ),
            (
                FindingCode::RoleUndeclared,
                Path::new("docs/domain/acme/procedure/rogue.md")
            ),
        ]
    );
}

fn requires_run(project_modules: &str) -> Vec<Finding> {
    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".instrukt")).unwrap();
    std::fs::write(
        home.path().join(".instrukt/instrukt.yaml"),
        "schema_version: 1\nmodules:\n  todos: false\n",
    )
    .unwrap();
    std::fs::write(
        repo.path().join(CONFIG_FILE_NAME),
        format!("schema_version: 1\nproject: demo\nrequires: [todos]\n{project_modules}"),
    )
    .unwrap();
    let home = Home::at_user_home(home.path().to_path_buf());
    validate(&ValidationRequest {
        repo_root: repo.path(),
        home: &home,
        paths: &[],
        wiring: &CoreWiring::default(),
    })
    .unwrap()
    .findings
}

#[test]
fn requires_naming_a_module_that_is_off_is_a_configuration_finding() {
    let found = requires_run("");
    assert_eq!(codes(&found), [FindingCode::RequiresModuleOff]);
    assert_eq!(found[0].path, Path::new(CONFIG_FILE_NAME));
    assert!(requires_run("modules:\n  todos: true\n").is_empty());
}
