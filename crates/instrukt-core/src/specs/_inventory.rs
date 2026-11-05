//! The use-case inventory: every use case and anonymous scenario of a
//! project's feature specs, with the todo that claims each use case.
//!
//! The reading rules are `use-case-coverage`'s "The inventory": definition
//! lines are ATX headings of level 2 to 6 and `Scenario:` lines inside fenced
//! Gherkin (prose is never Gherkin); a heading is a use case only while a
//! fenced Gherkin block follows it in its section; within one file an id
//! counts once. Claims come from the docs
//! model's pending carriers (`binding-reservation`, "The claim").

use std::path::{Path, PathBuf};

use crate::docs::{Corpus, PendingBlock, Scope, Snippet, Taxonomy};
use crate::ids::{SnippetId, TodoSlug, UseCaseId, UseCaseRef};

/// Which todo holds a use case for its build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// No pending marker encloses it and its file is not pending for one
    /// todo.
    Unclaimed,
    /// The todo whose pending marker encloses its definition line, or the
    /// single todo delivering its whole file.
    Held(TodoSlug),
    /// Its file is pending for several todos and carries no in-body marker
    /// to say which holds what: the delivering slugs.
    Ambiguous(Vec<TodoSlug>),
}

/// One use case of the inventory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseCase {
    reference: UseCaseRef,
    file: PathBuf,
    claim: Claim,
}

impl UseCase {
    /// Its spec id and use-case id: what a binding names.
    pub fn reference(&self) -> &UseCaseRef {
        &self.reference
    }

    /// The spec file, relative to the repository root.
    pub fn file(&self) -> &Path {
        &self.file
    }

    pub fn claim(&self) -> &Claim {
        &self.claim
    }
}

/// Gherkin with no use-case id: not bindable, the id-assignment backlog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnonymousScenario {
    spec: SnippetId,
    file: PathBuf,
    definition: String,
}

impl AnonymousScenario {
    pub fn spec(&self) -> &SnippetId {
        &self.spec
    }

    /// The spec file, relative to the repository root.
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// The heading or `Scenario:` line that opens it, trimmed.
    pub fn definition(&self) -> &str {
        &self.definition
    }
}

/// A whole spec file pending for several todos with no in-body marker
/// saying which of them holds which of its use cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AmbiguousClaim {
    pub spec: SnippetId,
    pub delivered_by: Vec<TodoSlug>,
}

/// Every use case and anonymous scenario in a project's feature specs, read
/// from its working tree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Inventory {
    use_cases: Vec<UseCase>,
    anonymous: Vec<AnonymousScenario>,
    ambiguous: Vec<AmbiguousClaim>,
    spec_files: usize,
}

impl Inventory {
    /// Reads the feature specs of `corpus` — both halves, companions
    /// excluded, pending use cases included as written.
    pub fn read(corpus: &Corpus) -> Self {
        let mut inventory = Self::default();
        for snippet in corpus.snippets().iter().filter(|s| is_feature_spec(s)) {
            inventory.add(snippet);
        }
        inventory
    }

    fn add(&mut self, snippet: &Snippet) {
        let version = snippet.current();
        let scan = scan_spec(version.body());
        if scan.use_cases.is_empty() && scan.anonymous.is_empty() {
            return;
        }
        self.spec_files += 1;
        let blocks = version.pending_blocks();
        let delivered_by = snippet.delivered_by();
        let ambiguous = delivered_by.len() > 1 && blocks.is_empty();
        if ambiguous && !scan.use_cases.is_empty() {
            self.ambiguous.push(AmbiguousClaim {
                spec: snippet.id().clone(),
                delivered_by: delivered_by.to_vec(),
            });
        }
        for (id, lines) in scan.use_cases {
            let claim = if ambiguous {
                Claim::Ambiguous(delivered_by.to_vec())
            } else {
                claim_for(&lines, &blocks, delivered_by)
            };
            self.use_cases.push(UseCase {
                reference: UseCaseRef {
                    spec: snippet.id().clone(),
                    use_case: id,
                },
                file: snippet.path().to_path_buf(),
                claim,
            });
        }
        for (_, definition) in scan.anonymous {
            self.anonymous.push(AnonymousScenario {
                spec: snippet.id().clone(),
                file: snippet.path().to_path_buf(),
                definition,
            });
        }
    }

