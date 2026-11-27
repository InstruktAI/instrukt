use super::*;

const LAYER: &str = "schema_version: 6\nproject: demo\n";
const INDEX: &str = "expanded:\n  - project/demo/policy/house\n";

/// A project policy whose rule is `rule`, requiring each of `reads`.
fn policy(rule: &str, reads: &[&str]) -> String {
    let mut text = "---\ndescription: House.\n---\n\n# House — Policy\n".to_owned();
    if !reads.is_empty() {
        text.push_str("\n## Required reads\n\n");
        for read in reads {
            text.push_str(&format!("- {read}\n"));
        }
    }
    text.push_str(&format!("\n## Rules\n\n{rule}\n"));
    text
}

/// A project tree: its layer and index, `house` as `house`, and `more`
/// files beside them.
fn plan(house: &str, more: &[(&str, &str)]) -> RenderPlan {
    let mut files = vec![
        (CONFIG_FILE_NAME, LAYER),
        ("docs/project/index.yaml", INDEX),
        ("docs/project/policy/house.md", house),
    ];
    files.extend_from_slice(more);
    rendered(Committed::of(&files))
}

fn rendered(tree: Committed) -> RenderPlan {
    render_tree(&tree, tree.file(OVERRIDE_FILE).is_some()).unwrap()
}

/// An earlier release's rendering, opening with the line it wrote.
fn earlier_rendering() -> String {
    format!("{EARLIER_GENERATED_LINE}\n# Old\n")
}

fn root(plan: &RenderPlan) -> &PlannedFile {
    plan.writes
        .iter()
        .find(|file| file.path == Path::new(AGENTS_FILE))
        .unwrap()
}

/// A unit of `source`, current when `current`, with `body`.
fn unit_of(source: &str, current: bool, body: &str) -> String {
    let prose = docs::split_frontmatter(source).1;
    let digest = if current {
        _units::source_digest(prose)
    } else {
        _units::source_digest("earlier prose")
    };
    format!("---\nsource_sha256: {digest}\nmodel: test-model\n---\n{body}")
}

/// A project tree whose house policy requires its style policy, each
/// with the unit `units` gives it.
fn with_units(house_unit: Option<String>, style_unit: Option<String>) -> Committed {
    let mut files: Vec<(&str, String)> = vec![
        (CONFIG_FILE_NAME, LAYER.to_owned()),
        ("docs/project/index.yaml", INDEX.to_owned()),
        ("docs/project/policy/house.md", house()),
        ("docs/project/policy/style.md", style()),
    ];
    if let Some(unit) = house_unit {
        files.push(("docs/_compressed/project/policy/house.md", unit));
    }
    if let Some(unit) = style_unit {
        files.push(("docs/_compressed/project/policy/style.md", unit));
    }
    let borrowed: Vec<(&str, &str)> = files.iter().map(|(p, t)| (*p, t.as_str())).collect();
    Committed::of(&borrowed)
}

fn house() -> String {
    policy("Be kind.", &["project/demo/policy/style"])
}

fn style() -> String {
    policy("Write plainly.", &[])
}

#[test]
fn a_current_unit_is_held_framed_under_its_id_after_one_legend() {
    let tree = with_units(
        Some(unit_of(
            &house(),
            true,
            "- MUST be kind -- people work here\n",
        )),
        Some(unit_of(
            &style(),
            true,
            "- PREFER plain words -- they carry\n",
        )),
    );
    let plan = rendered(tree);
    assert_eq!(plan.findings, []);
    insta::assert_snapshot!(root(&plan).text);
}

#[test]
fn a_stale_unit_holds_its_source_in_prose_and_the_others_compressed() {
    let tree = with_units(
        Some(unit_of(
            &house(),
            true,
            "- MUST be kind -- people work here\n",
        )),
        Some(unit_of(&style(), false, "- PREFER plain words\n")),
    );
    let plan = rendered(tree);
    assert_eq!(plan.findings, []);
    insta::assert_snapshot!(root(&plan).text);
}

#[test]
fn a_unit_with_a_finding_leaves_the_root_file_as_it_is() {
    let tree = with_units(
        Some(unit_of(&house(), true, "<!-- @exec: true -->\n")),
        None,
    );
    let plan = rendered(tree);
    assert_eq!(plan.writes, []);
    assert!(matches!(
        plan.findings.as_slice(),
        [RenderFinding {
            kind: RenderFindingKind::Unit { .. },
            ..
        }]
    ));
}

