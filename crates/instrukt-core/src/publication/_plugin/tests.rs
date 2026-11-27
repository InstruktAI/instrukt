use serde_json::Value as Json;

use super::*;

const INDEX: &str = super::_company::COMPANY_INDEX;

fn key() -> ProjectSlug {
    ProjectSlug::parse("acme").unwrap()
}

fn name() -> CompanyName {
    CompanyName::parse("Acme").unwrap()
}

fn acme() -> CompanyIdentity {
    CompanyIdentity {
        key: key(),
        name: name(),
    }
}

/// The findings of rendering `tree`: the root's, which leave it no company
/// repository to render, else the rendering's under Acme's identity.
fn findings(tree: &Committed) -> Vec<PluginFinding> {
    if let Err(
        finding @ (PluginFinding::ProjectRepository
        | PluginFinding::NoIndex
        | PluginFinding::UnreadableRoot { .. }),
    ) = identity(tree)
    {
        return vec![finding];
    }
    match render(tree, &acme()).unwrap() {
        Rendered::Findings(findings) => findings,
        Rendered::Files(files) => panic!("rendered {:?}", files.keys()),
    }
}

#[test]
fn a_path_names_its_file_from_the_folder_of_the_file_that_holds_it() {
    assert_eq!(
        relative("SKILL.md", "references/teams/a.md"),
        "references/teams/a.md"
    );
    assert_eq!(relative("SKILL.md", "SKILL.md"), "SKILL.md");
    assert_eq!(
        relative("references/teams/a.md", "references/roles.md"),
        "../roles.md"
    );
    assert_eq!(
        relative("references/teams/a.md", "SKILL.md"),
        "../../SKILL.md"
    );
    assert_eq!(
        relative("references/teams/a.md", "references/teams/b.md"),
        "b.md"
    );
}

#[test]
fn a_projects_repository_renders_no_plugin() {
    let tree = Committed::of(&[
        (CONFIG_FILE_NAME, "schema_version: 6\nproject: demo\n"),
        (INDEX, "expanded: [company.md]\n"),
        ("docs/company.md", "# Company\n"),
    ]);
    assert_eq!(findings(&tree), [PluginFinding::ProjectRepository]);
}

#[test]
fn a_project_layer_declaring_company_repository_renders_the_plugin() {
    let tree = Committed::of(&[
        (
            CONFIG_FILE_NAME,
            "schema_version: 6\nproject: company-acme\ncompany_repository: true\n",
        ),
        (INDEX, "expanded: [company.md]\n"),
        (
            "docs/company.md",
            "---\ndescription: Who Acme is.\n---\n# Company\n",
        ),
    ]);
    assert!(matches!(
        render(&tree, &acme()).unwrap(),
        Rendered::Files(_)
    ));
}

#[test]
fn a_repository_without_a_company_index_renders_no_plugin() {
    let tree = Committed::of(&[("docs/company.md", "# Company\n")]);
    assert_eq!(findings(&tree), [PluginFinding::NoIndex]);
}

#[test]
fn frontmatter_that_is_no_yaml_mapping_on_a_carried_document_is_a_finding() {
    let tree = Committed::of(&[
        (INDEX, "expanded: [company.md]\n"),
        ("docs/company.md", "---\n- a list\n---\n\n# Company\n"),
    ]);
    assert!(matches!(
        findings(&tree).as_slice(),
        [PluginFinding::FrontmatterInvalid { document, .. }] if document == "docs/company.md"
    ));
}

#[test]
fn an_empty_or_null_frontmatter_block_on_a_carried_document_is_a_finding() {
    for block in ["---\n---\n", "---\nnull\n---\n"] {
        let tree = Committed::of(&[
            (INDEX, "expanded: [company.md]\n"),
            ("docs/company.md", &format!("{block}\n# Company\n")),
        ]);
        assert!(
            matches!(
                findings(&tree).as_slice(),
                [PluginFinding::FrontmatterInvalid { document, .. }] if document == "docs/company.md"
            ),
            "{block:?}"
        );
    }
}