    /// Every use case, by spec (in identity order), then by first
    /// appearance in its file.
    pub fn use_cases(&self) -> &[UseCase] {
        &self.use_cases
    }

    /// The use case `reference` names, if the inventory holds it.
    pub fn use_case(&self, reference: &UseCaseRef) -> Option<&UseCase> {
        self.use_cases.iter().find(|u| &u.reference == reference)
    }

    /// Every anonymous scenario, each counted once.
    pub fn anonymous(&self) -> &[AnonymousScenario] {
        &self.anonymous
    }

    /// The spec files whose use cases no marker can attribute.
    pub fn ambiguous_claims(&self) -> &[AmbiguousClaim] {
        &self.ambiguous
    }

    /// How many spec files hold at least one use case or anonymous
    /// scenario.
    pub fn spec_files(&self) -> usize {
        self.spec_files
    }
}

/// A spec below `spec/feature/` in either half.
fn is_feature_spec(snippet: &Snippet) -> bool {
    if snippet.taxonomy() != Taxonomy::Spec {
        return false;
    }
    // The segment after the taxonomy: `<domain>/spec/<here>/…` or
    // `project/<slug>/spec/<here>/…`.
    let at = match snippet.scope() {
        Scope::Global => 2,
        Scope::Project => 3,
    };
    snippet.id().as_str().split('/').nth(at) == Some("feature")
}

/// The enclosing marker's todo wins; otherwise a single whole-file
/// delivering todo holds the use case.
fn claim_for(lines: &[usize], blocks: &[PendingBlock], delivered_by: &[TodoSlug]) -> Claim {
    let enclosing = blocks
        .iter()
        .find(|block| lines.iter().any(|line| block.lines.contains(line)));
    match (enclosing, delivered_by) {
        (Some(block), _) => Claim::Held(block.slug.clone()),
        (None, [only]) => Claim::Held(only.clone()),
        (None, _) => Claim::Unclaimed,
    }
}

/// What one spec body holds.
#[derive(Debug, Default, PartialEq, Eq)]
struct SpecScan {
    /// Each use-case id once, in order of first appearance, with every
    /// body line that defines it.
    use_cases: Vec<(UseCaseId, Vec<usize>)>,
    /// Each anonymous scenario: its body line and its trimmed text.
    anonymous: Vec<(usize, String)>,
}

impl SpecScan {
    fn define(&mut self, id: UseCaseId, line: usize) {
        match self.use_cases.iter_mut().find(|(known, _)| *known == id) {
            Some((_, lines)) => lines.push(line),
            None => self.use_cases.push((id, vec![line])),
        }
    }
}

/// One `Scenario:` line inside fenced Gherkin: its body line, its own id
/// and its trimmed text.
struct ScenarioLine {
    line: usize,
    id: Option<UseCaseId>,
    text: String,
}

/// A heading and what its section (up to the next heading) holds; the
/// preamble before the first heading is a section without a heading.
#[derive(Default)]
struct Section {
    heading: Option<Heading>,
    /// A fenced Gherkin block opens in the section.
    gherkin: bool,
    /// A `When` line inside fenced Gherkin.
    gherkin_when: bool,
    scenarios: Vec<ScenarioLine>,
}

struct Heading {
    line: usize,
    level: usize,
    id: Option<UseCaseId>,
    text: String,
}

