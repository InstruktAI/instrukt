//! Coverage: the inventory reconciled with the binding set, and the ratchet
//! over orphans and anonymous scenarios (`use-case-coverage`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::_bindings::{Binding, BindingSet};
use super::_inventory::{AmbiguousClaim, AnonymousScenario, Inventory};
use super::SpecsError;
use crate::docs::Corpus;
use crate::fsx::atomic_write;
use crate::home::Home;
use crate::ids::UseCaseRef;

/// The committed ratchet baseline, at the repository root.
pub const BASELINE_FILE: &str = "instrukt-coverage-baseline.json";

/// The report's counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    /// Spec files holding at least one scenario.
    pub spec_files: usize,
    /// Use cases plus anonymous scenarios.
    pub scenarios: usize,
    /// Use cases: scenarios with a use-case id.
    pub bindable: usize,
    pub anonymous: usize,
    pub bound: usize,
    pub unbound: usize,
    pub multi_bound: usize,
    /// Bindings naming no use case.
    pub orphans: usize,
}

/// A project's use cases against its tests' bindings, read from its working
/// tree.
#[derive(Debug, Clone)]
pub struct Coverage {
    repo_root: PathBuf,
    inventory: Inventory,
    bindings: BindingSet,
    /// Bindings per use case of the inventory.
    tests: HashMap<UseCaseRef, usize>,
    orphans: Vec<Binding>,
}

impl Coverage {
    /// Reads the feature specs and the tests of the repository rooted at
    /// `repo_root`.
    pub fn measure(repo_root: &Path, home: &Home) -> Result<Self, SpecsError> {
        let corpus = Corpus::load(repo_root, home)?;
        let inventory = Inventory::read(&corpus);
        let bindings = BindingSet::read(repo_root)?;
        let mut tests: HashMap<UseCaseRef, usize> = inventory
            .use_cases()
            .iter()
            .map(|u| (u.reference().clone(), 0))
            .collect();
        let mut orphans = Vec::new();
        for binding in bindings.bindings() {
            match binding.use_case().and_then(|r| tests.get_mut(r)) {
                Some(count) => *count += 1,
                None => orphans.push(binding.clone()),
            }
        }
        let coverage = Self {
            repo_root: repo_root.to_path_buf(),
            inventory,
            bindings,
            tests,
            orphans,
        };
        let counts = coverage.counts();
        tracing::debug!(
            event = "specs_coverage_measured",
            repo = %repo_root.display(),
            use_cases = counts.bindable,
            anonymous = counts.anonymous,
            bound = counts.bound,
            orphans = counts.orphans,
            "use-case coverage measured"
        );
        Ok(coverage)
    }

    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    pub fn bindings(&self) -> &BindingSet {
        &self.bindings
    }

    pub fn counts(&self) -> Counts {
        let bindable = self.inventory.use_cases().len();
        let anonymous = self.inventory.anonymous().len();
        let bound = self.tests.values().filter(|n| **n > 0).count();
        Counts {
            spec_files: self.inventory.spec_files(),
            scenarios: bindable + anonymous,
            bindable,
            anonymous,
            bound,
            unbound: bindable - bound,
            multi_bound: self.tests.values().filter(|n| **n > 1).count(),
            orphans: self.orphans.len(),
        }
    }

    /// The bindings naming no use case of the inventory, by file and line.
    pub fn orphans(&self) -> &[Binding] {
        &self.orphans
    }

    /// The use cases no binding names, in inventory order.
    pub fn unbound(&self) -> Vec<&UseCaseRef> {
        self.in_order(|n| n == 0).map(|(r, _)| r).collect()
    }

    /// The use cases named by more than one binding, with their number of
    /// tests, in inventory order.
    pub fn multi_bound(&self) -> Vec<(&UseCaseRef, usize)> {
        self.in_order(|n| n > 1).collect()
    }

