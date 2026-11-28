//! Where a company home keeps its settings: `company.yaml`, beside the
//! project layer its repository commits
//! (`project/instrukt/spec/feature/configuration/company-homes`, The home).
//! A home made before keeps them in a root `instrukt.yaml` declaring
//! `company`; that document is read where it stands and converted to
//! `company.yaml`, every byte kept, by the first path that persists a
//! conversion.

use std::path::{Path, PathBuf};
use std::time::Instant;

use serde_yaml_ng::{Mapping, Value};

use super::_error::{ConfigError, Finding, FindingKind};
use super::_layer::{
    CONFIG_FILE_NAME, InitOutcome, Layer, MigrationOutcome, WRITE_LOCK_TIMEOUT, elapsed_ms,
    lock_path, migrate, persists_conversion, write,
};
use super::_schema::COMPANY_SCHEMA_VERSION;
use crate::fsx::{FsError, LockGuard, atomic_write};
use crate::home::Home;
use crate::ids::ProjectSlug;

/// The file name of a company home's settings document.
pub const COMPANY_DOCUMENT: &str = "company.yaml";

/// What a company home keeps on the computer and its repository ignores:
/// the settings document, the secrets file, the other checkouts and
/// instrukt's local state, as the home's `.gitignore` lines
/// (`project/instrukt/spec/feature/configuration/company-homes`).
pub(crate) const COMPANY_IGNORED: [&str; 4] =
    ["/company.yaml", "/secrets.env", "/projects/", "/.instrukt/"];

/// Where a company home's settings document stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CompanyDocument {
    /// `company.yaml`, the root `instrukt.yaml` being no company document.
    Present(PathBuf),
    /// Only a root `instrukt.yaml` declaring `company`: a home made before
    /// `company.yaml`, not yet converted.
    Legacy(PathBuf),
    /// `company.yaml` and a root `instrukt.yaml` declaring `company`: which
    /// one holds the company's settings is the person's to say.
    Both { company: PathBuf, legacy: PathBuf },
    /// No company document: no `company.yaml`, and no root `instrukt.yaml`
    /// or one that is a project layer.
    Absent,
}

/// Where the settings document of company `name`'s home stands.
pub(crate) fn resolve(home: &Home, name: &ProjectSlug) -> Result<CompanyDocument, ConfigError> {
    resolve_in(&home.company_dir(name))
}

/// Where the settings document of the company home at `dir` stands. A root
/// `instrukt.yaml` that cannot be read is no company document beside a
/// `company.yaml`; without one, its error is the answer.
pub(crate) fn resolve_in(dir: &Path) -> Result<CompanyDocument, ConfigError> {
    let company = dir.join(COMPANY_DOCUMENT);
    let legacy = dir.join(CONFIG_FILE_NAME);
    let company_present = present(&company)?;
    let legacy_is_company = match is_company_document(&legacy) {
        Ok(declares) => declares,
        Err(error @ ConfigError::LinkedCompanyDocument { .. }) => return Err(error),
        Err(_) if company_present => false,
        Err(error) => return Err(error),
    };
    Ok(match (company_present, legacy_is_company) {
        (true, true) => CompanyDocument::Both { company, legacy },
        (true, false) => CompanyDocument::Present(company),
        (false, true) => CompanyDocument::Legacy(legacy),
        (false, false) => CompanyDocument::Absent,
    })
}

/// Whether the document value `value` is a company's settings: a mapping
/// declaring `company`.
pub(crate) fn declares_company(value: &Value) -> bool {
    value
        .get("company")
        .is_some_and(|company| !company.is_null())
}

fn present(path: &Path) -> Result<bool, ConfigError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(ConfigError::LinkedCompanyDocument {
            path: path.to_path_buf(),
        }),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ConfigError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn is_company_document(path: &Path) -> Result<bool, ConfigError> {
    if !present(path)? {
        return Ok(false);
    }
    Ok(super::_layer::read(path)?.is_some_and(|document| declares_company(&document.value)))
}

/// Public entry point for runner/backup to bring a company home's legacy
/// settings document forward to `company.yaml` before snapshotting.
pub fn convert_company_document(home: &Home, name: &ProjectSlug) -> Result<(), ConfigError> {
    convert(home, name)
}