fn scan_spec(body: &str) -> SpecScan {
    let mut sections = vec![Section::default()];
    // The open fence: its character, run length and whether it is Gherkin.
    let mut fence: Option<(char, usize, bool)> = None;
    for (index, raw) in body.split_inclusive('\n').enumerate() {
        let line = raw.trim_end_matches(['\n', '\r']);
        let trimmed = line.trim_start();
        let opener = fence_opener(trimmed);
        let (fenced, in_gherkin, opens_gherkin) = match (fence, opener) {
            (None, Some((ch, len, info))) => {
                let gherkin = info.eq_ignore_ascii_case("gherkin");
                fence = Some((ch, len, gherkin));
                (true, false, gherkin)
            }
            (Some((ch, len, _)), Some((close_ch, close_len, info)))
                if ch == close_ch && close_len >= len && info.is_empty() =>
            {
                fence = None;
                (true, false, false)
            }
            (Some((_, _, gherkin)), _) => (true, gherkin, false),
            (None, None) => (false, false, false),
        };
        if !fenced && let Some((level, title)) = heading(line) {
            sections.push(Section {
                heading: Some(Heading {
                    line: index,
                    level,
                    id: use_case_id(title),
                    text: line.trim().to_owned(),
                }),
                ..Section::default()
            });
            continue;
        }
        let Some(section) = sections.last_mut() else {
            continue;
        };
        section.gherkin |= opens_gherkin;
        if !in_gherkin {
            // Prose is never Gherkin.
            continue;
        }
        section.gherkin_when |= is_when_line(trimmed);
        if let Some(rest) = trimmed.strip_prefix("Scenario:") {
            section.scenarios.push(ScenarioLine {
                line: index,
                id: use_case_id(rest),
                text: line.trim().to_owned(),
            });
        }
    }
    let mut scan = SpecScan::default();
    for section in sections {
        scan_section(&mut scan, section);
    }
    scan
}

/// A use-case heading (level 2 to 6, an id, Gherkin in its section)
/// defines its id; an unnamed `Scenario:` that is its only `Scenario:`
/// line inherits that id, and every other unnamed one is anonymous. Under
/// any other heading an
/// unnamed `Scenario:` is anonymous, and a section with Gherkin `When`
/// but no `Scenario:` line is one anonymous scenario when its heading
/// carries no id.
fn scan_section(scan: &mut SpecScan, section: Section) {
    let use_case_heading = section.heading.as_ref().and_then(|h| match &h.id {
        Some(id) if (2..=6).contains(&h.level) && section.gherkin => Some((id.clone(), h.line)),
        _ => None,
    });
    if let Some((id, line)) = &use_case_heading {
        scan.define(id.clone(), *line);
    }
    if let Some(heading) = &section.heading
        && heading.id.is_none()
        && section.gherkin_when
        && section.scenarios.is_empty()
    {
        scan.anonymous.push((heading.line, heading.text.clone()));
    }
    // Only the heading's single scenario, named or not, inherits.
    let single = section.scenarios.len() == 1;
    for scenario in section.scenarios {
        match (scenario.id, &use_case_heading) {
            (Some(id), _) => scan.define(id, scenario.line),
            // The scenario takes the heading's identity, but its line
            // carries no id: it is no definition line and no claim can
            // enclose it.
            (None, Some(_)) if single => {}
            (None, _) => scan.anonymous.push((scenario.line, scenario.text)),
        }
    }
}

/// A fence opener or closer: its character, its run length and its info
/// string's first word.
fn fence_opener(trimmed: &str) -> Option<(char, usize, &str)> {
    let ch = trimmed.chars().next()?;
    if ch != '`' && ch != '~' {
        return None;
    }
    let len = trimmed.chars().take_while(|c| *c == ch).count();
    if len < 3 {
        return None;
    }
    let info = trimmed[len..].split_whitespace().next().unwrap_or_default();
    Some((ch, len, info))
}