#[test]
fn the_commit_the_index_and_the_working_tree_render_the_same_units() {
    let dir = tempfile::tempdir().unwrap();
    let root_dir = dir.path();
    let files = [
        (CONFIG_FILE_NAME, LAYER.to_owned()),
        ("docs/project/index.yaml", INDEX.to_owned()),
        ("docs/project/policy/house.md", house()),
        ("docs/project/policy/style.md", style()),
        (
            "docs/_compressed/project/policy/house.md",
            unit_of(&house(), true, "- MUST be kind -- people work here\n"),
        ),
    ];
    for (path, text) in &files {
        let target = root_dir.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, text).unwrap();
    }
    crate::git::run(root_dir, ["init", "-q", "--initial-branch=main"]).unwrap();
    crate::git::run(root_dir, ["add", "--all"]).unwrap();
    crate::git::run(
        root_dir,
        [
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "commit",
            "-q",
            "-m",
            "docs",
        ],
    )
    .unwrap();
    let head = crate::ids::CommitSha::parse(
        crate::git::run(root_dir, ["rev-parse", "HEAD"])
            .unwrap()
            .trim(),
    )
    .unwrap();
    let text = |tree: Committed| root(&rendered(tree)).text.clone();
    let committed = text(Committed::read(root_dir, &head).unwrap());
    assert_eq!(
        text(Committed::read_index(root_dir, None).unwrap()),
        committed
    );
    assert_eq!(text(Committed::working_tree(root_dir).unwrap()), committed);
    let expected = text(with_units(
        Some(unit_of(
            &house(),
            true,
            "- MUST be kind -- people work here\n",
        )),
        None,
    ));
    assert_eq!(committed, expected);
}

/// How the global files of a home whose published `general` position
/// holds `published` and `units` (both relative to that position) hold
/// `general/policy/care`, which the publishing project does not own.
fn foreign_holding(published: &str, unit: Option<&str>, link_units: bool) -> UnitText {
    let recorded = _units::source_digest(docs::split_frontmatter(published).1);
    foreign_holding_recorded(published, unit, link_units, &recorded)
}

/// As [`foreign_holding`], with the global index recording `recorded` as
/// the source digest of `general/policy/care`.
fn foreign_holding_recorded(
    published: &str,
    unit: Option<&str>,
    link_units: bool,
    recorded: &str,
) -> UnitText {
    let dir = tempfile::tempdir().unwrap();
    let home = Home::at_user_home(dir.path().to_path_buf());
    let position = home.docs_dir().join("general");
    std::fs::create_dir_all(position.join("policy")).unwrap();
    std::fs::write(position.join("policy/care.md"), published).unwrap();
    if link_units {
        let elsewhere = dir.path().join("elsewhere");
        std::fs::create_dir_all(elsewhere.join("policy")).unwrap();
        if let Some(unit) = unit {
            std::fs::write(elsewhere.join("policy/care.md"), unit).unwrap();
        }
        std::os::unix::fs::symlink(&elsewhere, position.join("_compressed")).unwrap();
    } else if let Some(unit) = unit {
        std::fs::create_dir_all(position.join("_compressed/policy")).unwrap();
        std::fs::write(position.join("_compressed/policy/care.md"), unit).unwrap();
    }
    let project = ProjectSlug::parse("demo").unwrap();
    let tree = Committed::of(&[]);
    let corpus = Corpus::from_sources(Some(project.clone()), None, Vec::new());
    let owned = BTreeSet::new();
    let indexes = BTreeMap::new();
    let values = Values::committed();
    let index: GlobalIndex = serde_yaml_ng::from_str(&format!(
            "schema_version: 3\nsnippets:\n- id: general/policy/care\n  description: Care.\n  type: policy\n  scope: global\n  domain: general\n  path: general/policy/care.md\n  source_project: other\n  content_sha256: {recorded}\n"
        ))
        .unwrap();
    let source = Published {
        home: &home,
        project: &project,
        tree: &tree,
        corpus: &corpus,
        owned: &owned,
        indexes: &indexes,
        values: &values,
        units: UnitRendering::Compressed,
        index: &index,
    };
    source
        .unit(&SnippetId::parse("general/policy/care").unwrap())
        .unwrap()
}