#[test]
fn a_visibility_other_than_internal_or_public_is_a_finding_naming_it() {
    let tree = Committed::of(&[
        (INDEX, "expanded: [company.md]\n"),
        (
            "docs/company.md",
            "---\nvisibility: secret\n---\n\n# Company\n",
        ),
    ]);
    assert_eq!(
        findings(&tree),
        [PluginFinding::UnknownVisibility {
            document: "docs/company.md".to_owned(),
            value: "secret".to_owned(),
        }]
    );
}

#[test]
fn a_reference_to_a_document_the_plugin_does_not_carry_is_a_finding() {
    let tree = Committed::of(&[
        (INDEX, "expanded: [company.md]\n"),
        ("docs/company.md", "# Company\n\nSee [drafts](drafts.md).\n"),
        ("docs/drafts.md", "# Drafts\n"),
    ]);
    assert_eq!(
        findings(&tree),
        [PluginFinding::Uncarried {
            document: "docs/company.md".to_owned(),
            reference: "drafts.md".to_owned(),
        }]
    );
}

#[test]
fn a_reference_to_a_file_that_is_no_document_is_a_finding() {
    let tree = Committed::of(&[
        (INDEX, "expanded: [company.md]\n"),
        ("docs/company.md", "# Company\n\nSee ![the map](map.png).\n"),
        ("docs/map.png", "PNG"),
    ]);
    assert_eq!(
        findings(&tree),
        [PluginFinding::NotADocument {
            document: "docs/company.md".to_owned(),
            reference: "map.png".to_owned(),
        }]
    );
}

const CATALOG_TEXT: &str = "{\"name\": \"acme-tools\", \"owner\": {\"name\": \"Acme\"}, \"plugins\": [{\"name\": \"other\", \"source\": \"./other\", \"extra\": 1}]}";

/// A company's repository holding `catalog`, one expanded document and
/// `more`.
fn repository(catalog: Option<&str>, more: &[(&str, &str)]) -> Committed {
    let mut files = vec![
        (INDEX, "expanded: [company.md]\n"),
        ("docs/company.md", "# Company\n\nWe make things.\n"),
    ];
    files.extend(catalog.map(|text| (CATALOG, text)));
    files.extend_from_slice(more);
    Committed::of(&files)
}

/// The catalog with the plugin's entry set at `version`, or the
/// finding that it cannot be read.
fn catalog(
    tree: &Committed,
    version: &str,
) -> Result<Result<Option<PlannedFile>, PluginFinding>, PublicationError> {
    let version = PluginVersion::parse(version).unwrap();
    match read_catalog(tree) {
        Ok(Some((document, bytes))) => {
            catalog_entry(document, Some(bytes), &key(), &name(), &version).map(Ok)
        }
        Ok(None) => catalog_entry(
            new_catalog(&key(), &name()),
            None,
            &key(),
            &name(),
            &version,
        )
        .map(Ok),
        Err(finding) => Ok(Err(finding)),
    }
}

/// What rendering from `tree` changes, which must be knowable.
fn drift_of(tree: &Committed) -> PluginDrift {
    planned(tree, &acme()).unwrap().drift.unwrap()
}

/// `tree` with `plan`'s writes applied and its removals made.
fn applied(tree: &[(String, String)], plan: &PluginDrift) -> Vec<(String, String)> {
    let mut files: BTreeMap<String, String> = tree.iter().cloned().collect();
    for path in &plan.removals {
        files.remove(path.to_str().unwrap());
    }
    for file in &plan.writes {
        files.insert(file.path.to_str().unwrap().to_owned(), file.text.clone());
    }
    files.into_iter().collect()
}

fn view(files: &[(String, String)]) -> Committed {
    let borrowed: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, text)| (path.as_str(), text.as_str()))
        .collect();
    Committed::of(&borrowed)
}

fn files_of(tree: &[(&str, &str)]) -> Vec<(String, String)> {
    tree.iter()
        .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
        .collect()
}

const COMPANY: &str = "# Company\n\nWe make things. Roles: [roles](roles.md).\n";
const ROLES: &str = "---\ndescription: Who does what.\n---\n\n# Roles\n\nAda builds.\n";

/// A unit of `document`, current when `current`, with `body`.
fn unit_of(document: &str, current: bool, body: &str) -> String {
    let prose = docs::split_frontmatter(document).1;
    let digest = super::super::_units::source_digest(if current { prose } else { "older" });
    format!("---\nsource_sha256: {digest}\nmodel: m\n---\n{body}")
}