/// Converts the legacy settings document of company `name`'s home to
/// `company.yaml`: the ignore line first, so an interrupted conversion never
/// exposes the document to git, then the rename under the company document's
/// write lock, every byte kept. Nothing happens without a legacy document;
/// both present is refused and neither file changes.
pub(crate) fn convert(home: &Home, name: &ProjectSlug) -> Result<(), ConfigError> {
    let rel = PathBuf::from("companies").join(name.as_str());
    let dir = crate::fsx::confined(home.root(), &rel)?;
    // Confinement resolves home.root(), while diagnostics name the home's own spelling.
    let reported = home.company_dir(name);
    let both = || ConfigError::BothCompanyDocuments {
        company: reported.join(COMPANY_DOCUMENT),
        legacy: reported.join(CONFIG_FILE_NAME),
    };
    match resolve_in(&dir)? {
        CompanyDocument::Legacy(_) => {}
        CompanyDocument::Both { .. } => return Err(both()),
        CompanyDocument::Present(_) | CompanyDocument::Absent => return Ok(()),
    }
    ignore_local_material(&dir)?;
    let company = dir.join(COMPANY_DOCUMENT);
    let _lock = LockGuard::acquire(&lock_path(home, &company), WRITE_LOCK_TIMEOUT)?;
    // Another writer may have converted it while this one waited; if both
    // documents appeared in the meantime, refuse rather than swallowing.
    let legacy = match resolve_in(&dir)? {
        CompanyDocument::Legacy(legacy) => legacy,
        CompanyDocument::Both { .. } => return Err(both()),
        CompanyDocument::Present(_) | CompanyDocument::Absent => return Ok(()),
    };
    crate::fsx::rename(&legacy, &company).map_err(|source| {
        ConfigError::Fs(FsError::Write {
            path: company.clone(),
            source,
        })
    })?;
    tracing::debug!(
        event = "company_document_converted",
        from = %legacy.display(),
        to = %company.display(),
    );
    Ok(())
}

/// Writes the settings document of company `name` into `dir`: its ignore
/// lines first, then `company.yaml` holding the current schema version and
/// the name, when `dir` holds none. Returns whether it wrote one. The
/// caller holds whatever lock serializes writers of `dir`.
pub fn write_company_settings(dir: &Path, name: &ProjectSlug) -> Result<bool, ConfigError> {
    ignore_local_material(dir)?;
    let path = dir.join(COMPANY_DOCUMENT);
    if present(&path)? {
        return Ok(false);
    }
    let mut document = Mapping::new();
    document.insert(
        Value::String("schema_version".to_owned()),
        Value::Number(COMPANY_SCHEMA_VERSION.into()),
    );
    document.insert(
        Value::String("company".to_owned()),
        Value::String(name.to_string()),
    );
    write(&path, &[], &Value::Mapping(document))?;
    Ok(true)
}

/// Appends each line of [`COMPANY_IGNORED`] the `.gitignore` of the company
/// home at `dir` lacks, keeping every other line as it is. A `.gitignore`
/// that is a symbolic link is refused before anything is read or written:
/// the file may come from a cloned repository, and nothing outside the home
/// is read through it.
pub(crate) fn ignore_local_material(dir: &Path) -> Result<(), ConfigError> {
    let path = dir.join(".gitignore");
    match std::fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(ConfigError::LinkedIgnoreFile { path });
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(ConfigError::Read { path, source }),
    }
    let current = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(ConfigError::Read { path, source }),
    };
    let present: std::collections::BTreeSet<&str> = current.lines().map(str::trim).collect();
    let missing: Vec<&str> = COMPANY_IGNORED
        .into_iter()
        .filter(|line| !present.contains(line))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut text = current.clone();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for line in missing {
        text.push_str(line);
        text.push('\n');
    }
    atomic_write(&path, text.as_bytes())?;
    Ok(())
}

/// Converts a company's legacy document to `company.yaml` where reading
/// `layer` through `home` persists conversion; nothing otherwise.
pub(super) fn converted(home: &Home, layer: &Layer) -> Result<(), ConfigError> {
    match layer {
        Layer::Company(name) if persists_conversion(home, layer) => convert(home, name),
        _ => Ok(()),
    }
}