    pub fn anonymous(&self) -> &[AnonymousScenario] {
        self.inventory.anonymous()
    }

    pub fn ambiguous_claims(&self) -> &[AmbiguousClaim] {
        self.inventory.ambiguous_claims()
    }

    fn in_order(&self, keep: impl Fn(usize) -> bool) -> impl Iterator<Item = (&UseCaseRef, usize)> {
        self.inventory.use_cases().iter().filter_map(move |u| {
            let n = self.tests.get(u.reference()).copied().unwrap_or_default();
            keep(n).then_some((u.reference(), n))
        })
    }

    /// Compares the ratcheted counts with the committed baseline (zero for
    /// both when there is none) and collects the ambiguous claims.
    pub fn check(&self) -> Result<Check, SpecsError> {
        let baseline = Baseline::read(&self.repo_root)?;
        let counts = self.counts();
        let ratchets = [
            Ratchet {
                debt: Debt::Orphans,
                count: counts.orphans,
                baseline: baseline.orphans,
            },
            Ratchet {
                debt: Debt::Anonymous,
                count: counts.anonymous,
                baseline: baseline.anonymous,
            },
        ];
        let check = Check {
            ambiguous: self.ambiguous_claims().to_vec(),
            ratchets: ratchets.to_vec(),
        };
        tracing::debug!(
            event = "specs_coverage_checked",
            repo = %self.repo_root.display(),
            passes = check.passes(),
            "use-case coverage checked"
        );
        Ok(check)
    }

    /// Records the current orphan and anonymous counts as the baseline.
    pub fn update_baseline(&self) -> Result<Baseline, SpecsError> {
        let counts = self.counts();
        let baseline = Baseline {
            orphans: counts.orphans,
            anonymous: counts.anonymous,
        };
        let path = self.repo_root.join(BASELINE_FILE);
        let mut text =
            serde_json::to_string_pretty(&baseline).map_err(|source| SpecsError::Baseline {
                path: path.clone(),
                source,
            })?;
        text.push('\n');
        atomic_write(&path, text.as_bytes())?;
        tracing::info!(
            event = "specs_baseline_updated",
            path = %path.display(),
            orphans = baseline.orphans,
            anonymous = baseline.anonymous,
            "coverage baseline updated"
        );
        Ok(baseline)
    }
}

/// The committed counts the check may not exceed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Baseline {
    pub orphans: usize,
    pub anonymous: usize,
}

impl Baseline {
    fn read(repo_root: &Path) -> Result<Self, SpecsError> {
        let path = repo_root.join(BASELINE_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => return Err(SpecsError::Read { path, source }),
        };
        serde_json::from_str(&text).map_err(|source| SpecsError::Baseline { path, source })
    }
}

/// A ratcheted kind of debt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Debt {
    Orphans,
    Anonymous,
}

/// One ratcheted count against its baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ratchet {
    pub debt: Debt,
    pub count: usize,
    pub baseline: usize,
}

/// The outcome of `--check`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    ambiguous: Vec<AmbiguousClaim>,
    ratchets: Vec<Ratchet>,
}

impl Check {
    /// No ambiguous claim and no count above its baseline.
    pub fn passes(&self) -> bool {
        self.ambiguous.is_empty() && self.regressions().is_empty()
    }

    /// Every ambiguous claim; each fails the check.
    pub fn ambiguous(&self) -> &[AmbiguousClaim] {
        &self.ambiguous
    }

    /// The counts above their baseline.
    pub fn regressions(&self) -> Vec<Ratchet> {
        self.ratchets
            .iter()
            .filter(|r| r.count > r.baseline)
            .copied()
            .collect()
    }

    /// The counts below their baseline: the baseline can be tightened.
    pub fn tightenable(&self) -> Vec<Ratchet> {
        self.ratchets
            .iter()
            .filter(|r| r.count < r.baseline)
            .copied()
            .collect()
    }
}