/// A company repository expanding `company.md`, listing `roles.md`,
/// with `company.md`'s unit `unit`.
fn with_unit(unit: Option<&str>) -> Vec<(String, String)> {
    let mut files = vec![
        (INDEX, "expanded: [company.md]\nlisted: [roles.md]\n"),
        ("docs/company.md", COMPANY),
        ("docs/roles.md", ROLES),
        (CATALOG, CATALOG_TEXT),
    ];
    files.extend(unit.map(|unit| ("docs/_compressed/company.md", unit)));
    files_of(&files)
}

fn rendered_files(tree: &Committed) -> BTreeMap<String, String> {
    match render(tree, &acme()).unwrap() {
        Rendered::Files(files) => files,
        Rendered::Findings(findings) => panic!("findings {findings:?}"),
    }
}

const SKILL: &str = "skills/acme/SKILL.md";
const CARRIED: &str = "skills/acme/references/company.md";

#[test]
fn a_document_held_as_its_unit_is_framed_with_the_file_carrying_its_prose() {
    let unit = unit_of(
        COMPANY,
        true,
        "- we make things; roles: [roles](roles.md)\n",
    );
    let files = rendered_files(&view(&with_unit(Some(&unit))));
    insta::assert_snapshot!("carried_prose", files[CARRIED]);
    insta::assert_snapshot!(files[SKILL]);
}

#[test]
fn a_stale_unit_holds_its_document_in_prose_in_the_main_file_alone() {
    let unit = unit_of(COMPANY, false, "- we make things\n");
    let files = rendered_files(&view(&with_unit(Some(&unit))));
    assert!(!files.contains_key(CARRIED));
    insta::assert_snapshot!(files[SKILL]);
}

#[test]
fn the_version_rises_when_only_a_unit_changes() {
    let first = with_unit(Some(&unit_of(COMPANY, true, "- we make things\n")));
    let planned = planned(&view(&first), &acme()).unwrap();
    let version = planned.drift.clone().unwrap().version;
    let mut changed = applied_plan(&first, &planned);
    for (path, text) in &mut changed {
        if path == "docs/_compressed/company.md" {
            *text = unit_of(COMPANY, true, "- we make many things\n");
        }
    }
    assert_eq!(drift_of(&view(&changed)).version, version.raised());
}

#[test]
fn a_version_is_three_numbers_of_ascii_digits() {
    let parsed = PluginVersion::parse("1.20.3").unwrap();
    assert_eq!(parsed.to_string(), "1.20.3");
    assert_eq!(parsed.raised().to_string(), "1.20.4");
    for text in [
        "1.2", "1.2.3.4", "+1.0.0", "1.-2.3", "1..3", "v1.2.3", "1.2.٣", "",
    ] {
        assert_eq!(PluginVersion::parse(text), None, "{text:?}");
    }
}

#[test]
fn the_last_number_rises_by_one_without_bound_and_the_first_two_stay_as_written() {
    let raised = |text: &str| PluginVersion::parse(text).unwrap().raised().to_string();
    assert_eq!(
        raised("2.7.18446744073709551615"),
        "2.7.18446744073709551616"
    );
    assert_eq!(raised("2.7.99"), "2.7.100");
    assert_eq!(raised("01.007.009"), "01.007.010");
    assert_eq!(
        raised("2.7.99999999999999999999999"),
        "2.7.100000000000000000000000"
    );
}

/// A company's repository rendered once, and the version it was
/// rendered at.
fn rendered_once() -> (Vec<(String, String)>, PluginVersion) {
    let first = files_of(&[
        (INDEX, "expanded: [company.md]\n"),
        ("docs/company.md", "# Company\n\nWe make things.\n"),
        (CATALOG, CATALOG_TEXT),
    ]);
    let planned = planned(&view(&first), &acme()).unwrap();
    let version = planned.drift.clone().unwrap().version;
    (applied_plan(&first, &planned), version)
}

#[test]
fn the_version_is_kept_when_the_rendering_equals_what_the_folder_holds() {
    let (rendered, version) = rendered_once();
    let again = drift_of(&view(&rendered));
    assert!(again.writes.is_empty() && again.removals.is_empty());
    assert_eq!(again.version, version);
}