const CARE: &str = "---\ndescription: Care.\n---\n\n# Care — Policy\n\nTake care.\n";

#[test]
fn a_current_foreign_unit_is_held_framed_under_its_id() {
    let unit = unit_of(CARE, true, "- MUST take care -- it lasts\n");
    let UnitText::Held(text) = foreign_holding(CARE, Some(&unit), false) else {
        unreachable!("a current foreign unit is held");
    };
    insta::assert_snapshot!(text);
}

#[test]
fn a_foreign_unit_is_current_by_the_digest_the_index_records_for_its_source() {
    let recorded = _units::source_digest("the prose as its owner published it");
    let declaring = |digest: &str| {
        format!("---\nsource_sha256: {digest}\nmodel: m\n---\n- MUST take care -- it lasts\n")
    };
    assert!(matches!(
        foreign_holding_recorded(CARE, Some(&declaring(&recorded)), false, &recorded),
        UnitText::Held(_)
    ));
    let prose = _units::source_digest(docs::split_frontmatter(CARE).1);
    assert_eq!(
        foreign_holding_recorded(CARE, Some(&declaring(&prose)), false, &recorded),
        UnitText::Prose
    );
}

#[test]
fn a_foreign_unit_behind_a_link_holds_its_source_in_prose_and_is_reported() {
    let unit = unit_of(CARE, true, "- MUST take care\n");
    assert!(matches!(
        foreign_holding(CARE, Some(&unit), true),
        UnitText::Fallback(findings)
            if matches!(findings.as_slice(), [UnitFinding { kind: UnitFindingKind::Link, .. }])
    ));
}

#[test]
fn an_unreadable_foreign_unit_holds_its_source_in_prose_and_is_reported() {
    assert!(matches!(
        foreign_holding(CARE, Some("no frontmatter\n"), false),
        UnitText::Fallback(findings)
            if matches!(findings.as_slice(), [UnitFinding { kind: UnitFindingKind::Unreadable { .. }, .. }])
    ));
}

#[test]
fn an_absent_foreign_unit_is_missing_and_holds_its_source_in_prose_unreported() {
    assert_eq!(foreign_holding(CARE, None, false), UnitText::Prose);
}

#[test]
fn a_unit_keeping_a_reference_for_an_agent_program_holds_its_source_in_prose() {
    let care = "---\ndescription: Care.\n---\n\n# Care — Policy\n\n@docs/general/policy/other.md\n";
    let unit = unit_of(care, true, "@docs/general/policy/other.md\n");
    assert!(matches!(
        foreign_holding(care, Some(&unit), false),
        UnitText::Fallback(findings)
            if matches!(findings.as_slice(), [UnitFinding { kind: UnitFindingKind::RenderingRule { .. }, .. }])
    ));
}

#[test]
fn a_rendering_is_recognised_by_either_generated_line_exactly() {
    for line in [GENERATED_LINE, EARLIER_GENERATED_LINE] {
        assert!(is_rendering(format!("{line}\n# Body\n").as_bytes()));
        assert!(is_rendering(format!("{line}\r\n# Body\r\n").as_bytes()));
        assert!(is_rendering(line.as_bytes()));
    }
    let near_match = GENERATED_LINE.replacen("instrukt", "instrukt-example", 1);
    let one_character_off = GENERATED_LINE.replacen("index", "indeX", 1);
    for other in [
        near_match.as_str(),
        "<!-- Generated by instrukt-example -->",
        "<!-- Generated from docs/index.yaml by the company publisher -->",
        one_character_off.as_str(),
        "# Instructions a person wrote",
        "",
    ] {
        assert!(
            !is_rendering(format!("{other}\n# Body\n").as_bytes()),
            "{other}"
        );
    }
    assert!(!is_rendering(b""));
}

#[test]
fn an_unheld_read_is_exactly_its_reference_line() {
    let plan = plan(&policy("Be kind.", &["sdlc/policy/tests"]), &[]);
    assert_eq!(plan.findings, []);
    insta::assert_snapshot!(root(&plan).text);
}

#[test]
fn a_held_snippet_is_inlined() {
    let plan = plan(&policy("Every change keeps the house kind.", &[]), &[]);
    let written: Vec<&Path> = plan.writes.iter().map(|file| file.path.as_path()).collect();
    assert_eq!(written, [Path::new(AGENTS_FILE)]);
    assert!(is_rendering(root(&plan).text.as_bytes()));
    insta::assert_snapshot!(root(&plan).text);
}