/// [`init`](super::_layer::init) of the company home `name`, its folder made and its legacy
/// document converted: an existing document is migrated, both present
/// refused, an absent one written as `company.yaml`.
pub(super) fn init_company(
    home: &Home,
    layer: &Layer,
    name: &ProjectSlug,
    started: Instant,
) -> Result<InitOutcome, ConfigError> {
    match resolve(home, name)? {
        CompanyDocument::Both { company, legacy } => {
            return Err(ConfigError::BothCompanyDocuments { company, legacy });
        }
        CompanyDocument::Present(_) | CompanyDocument::Legacy(_) => {
            return match migrate(home, layer)? {
                MigrationOutcome::Migrated { from } => Ok(InitOutcome::Migrated { from }),
                MigrationOutcome::AlreadyCurrent | MigrationOutcome::Absent => {
                    Ok(InitOutcome::AlreadyCurrent)
                }
            };
        }
        CompanyDocument::Absent => {}
    }
    let path = layer.path(home);
    let _lock = LockGuard::acquire(&lock_path(home, &path), WRITE_LOCK_TIMEOUT)?;
    if !write_company_settings(&home.company_dir(name), name)? {
        return Ok(InitOutcome::AlreadyCurrent);
    }
    tracing::debug!(
        event = "config_initialized",
        path = %path.display(),
        schema_version = layer.kind().current_schema_version(),
        duration_ms = elapsed_ms(started),
    );
    Ok(InitOutcome::Created)
}