#[test]
fn the_version_is_raised_when_a_document_changes() {
    let (mut changed, version) = rendered_once();
    for (path, text) in &mut changed {
        if path == "docs/company.md" {
            *text = "# Company\n\nWe make more things.\n".to_owned();
        }
    }
    assert_eq!(drift_of(&view(&changed)).version, version.raised());
}

#[test]
fn the_version_is_raised_when_a_file_the_skill_no_longer_holds_is_removed() {
    let (mut stray, version) = rendered_once();
    stray.push(("skills/acme/notes.md".to_owned(), "old".to_owned()));
    let cleaned = drift_of(&view(&stray));
    assert_eq!(cleaned.removals, [PathBuf::from("skills/acme/notes.md")]);
    assert_eq!(cleaned.version, version.raised());
}

/// `tree` with the plan's skill files and catalog applied.
fn applied_plan(tree: &[(String, String)], plan: &PluginPlan) -> Vec<(String, String)> {
    let mut files = applied(tree, plan.drift.as_ref().unwrap());
    if let Some(catalog) = &plan.catalog {
        files.retain(|(path, _)| path != CATALOG);
        files.push((CATALOG.to_owned(), catalog.text.clone()));
    }
    files
}

#[test]
fn the_skill_stands_at_the_root_and_no_manifest_is_written() {
    let tree = repository(Some(CATALOG_TEXT), &[]);
    let paths: Vec<PathBuf> = drift_of(&tree)
        .writes
        .into_iter()
        .map(|file| file.path)
        .collect();
    assert_eq!(paths, [PathBuf::from("skills/acme/SKILL.md")]);
}

#[test]
fn findings_that_leave_the_version_unknown_leave_the_catalog_unwritten() {
    let tree = Committed::of(&[(CATALOG, CATALOG_TEXT)]);
    let plan = planned(&tree, &acme()).unwrap();
    assert_eq!(plan.findings, [PluginFinding::NoIndex]);
    assert_eq!(plan.drift, None);
    assert_eq!(plan.catalog, None);
}

#[test]
fn the_entry_is_set_by_value_and_every_other_member_keeps_its_value_and_place() {
    let listed = r#"{"name": "acme-tools", "plugins": [{"name": "acme", "version": "9", "category": "knowledge", "source": "./old"}, {"name": "other"}], "owner": {"name": "Acme"}}"#;
    let tree = repository(Some(listed), &[]);
    let file = catalog(&tree, "1.2.3").unwrap().unwrap().unwrap();
    // What the entry held beyond its own values, the other entry and the
    // catalog's other members are the input's, in the input's order.
    let input: Json = serde_json::from_str(listed).unwrap();
    let written: Json = serde_json::from_str(&file.text).unwrap();
    assert_eq!(
        written["plugins"][0]["category"],
        input["plugins"][0]["category"]
    );
    assert_eq!(written["plugins"][1], input["plugins"][1]);
    assert_eq!(written["owner"], input["owner"]);
    let members =
        |value: &Json| -> Vec<String> { value.as_object().unwrap().keys().cloned().collect() };
    assert_eq!(members(&written), members(&input));
    // Once set, the entry holds its values: the catalog is left as it is.
    let tree = repository(Some(&file.text), &[]);
    assert_eq!(catalog(&tree, "1.2.3").unwrap(), Ok(None));
}

#[test]
fn a_missing_catalog_is_created_holding_the_entry() {
    let file = catalog(&repository(None, &[]), "0.1.0")
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(file.current, None);
    // The created catalog already holds the entry: nothing more to set.
    let created = repository(Some(&file.text), &[]);
    assert_eq!(catalog(&created, "0.1.0").unwrap(), Ok(None));
}

/// Whether setting the entry finds the catalog `text` unreadable.
fn unreadable(text: &str) -> bool {
    matches!(
        catalog(&repository(Some(text), &[]), "0.1.0").unwrap(),
        Err(PluginFinding::CatalogUnreadable { .. })
    )
}

#[test]
fn a_catalog_that_is_not_json_is_a_finding() {
    assert!(unreadable("{"));
}

#[test]
fn a_catalog_that_is_no_json_object_is_a_finding() {
    assert!(unreadable("[]"));
}

#[test]
fn a_catalog_holding_no_plugins_list_is_a_finding() {
    assert!(unreadable("{\"plugins\": {}}"));
}