#[test]
fn a_required_read_keeps_its_place_whether_held_or_named() {
    let care = "---\ndescription: Care.\n---\n\n# Care — Policy\n\n## Rules\n\nTake care.\n";
    let plan = plan(
        &policy("Be kind.", &["general", "sdlc", "sdlc/policy/tests"]),
        &[
            (
                "docs/general/index.yaml",
                "expanded:\n  - general/policy/care\n",
            ),
            ("docs/general/policy/care.md", care),
        ],
    );
    assert_eq!(plan.findings, []);
    insta::assert_snapshot!(root(&plan).text);
}

/// The position headings of a rendering, in order.
fn headings(text: &str) -> Vec<&str> {
    text.lines().filter(|line| line.starts_with("# ")).collect()
}

#[test]
fn a_held_read_and_a_named_one_open_the_same_headings() {
    let tests = "---\ndescription: Tests.\n---\n\n# Tests — Policy\n\n## Rules\n\nTest first.\n";
    let sdlc = [
        (
            "docs/domain/sdlc/index.yaml",
            "description: Code work\nexpanded:\n  - sdlc/policy/tests\n",
        ),
        ("docs/domain/sdlc/policy/tests.md", tests),
    ];
    for read in ["sdlc/policy/tests", "sdlc"] {
        let house = policy("Be kind.", &[read]);
        let held = plan(&house, &sdlc);
        let named = plan(&house, &[]);
        assert_eq!(held.findings, []);
        assert_eq!(named.findings, []);
        assert_eq!(
            headings(&root(&held).text),
            headings(&root(&named).text),
            "reading `{read}`"
        );
    }
}

#[test]
fn a_root_file_without_the_line_is_written_over_without_a_finding() {
    let mine = "# My own instructions\n";
    let plan = plan(&policy("Be kind.", &[]), &[(AGENTS_FILE, mine)]);
    assert_eq!(root(&plan).current.as_deref(), Some(mine.as_bytes()));
    assert!(root(&plan).differs());
    assert_eq!(plan.findings, []);
}

#[test]
fn an_index_selecting_nothing_renders_the_generated_line_alone() {
    let plan = rendered(Committed::of(&[
        (CONFIG_FILE_NAME, LAYER),
        ("docs/project/index.yaml", ""),
    ]));
    assert_eq!(root(&plan).text, format!("{GENERATED_LINE}\n"));
}

#[test]
fn a_project_layer_declaring_company_repository_renders_the_company_index() {
    let plan = rendered(Committed::of(&[
        (
            CONFIG_FILE_NAME,
            "schema_version: 6\nproject: company-acme\ncompany_repository: true\n",
        ),
        ("docs/index.yaml", "expanded: []\nlisted: []\n"),
    ]));
    assert_eq!(plan.findings, []);
    assert_eq!(plan.notices, []);
    let written: Vec<&Path> = plan.writes.iter().map(|file| file.path.as_path()).collect();
    assert_eq!(written, [Path::new(AGENTS_FILE)]);
}

#[test]
fn any_other_project_layer_stays_a_project_whatever_docs_holds() {
    let plan = rendered(Committed::of(&[
        (CONFIG_FILE_NAME, LAYER),
        ("docs/index.yaml", "expanded: []\nlisted: []\n"),
    ]));
    assert_eq!(plan.writes, []);
    assert_eq!(plan.notices, [project_without_index()]);
}

#[test]
fn every_rendering_below_the_root_is_removed_and_nothing_else() {
    let current = format!("{GENERATED_LINE}\n# Old\n");
    let near_match = format!(
        "{}\n# Theirs\n",
        GENERATED_LINE.replacen("instrukt", "instrukt-example", 1)
    );
    let plan = plan(
        &policy("Be kind.", &[]),
        &[
            ("old/AGENTS.md", &current),
            ("older/AGENTS.md", &earlier_rendering()),
            ("theirs/AGENTS.md", &near_match),
            ("mine/AGENTS.md", "# Mine\n"),
            ("tools/AGENTS.master.md", "## Tools\n\nThe tools.\n"),
        ],
    );
    assert_eq!(
        plan.removals,
        [
            PathBuf::from("old/AGENTS.md"),
            PathBuf::from("older/AGENTS.md")
        ]
    );
    assert_eq!(
        plan.writes
            .iter()
            .map(|file| &file.path)
            .collect::<Vec<_>>(),
        [Path::new(AGENTS_FILE)]
    );
    assert_eq!(plan.findings, []);
}