/// An ATX heading: its level and its title.
fn heading(line: &str) -> Option<(usize, &str)> {
    let level = line.chars().take_while(|c| *c == '#').count();
    let rest = &line[level..];
    let valid = (1..=6).contains(&level)
        && (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t'));
    valid.then_some((level, rest))
}

/// A line beginning with the word `When`.
fn is_when_line(trimmed: &str) -> bool {
    trimmed
        .strip_prefix("When")
        .is_some_and(|rest| rest.chars().next().is_none_or(|c| !is_token_char(c)))
}

fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// The use-case id a definition line carries: its first id token.
fn use_case_id(text: &str) -> Option<UseCaseId> {
    UseCaseId::first_in(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(scan: &SpecScan) -> Vec<UseCaseId> {
        scan.use_cases.iter().map(|(id, _)| id.clone()).collect()
    }

    fn id(text: &str) -> UseCaseId {
        UseCaseId::parse(text).unwrap()
    }

    #[test]
    fn a_heading_with_gherkin_and_a_scenario_line_are_use_cases_counted_once_per_id() {
        let body = "### Use cases\n\n#### UC-AB1: one\n\n```gherkin\nGiven x\nWhen y\nThen z\n```\n\n\
                    ```gherkin\nScenario: UC-AB2 two\n  When y\nScenario: UC-AB1 again\n```\n";
        let scan = scan_spec(body);
        assert_eq!(ids(&scan), vec![id("UC-AB1"), id("UC-AB2")]);
        assert!(scan.anonymous.is_empty());
    }

    #[test]
    fn a_heading_without_gherkin_is_retired() {
        let scan = scan_spec("#### UC-AB1: retired\n\nNo longer specified.\n");
        assert_eq!(scan, SpecScan::default());
    }

    #[test]
    fn an_unnamed_scenario_under_an_id_less_heading_counts_once() {
        let body = "## Behaviour\n\n```gherkin\nScenario: unnamed\nWhen it runs\n```\n";
        let scan = scan_spec(body);
        assert!(scan.use_cases.is_empty());
        assert_eq!(scan.anonymous.len(), 1);
    }

    #[test]
    fn scenario_and_when_lines_outside_fenced_gherkin_are_prose() {
        let body = "## Notes\n\nScenario: told in prose\nWhen it runs, nothing is counted.\n\n\
                    ```text\nWhen it runs\n```\n";
        assert_eq!(scan_spec(body), SpecScan::default());
    }

    #[test]
    fn headings_inside_fenced_code_are_not_headings() {
        let body = "## Example\n\n```sh\n# UC-AB1 not a heading\n```\n";
        assert_eq!(scan_spec(body), SpecScan::default());
    }

    #[test]
    fn a_level_one_heading_defines_no_use_case() {
        let body = "# UC-AB1 title\n\n```gherkin\nWhen x\n```\n";
        assert_eq!(scan_spec(body), SpecScan::default());
    }

    #[test]
    fn a_word_starting_with_when_is_not_a_when_line() {
        assert!(is_when_line("When it runs"));
        assert!(is_when_line("When"));
        assert!(!is_when_line("Whenever it runs"));
    }

    fn slug(text: &str) -> TodoSlug {
        TodoSlug::parse(text).unwrap()
    }

    #[test]
    fn the_enclosing_marker_claims_before_the_whole_file() {
        let blocks = [PendingBlock {
            slug: slug("inner"),
            lines: 3..=9,
        }];
        let whole = [slug("whole")];
        assert_eq!(claim_for(&[5], &blocks, &whole), Claim::Held(slug("inner")));
        assert_eq!(
            claim_for(&[12], &blocks, &whole),
            Claim::Held(slug("whole"))
        );
        assert_eq!(claim_for(&[12], &blocks, &[]), Claim::Unclaimed);
        assert_eq!(
            claim_for(&[12], &blocks, &[slug("a"), slug("b")]),
            Claim::Unclaimed
        );
    }
}