/// The output of the renderer before the skill moved to the root: its
/// manifest held the version, and its catalog entry none.
const EARLIER_MANIFEST: &str = "{\n  \"name\": \"acme\",\n  \"version\": \"0.3.7\",\n  \"description\": \"The knowledge of Acme, as the company keeps it in its own repository.\",\n  \"author\": {\n    \"name\": \"Acme\"\n  }\n}\n";
const EARLIER_CATALOG: &str = "{\n  \"name\": \"acme-tools\",\n  \"owner\": {\n    \"name\": \"Acme\"\n  },\n  \"plugins\": [\n    {\n      \"name\": \"acme\",\n      \"source\": \"./plugins/acme\",\n      \"description\": \"The knowledge of Acme, as the company keeps it in its own repository.\"\n    }\n  ]\n}\n";

#[test]
fn an_earlier_renderings_output_upgrades_with_its_version_and_then_converges() {
    let earlier = files_of(&[
        (INDEX, "expanded: [company.md]\n"),
        ("docs/company.md", "# Company\n\nWe make things.\n"),
        (CATALOG, EARLIER_CATALOG),
        ("plugins/acme/.claude-plugin/plugin.json", EARLIER_MANIFEST),
        (
            "plugins/acme/skills/acme/SKILL.md",
            "an earlier rendering\n",
        ),
    ]);
    let upgrade = planned(&view(&earlier), &acme()).unwrap();
    assert_eq!(upgrade.findings, []);
    // The skill is written at the root, a step above the version the
    // earlier manifest held; its folder is left as it is.
    let drift = upgrade.drift.clone().unwrap();
    let carried = PluginVersion::parse("0.3.7").unwrap().raised();
    assert_eq!(drift.version, carried);
    assert!(drift.removals.is_empty());
    let written: Vec<&Path> = drift
        .writes
        .iter()
        .map(|file| file.path.as_path())
        .collect();
    assert_eq!(written, [Path::new("skills/acme/SKILL.md")]);
    let upgraded = applied_plan(&earlier, &upgrade);
    // What the earlier rendering left is kept as it was.
    let left = |files: &[(String, String)]| -> Vec<(String, String)> {
        let earlier = [
            "plugins/acme/.claude-plugin/plugin.json",
            "plugins/acme/skills/acme/SKILL.md",
        ];
        files
            .iter()
            .filter(|(path, _)| earlier.contains(&path.as_str()))
            .cloned()
            .collect()
    };
    assert_eq!(left(&upgraded), left(&earlier));
    // Rendering again changes nothing.
    let again = planned(&view(&upgraded), &acme()).unwrap();
    assert_eq!(again.findings, []);
    assert_eq!(again.catalog, None);
    let again = again.drift.unwrap();
    assert!(again.writes.is_empty() && again.removals.is_empty());
    assert_eq!(again.version, carried);
}

/// The findings of planning a repository whose catalog holds the earlier
/// versionless entry, beside `more`.
fn versionless_findings(more: &[(&str, &str)]) -> Vec<PluginFinding> {
    let mut files = vec![
        (INDEX, "expanded: [company.md]\n"),
        ("docs/company.md", "# Company\n"),
        (CATALOG, EARLIER_CATALOG),
    ];
    files.extend_from_slice(more);
    planned(&Committed::of(&files), &acme()).unwrap().findings
}

#[test]
fn a_versionless_entry_without_an_earlier_manifest_is_a_finding() {
    assert_eq!(
        versionless_findings(&[]),
        [PluginFinding::EntryVersion { version: None }]
    );
}

#[test]
fn a_versionless_entry_beside_a_manifest_version_that_is_not_three_numbers_is_a_finding() {
    assert_eq!(
        versionless_findings(&[(
            "plugins/acme/.claude-plugin/plugin.json",
            "{\"version\": \"1.0\"}\n"
        )]),
        [PluginFinding::EntryVersion { version: None }]
    );
}

#[test]
fn an_entry_whose_version_is_not_three_numbers_is_a_finding_naming_it() {
    let held = "one";
    let plan = planned(
        &repository(
            Some(&format!(
                "{{\"plugins\": [{{\"name\": \"acme\", \"version\": \"{held}\"}}]}}"
            )),
            &[],
        ),
        &acme(),
    )
    .unwrap();
    assert_eq!(
        plan.findings,
        [PluginFinding::EntryVersion {
            version: Some(held.to_owned())
        }]
    );
    assert_eq!((plan.drift, plan.catalog), (None, None));
}