#[test]
fn a_root_rendering_is_removed_only_without_an_index() {
    let without = rendered(Committed::of(&[
        (CONFIG_FILE_NAME, LAYER),
        ("AGENTS.master.md", "## Purpose\n\nThe demo project.\n"),
        (AGENTS_FILE, &earlier_rendering()),
    ]));
    assert_eq!(without.removals, [PathBuf::from(AGENTS_FILE)]);
    assert_eq!(without.writes, []);
    assert_eq!(without.notices, [project_without_index()]);

    // A present index whose rendering reports a finding: the root file is
    // neither written nor removed.
    let found = plan(
        &policy("<!-- @config: project -->", &[]),
        &[(AGENTS_FILE, &earlier_rendering())],
    );
    assert_eq!(found.writes, []);
    assert_eq!(found.removals, Vec::<PathBuf>::new());
    assert_eq!(found.findings.len(), 1);
}

#[test]
fn a_linked_index_is_a_finding_naming_it_and_the_root_file_stays() {
    let tree = Committed::of(&[
        (CONFIG_FILE_NAME, LAYER),
        (AGENTS_FILE, &earlier_rendering()),
    ])
    .with_link("docs/project/index.yaml");
    let plan = rendered(tree);
    assert_eq!(plan.writes, []);
    assert_eq!(plan.removals, Vec::<PathBuf>::new());
    assert_eq!(
        plan.findings,
        [RenderFinding {
            path: PathBuf::from(AGENTS_FILE),
            kind: RenderFindingKind::LinkedIndex {
                index: "docs/project/index.yaml".to_owned(),
            },
        }]
    );
    let below = rendered(
        Committed::of(&[
            (CONFIG_FILE_NAME, LAYER),
            (AGENTS_FILE, &earlier_rendering()),
        ])
        .with_link("docs"),
    );
    assert_eq!(below.findings, plan.findings);
    assert_eq!(below.removals, Vec::<PathBuf>::new());
}

#[test]
fn an_override_beside_a_planned_root_is_named_and_below_the_root_is_not() {
    let plan = plan(
        &policy("Be kind.", &[]),
        &[
            (OVERRIDE_FILE, "# Override\n"),
            ("tools/AGENTS.override.md", "# Tools override\n"),
        ],
    );
    assert_eq!(
        plan.findings,
        [RenderFinding {
            path: PathBuf::from(OVERRIDE_FILE),
            kind: RenderFindingKind::OverrideBesideRoot,
        }]
    );
}

#[test]
fn a_configured_value_in_a_document_held_in_full_is_a_finding_naming_it() {
    let plan = plan(&policy("<!-- @config: project -->", &[]), &[]);
    assert_eq!(
        plan.findings,
        [RenderFinding {
            path: PathBuf::from(AGENTS_FILE),
            kind: RenderFindingKind::ConfigInCommittedRendering {
                document: "docs/project/policy/house.md".to_owned(),
            },
        }]
    );
    assert_eq!(plan.writes, []);
}

#[test]
fn a_visible_reference_is_kept_and_one_in_code_is_none() {
    assert_eq!(
        kept_references("Read it (@notes/house.md) first.\n"),
        ["@notes/house.md"]
    );
    assert_eq!(
        kept_references("```\nRead it (@notes/house.md) first.\n```\n"),
        Vec::<String>::new()
    );
}

#[test]
fn finishing_drops_lock_sentinels_and_collapses_blank_runs() {
    let sentinel = "<!-- LOCKED_FILE: keep -->";
    let text = format!("{sentinel}\n\n\n# A\n\n\n\nb\n\n");
    assert_eq!(finish(&text), finish("# A\n\nb"));
    assert_eq!(finish("# A\n\nb").lines().count(), 3);
}

#[test]
fn a_domain_line_without_a_description_is_its_command() {
    let domain = DomainName::parse("sdlc").unwrap();
    assert_eq!(domain_line(&domain, Some(" ")), domain_line(&domain, None));
    assert_ne!(
        domain_line(&domain, Some("Code work")),
        domain_line(&domain, None)
    );
}