/// What the company home `name` keeps on the computer that its repository
/// tracks or does not ignore, each a finding naming the path
/// (`project/instrukt/spec/feature/configuration/company-homes`, "What
/// stays out of the repository"). The settings document is the one at
/// `document`, where it stands: a home made before `company.yaml` keeps it
/// as its root `instrukt.yaml` until converted. Nothing outside a git
/// working tree.
pub(super) fn local_material(
    home: &Home,
    name: &crate::ids::ProjectSlug,
    document: &Path,
) -> Result<Vec<Finding>, ConfigError> {
    let dir = home.company_dir(name);
    if crate::git::worktree_root(&dir)?.is_none() {
        return Ok(Vec::new());
    }
    let settings = document
        .file_name()
        .map(|file| format!("/{}", file.to_string_lossy()));
    let mut found = Vec::new();
    for line in COMPANY_IGNORED {
        let line = match &settings {
            Some(settings) if line == COMPANY_IGNORED[0] => settings.as_str(),
            _ => line,
        };
        let relative = line.trim_matches('/');
        let path = dir.join(relative);
        let tracked = !crate::git::run(&dir, ["ls-files", "--", relative])?
            .trim()
            .is_empty();
        let message = if tracked {
            "the home's repository tracks it; it stays on this computer, so untrack it with `git rm -r --cached` and commit".to_owned()
        } else if path.exists() && !crate::git::succeeds(&dir, ["check-ignore", "-q", relative])? {
            format!(
                "the home's repository does not ignore it; `instrukt config init --company {name}` adds its ignore line"
            )
        } else {
            continue;
        };
        found.push(Finding {
            document: path,
            key: "repository".into(),
            kind: FindingKind::LeavesTheComputer,
            message,
        });
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::{
        COMPANY_DOCUMENT, CONFIG_FILE_NAME, CompanyDocument, ConfigError, Home, PathBuf,
        ProjectSlug, Value, convert, declares_company, lock_path, resolve, write_company_settings,
    };

    fn acme() -> ProjectSlug {
        ProjectSlug::parse("acme").unwrap()
    }

    fn home_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Home, PathBuf) {
        let root = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(root.path().to_path_buf()).for_writing();
        let dir = home.company_dir(&acme());
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        (root, home, dir)
    }

    const LEGACY: &str = "schema_version: 6\ncompany: acme\nmembers:\n  ana: {}\n";
    const COMPANY_YAML: &str = "company: acme\n";
    const PROJECT_LAYER: &str =
        "schema_version: 6\nproject: company-acme\ncompany_repository: true\n";

    #[test]
    fn company_yaml_beside_a_project_layer_is_present() {
        let (_root, home, dir) = home_with(&[
            (COMPANY_DOCUMENT, COMPANY_YAML),
            (CONFIG_FILE_NAME, PROJECT_LAYER),
        ]);
        assert_eq!(
            resolve(&home, &acme()).unwrap(),
            CompanyDocument::Present(dir.join(COMPANY_DOCUMENT))
        );
    }

    #[test]
    fn a_root_document_declaring_company_alone_is_legacy() {
        let (_root, home, dir) = home_with(&[(CONFIG_FILE_NAME, LEGACY)]);
        assert_eq!(
            resolve(&home, &acme()).unwrap(),
            CompanyDocument::Legacy(dir.join(CONFIG_FILE_NAME))
        );
    }

    #[test]
    fn a_generated_home_without_company_yaml_holds_no_company_document() {
        let (_root, home, _dir) = home_with(&[(CONFIG_FILE_NAME, PROJECT_LAYER)]);
        assert_eq!(resolve(&home, &acme()).unwrap(), CompanyDocument::Absent);
    }

    #[test]
    fn both_present_is_named() {
        let (_root, home, dir) =
            home_with(&[(COMPANY_DOCUMENT, COMPANY_YAML), (CONFIG_FILE_NAME, LEGACY)]);
        assert_eq!(
            resolve(&home, &acme()).unwrap(),
            CompanyDocument::Both {
                company: dir.join(COMPANY_DOCUMENT),
                legacy: dir.join(CONFIG_FILE_NAME),
            }
        );
    }

    #[test]
    fn converting_with_both_present_is_refused_and_changes_neither_file() {
        let (_root, home, dir) =
            home_with(&[(COMPANY_DOCUMENT, COMPANY_YAML), (CONFIG_FILE_NAME, LEGACY)]);
        let error = convert(&home, &acme()).unwrap_err();
        assert!(
            matches!(
                &error,
                ConfigError::BothCompanyDocuments { company, legacy }
                    if company == &dir.join(COMPANY_DOCUMENT) && legacy == &dir.join(CONFIG_FILE_NAME)
            ),
            "{error}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(COMPANY_DOCUMENT)).unwrap(),
            COMPANY_YAML
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(CONFIG_FILE_NAME)).unwrap(),
            LEGACY
        );
        assert!(!dir.join(".gitignore").exists());
    }

    #[test]
    fn converting_with_both_present_under_a_symlinked_home_root_names_the_homes_documents() {
        let scratch = tempfile::tempdir().unwrap();
        let real_user_home = scratch.path().join("real");
        let linked_user_home = scratch.path().join("linked");
        std::fs::create_dir_all(&real_user_home).unwrap();
        std::os::unix::fs::symlink(&real_user_home, &linked_user_home).unwrap();

        let home = Home::at_user_home(linked_user_home).for_writing();
        let dir = home.company_dir(&acme());
        let company_path = dir.join(COMPANY_DOCUMENT);
        let legacy_path = dir.join(CONFIG_FILE_NAME);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&company_path, COMPANY_YAML).unwrap();
        std::fs::write(&legacy_path, LEGACY).unwrap();
        let company_before = std::fs::read(&company_path).unwrap();
        let legacy_before = std::fs::read(&legacy_path).unwrap();

        let error = convert(&home, &acme()).unwrap_err();
        assert!(
            matches!(
                &error,
                ConfigError::BothCompanyDocuments { company, legacy }
                    if company == &company_path && legacy == &legacy_path
            ),
            "{error}"
        );
        assert_eq!(std::fs::read(&company_path).unwrap(), company_before);
        assert_eq!(std::fs::read(&legacy_path).unwrap(), legacy_before);
        assert!(!dir.join(".gitignore").exists());
    }

    #[test]
    fn converting_a_legacy_document_keeps_every_byte_and_ignores_it_first() {
        let legacy = format!("# my settings\n{LEGACY}");
        let (_root, home, dir) = home_with(&[
            (CONFIG_FILE_NAME, &legacy),
            (".gitignore", "/instrukt.yaml\n"),
        ]);
        // The home is the checkout of the company's repository.
        crate::git::run(&dir, ["init", "--quiet"]).unwrap();
        convert(&home, &acme()).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(COMPANY_DOCUMENT)).unwrap(),
            legacy
        );
        assert!(!dir.join(CONFIG_FILE_NAME).exists());
        // Git keeps the converted settings out of the repository.
        assert!(crate::git::succeeds(&dir, ["check-ignore", "--quiet", COMPANY_DOCUMENT]).unwrap());
        // Converting again finds nothing to convert.
        convert(&home, &acme()).unwrap();
        assert_eq!(
            resolve(&home, &acme()).unwrap(),
            CompanyDocument::Present(dir.join(COMPANY_DOCUMENT))
        );
    }

    #[test]
    fn inspection_reads_the_legacy_document_where_it_stands() {
        let (_root, home, dir) = home_with(&[(CONFIG_FILE_NAME, LEGACY)]);
        let inspection = home.for_inspection();
        let layer = super::super::Layer::Company(acme());
        let view = super::super::inspect_layer(&inspection, &layer).unwrap();
        assert_eq!(view.path, dir.join(CONFIG_FILE_NAME));
        assert!(view.value().is_some_and(declares_company));
        assert!(!dir.join(COMPANY_DOCUMENT).exists());
        assert_eq!(
            std::fs::read_to_string(dir.join(CONFIG_FILE_NAME)).unwrap(),
            LEGACY
        );
    }

    #[test]
    fn a_writing_read_converts_the_legacy_document() {
        let (_root, home, dir) = home_with(&[(CONFIG_FILE_NAME, LEGACY)]);
        let layer = super::super::Layer::Company(acme());
        let value = super::super::_layer::read_usable(&home, &layer)
            .unwrap()
            .unwrap();
        assert_eq!(value, serde_yaml_ng::from_str::<Value>(LEGACY).unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.join(COMPANY_DOCUMENT)).unwrap(),
            LEGACY
        );
        assert!(!dir.join(CONFIG_FILE_NAME).exists());
    }

    #[test]
    fn the_company_document_and_the_project_layer_beside_it_never_share_a_lock() {
        let (_root, home, dir) = home_with(&[]);
        assert_ne!(
            lock_path(&home, &dir.join(COMPANY_DOCUMENT)),
            lock_path(&home, &dir.join(CONFIG_FILE_NAME))
        );
    }

    #[test]
    fn an_ignore_file_that_is_a_link_is_refused_and_nothing_is_read_through_it() {
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secrets.env");
        let secret_bytes = "GITHUB_TOKEN=machine-secret-4f1e\n";
        std::fs::write(&secret, secret_bytes).unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(&secret, dir.path().join(".gitignore")).unwrap();
        let error = write_company_settings(dir.path(), &acme()).unwrap_err();
        assert!(matches!(
            error,
            ConfigError::LinkedIgnoreFile { ref path } if path == &dir.path().join(".gitignore")
        ));
        assert_eq!(std::fs::read_to_string(&secret).unwrap(), secret_bytes);
        assert!(
            std::fs::symlink_metadata(dir.path().join(".gitignore"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        // Nothing was written into the tree: it holds the link alone.
        let held: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(held, [std::ffi::OsString::from(".gitignore")]);
    }

    #[test]
    fn writing_settings_never_replaces_a_document_the_home_holds() {
        let dir = tempfile::tempdir().unwrap();
        let held = "company: acme\nmine: 1\n";
        std::fs::write(dir.path().join(COMPANY_DOCUMENT), held).unwrap();
        assert!(!write_company_settings(dir.path(), &acme()).unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(COMPANY_DOCUMENT)).unwrap(),
            held
        );
    }

    #[test]
    fn symlinked_company_yaml_and_legacy_instrukt_yaml_are_refused() {
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("outside.yaml");
        std::fs::write(&target, LEGACY).unwrap();

        // Symlinked company.yaml is refused.
        let (_root, home, dir) = home_with(&[]);
        std::os::unix::fs::symlink(&target, dir.join(COMPANY_DOCUMENT)).unwrap();
        assert!(matches!(
            resolve(&home, &acme()),
            Err(ConfigError::LinkedCompanyDocument { .. })
        ));

        // Symlinked legacy instrukt.yaml beside regular company.yaml is NOT swallowed.
        let (_root2, home2, dir2) = home_with(&[(COMPANY_DOCUMENT, COMPANY_YAML)]);
        std::os::unix::fs::symlink(&target, dir2.join(CONFIG_FILE_NAME)).unwrap();
        assert!(matches!(
            resolve(&home2, &acme()),
            Err(ConfigError::LinkedCompanyDocument { .. })
        ));

        // Escaping symlinked company folder is refused by convert().
        let root3 = tempfile::tempdir().unwrap();
        let home3 = Home::at_user_home(root3.path().to_path_buf()).for_writing();
        std::fs::create_dir_all(home3.companies_dir()).unwrap();
        let escaped_dir = outside.path().join("escaped-acme");
        std::fs::create_dir_all(&escaped_dir).unwrap();
        std::fs::write(escaped_dir.join(CONFIG_FILE_NAME), LEGACY).unwrap();
        std::os::unix::fs::symlink(&escaped_dir, home3.company_dir(&acme())).unwrap();
        assert!(matches!(
            convert(&home3, &acme()),
            Err(ConfigError::Fs(crate::fsx::FsError::Unconfined { .. }))
        ));
        assert!(!escaped_dir.join(COMPANY_DOCUMENT).exists());
    }
}