/// What planning finds with a link at `link`, and whether it still
/// knows what differs.
fn linked(link: &str) -> (Vec<PluginFinding>, bool) {
    let tree = repository(Some(CATALOG_TEXT), &[]).with_link(link);
    let plan = planned(&tree, &acme()).unwrap();
    (plan.findings, plan.drift.is_some())
}

#[test]
fn a_link_on_the_way_to_the_skill_is_a_finding_and_nothing_is_compared() {
    let link = "skills/acme";
    assert_eq!(
        linked(link),
        (
            vec![PluginFinding::Link {
                path: link.to_owned()
            }],
            false
        )
    );
}

#[test]
fn a_link_inside_the_skill_is_a_finding_and_nothing_is_compared() {
    let link = "skills/acme/x.md";
    assert_eq!(
        linked(link),
        (
            vec![PluginFinding::Link {
                path: link.to_owned()
            }],
            false
        )
    );
}

#[test]
fn a_link_on_the_way_to_the_catalog_is_a_finding_and_nothing_is_compared() {
    let link = ".claude-plugin";
    assert_eq!(
        linked(link),
        (
            vec![PluginFinding::Link {
                path: link.to_owned()
            }],
            false
        )
    );
}

#[test]
fn a_link_elsewhere_is_no_finding() {
    assert_eq!(linked("skills/other"), (vec![], true));
}

#[test]
fn a_plugins_folder_is_never_read_written_or_removed() {
    let tree = repository(
        Some(CATALOG_TEXT),
        &[("plugins/acme/skills/acme/SKILL.md", "an earlier layout")],
    );
    let drift = drift_of(&tree);
    assert!(drift.removals.is_empty());
    let written: Vec<&Path> = drift
        .writes
        .iter()
        .map(|file| file.path.as_path())
        .collect();
    assert_eq!(written, [Path::new("skills/acme/SKILL.md")]);
}

#[test]
fn the_shared_readings_findings_and_the_plugins_own_are_reported_together() {
    let tree = Committed::of(&[
        (INDEX, "expanded: [company.md, people.md]\n"),
        (
            "docs/company.md",
            "# Company\n\nSee [notes](../notes.md).\n",
        ),
        (
            "docs/people.md",
            "---\nvisibility: secret\n---\n\n# People\n",
        ),
    ]);
    let found = findings(&tree);
    assert!(
        matches!(
            found.as_slice(),
            [
                PluginFinding::Reading(_),
                PluginFinding::UnknownVisibility { document, .. },
            ] if document == "docs/people.md"
        ),
        "{found:?}"
    );
}

#[test]
fn a_company_repository_without_a_company_identity_is_refused_with_its_finding() {
    let tree = Committed::of(&[
        (
            "instrukt.yaml",
            "project: company-acme\ncompany_repository: true\n",
        ),
        (INDEX, "expanded: [company.md]\n"),
        ("docs/company.md", "# Company\n"),
    ]);
    let refused = plan(&tree).unwrap();
    assert_eq!(
        refused.findings,
        [PluginFinding::Identity(IdentityFinding::NameMissing)]
    );
    assert_eq!(
        (refused.folder, refused.drift, refused.catalog),
        (None, None, None)
    );
    assert_eq!(
        super::findings(&tree).unwrap(),
        [PluginFinding::Identity(IdentityFinding::NameMissing)]
    );
}

#[test]
fn the_plugin_is_rendered_under_the_identity_the_project_layer_declares() {
    let tree = Committed::of(&[
        (
            "instrukt.yaml",
            "project: company-acme\ncompany_repository: true\ncompany_name: Acme\n",
        ),
        (INDEX, "expanded: [company.md]\n"),
        ("docs/company.md", "# Company\n"),
    ]);
    let rendered = plan(&tree).unwrap();
    assert!(rendered.findings.is_empty(), "{:?}", rendered.findings);
    assert_eq!(rendered.folder, Some(PathBuf::from("skills/acme")));
    assert_eq!(rendered, planned(&tree, &acme()).unwrap());
}
