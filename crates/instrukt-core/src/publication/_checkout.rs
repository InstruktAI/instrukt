//! Checkout entrypoints use the recorded published revision, without publishing
//! branch proposals. Only an unpublished primary checkout may bootstrap sync.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::_render::Renderer;
use super::_source::{self, Committed};
use super::_sync::{committed_index, lock_outputs, position_indexes, project_agents};
use super::_tree::Outputs;
use super::_tx::confined;
use super::{PublicationError, PublishedRecord, SyncOutcome, SyncRequest};
use crate::config::Configuration;
use crate::docs::{Corpus, Position, Values};
use crate::fsx::atomic_write;
use crate::index::GlobalIndex;

const CHECKOUT_OUTPUTS_VERSION: u32 = 1;

pub(crate) enum Checkout {
    Primary,
    Linked,
    Bootstrapped,
}

pub(crate) fn checkout_entrypoints(
    request: &SyncRequest<'_>,
) -> Result<Checkout, PublicationError> {
    let project = _source::locate(request.cwd)?;
    let primary = project.worktree == project.primary;
    if PublishedRecord::load(request.home, &project.slug)?.is_none() {
        if !primary {
            return Err(PublicationError::CheckoutUnpublished {
                project: project.slug,
            });
        }
        let report = super::sync(request)
            .map_err(|error| PublicationError::CheckoutPublication(Box::new(error)))?;
        for leftover in &report.leftovers {
            tracing::warn!(event="checkout_bootstrap_leftover", path=%leftover.display());
        }
        if let SyncOutcome::Refused { refusal, .. } = report.outcome {
            return Err(PublicationError::CheckoutRefused(refusal.to_string()));
        }
        return Ok(Checkout::Bootstrapped);
    }
    let home = request.home;
    // This lock covers only live publication application and checkout rendering,
    // never publication preparation (including a pull/rebase waiting on sync).
    let _outputs = lock_outputs(home)?;
    let record = PublishedRecord::load(home, &project.slug)?.ok_or_else(|| {
        PublicationError::CheckoutUnpublished {
            project: project.slug.clone(),
        }
    })?;
    let committed = Committed::read(&project.primary, &record.commit)?;
    let temp = tempfile::tempdir().map_err(|source| PublicationError::Io {
        path: std::env::temp_dir(),
        source,
    })?;
    committed.materialize(temp.path())?;
    let config = Configuration::load(&home.for_inspection(), Some(temp.path()))?;
    let corpus = Corpus::from_sources(Some(project.slug.clone()), home, committed.doc_sources());
    let (held, _) = super::_tree::held_positions(&committed);
    let owned: BTreeSet<_> = held.iter().cloned().collect();
    let index = GlobalIndex::load(home)?.unwrap_or_default();
    let owners = super::_tree::claims(home)?;
    let indexes = position_indexes(home, &committed, &held, &owners, &project.slug)?;
    let project_index = committed_index(&committed, &Position::Project)?;
    let values = Values::publish(&config, request.wiring.caller_identity.as_deref());
    let renderer = Renderer {
        home,
        project: &project.slug,
        corpus: &corpus,
        owned: &owned,
        index: &index,
        indexes: &indexes,
        values: &values,
    };
    let agents = project_agents(
        home,
        &committed,
        &renderer,
        project_index.as_ref(),
        &mut Vec::new(),
    )?;
    let git_dir = PathBuf::from(crate::git::run(
        &project.worktree,
        ["rev-parse", "--absolute-git-dir"],
    )?);
    let inventory_file = confined(&git_dir, Path::new("instrukt-entrypoints.json"))?;
    let previous = match CheckoutOutputs::load(&inventory_file)? {
        Some(inventory) => inventory,
        None => CheckoutOutputs {
            schema_version: CHECKOUT_OUTPUTS_VERSION,
            agents: Outputs::load(home, &project.slug)?.agents,
        },
    };
    // Check the entire write set before touching a checkout; links never redirect
    // generated files outside the checkout being opened.
    for rel in agents.iter().map(|(rel, _)| rel).chain(&previous.agents) {
        confined(&project.worktree, rel)?;
    }
    // Remember every possible write before touching outputs. If a render stops
    // midway, the next render still knows which now-withdrawn files it owns.
    let desired: Vec<PathBuf> = agents.iter().map(|(rel, _)| rel.clone()).collect();
    let mut retained: BTreeSet<PathBuf> = previous.agents.iter().cloned().collect();
    retained.extend(desired.iter().cloned());
    CheckoutOutputs {
        schema_version: CHECKOUT_OUTPUTS_VERSION,
        agents: retained.into_iter().collect(),
    }
    .save(&inventory_file)?;
    for rel in previous
        .agents
        .iter()
        .filter(|old| !agents.iter().any(|(rel, _)| rel == *old))
    {
        let file = confined(&project.worktree, rel)?;
        match std::fs::remove_file(&file) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(PublicationError::Io { path: file, source }),
        }
    }
    for (rel, text) in agents {
        atomic_write(&confined(&project.worktree, &rel)?, text.as_bytes())?;
    }
    CheckoutOutputs {
        schema_version: CHECKOUT_OUTPUTS_VERSION,
        agents: desired,
    }
    .save(&inventory_file)?;
    tracing::debug!(event="checkout_entrypoints_rendered", project=%project.slug, checkout=%project.worktree.display(), commit=%record.commit);
    Ok(if primary {
        Checkout::Primary
    } else {
        Checkout::Linked
    })
}

#[derive(Serialize, Deserialize)]
struct CheckoutOutputs {
    schema_version: u32,
    agents: Vec<PathBuf>,
}

impl CheckoutOutputs {
    fn load(file: &Path) -> Result<Option<Self>, PublicationError> {
        let bytes = match std::fs::read(file) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(PublicationError::Io {
                    path: file.to_path_buf(),
                    source,
                });
            }
        };
        let inventory: Self =
            serde_json::from_slice(&bytes).map_err(|error| Self::invalid(file, error))?;
        if inventory.schema_version != CHECKOUT_OUTPUTS_VERSION {
            return Err(Self::invalid(
                file,
                "unsupported checkout output inventory version; upgrade instrukt",
            ));
        }
        Ok(Some(inventory))
    }

    fn save(&self, file: &Path) -> Result<(), PublicationError> {
        let bytes = serde_json::to_vec(self).map_err(|error| Self::invalid(file, error))?;
        atomic_write(file, &bytes)?;
        Ok(())
    }

    fn invalid(file: &Path, reason: impl std::fmt::Display) -> PublicationError {
        PublicationError::Rendering {
            file: file.display().to_string(),
            reason: reason.to_string(),
        }
    }
}
