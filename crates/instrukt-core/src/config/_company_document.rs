//! Where a company home keeps its settings: `company.yaml`, beside the
//! project layer its repository commits
//! (`project/instrukt/spec/feature/configuration/company-homes`, The home).
//! A home made before keeps them in a root `instrukt.yaml` declaring
//! `company`; that document is read where it stands and converted to
//! `company.yaml`, every byte kept, by the first path that persists a
//! conversion.

use std::path::{Path, PathBuf};

use serde_yaml_ng::{Mapping, Value};

use super::_error::ConfigError;
use super::_layer::{CONFIG_FILE_NAME, WRITE_LOCK_TIMEOUT, lock_path, write};
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
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ConfigError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn is_company_document(path: &Path) -> Result<bool, ConfigError> {
    Ok(super::_layer::read(path)?.is_some_and(|document| declares_company(&document.value)))
}

/// Converts the legacy settings document of company `name`'s home to
/// `company.yaml`: the ignore line first, so an interrupted conversion never
/// exposes the document to git, then the rename under the company document's
/// write lock, every byte kept. Nothing happens without a legacy document;
/// both present is refused and neither file changes.
pub(crate) fn convert(home: &Home, name: &ProjectSlug) -> Result<(), ConfigError> {
    let dir = home.company_dir(name);
    match resolve_in(&dir)? {
        CompanyDocument::Legacy(_) => {}
        CompanyDocument::Both { company, legacy } => {
            return Err(ConfigError::BothCompanyDocuments { company, legacy });
        }
        CompanyDocument::Present(_) | CompanyDocument::Absent => return Ok(()),
    }
    ignore_local_material(&dir)?;
    let company = dir.join(COMPANY_DOCUMENT);
    let _lock = LockGuard::acquire(&lock_path(home, &company), WRITE_LOCK_TIMEOUT)?;
    // Another writer may have converted it while this one waited.
    let CompanyDocument::Legacy(legacy) = resolve_in(&dir)? else {
        return Ok(());
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

#[cfg(test)]
mod tests {
    use super::{
        COMPANY_DOCUMENT, CONFIG_FILE_NAME, CompanyDocument, ConfigError, Home, PathBuf,
        ProjectSlug, convert, declares_company, lock_path, resolve, write_company_settings,
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
    const PROJECT_LAYER: &str =
        "schema_version: 6\nproject: company-acme\ncompany_repository: true\n";

    #[test]
    fn company_yaml_beside_a_project_layer_is_present() {
        let (_root, home, dir) = home_with(&[
            (COMPANY_DOCUMENT, "company: acme\n"),
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
        let (_root, home, _dir) = home_with(&[]);
        assert_eq!(resolve(&home, &acme()).unwrap(), CompanyDocument::Absent);
    }

    #[test]
    fn both_present_is_named_and_converting_changes_neither_file() {
        let (_root, home, dir) = home_with(&[
            (COMPANY_DOCUMENT, "company: acme\n"),
            (CONFIG_FILE_NAME, LEGACY),
        ]);
        let both = CompanyDocument::Both {
            company: dir.join(COMPANY_DOCUMENT),
            legacy: dir.join(CONFIG_FILE_NAME),
        };
        assert_eq!(resolve(&home, &acme()).unwrap(), both);
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
            "company: acme\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join(CONFIG_FILE_NAME)).unwrap(),
            LEGACY
        );
        assert!(!dir.join(".gitignore").exists());
    }

    #[test]
    fn converting_a_legacy_document_keeps_every_byte_and_ignores_it_first() {
        let legacy = format!("# my settings\n{LEGACY}");
        let (_root, home, dir) = home_with(&[
            (CONFIG_FILE_NAME, &legacy),
            (".gitignore", "/instrukt.yaml\n"),
        ]);
        convert(&home, &acme()).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(COMPANY_DOCUMENT)).unwrap(),
            legacy
        );
        assert!(!dir.join(CONFIG_FILE_NAME).exists());
        // The earlier line is kept and the missing ones follow it.
        insta::assert_snapshot!(
            "converted_ignore_file",
            std::fs::read_to_string(dir.join(".gitignore")).unwrap()
        );
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
        assert!(value.get("members").is_some());
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
    fn writing_settings_ignores_them_and_never_replaces_a_document() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write_company_settings(dir.path(), &acme()).unwrap());
        insta::assert_snapshot!(
            "written_settings",
            std::fs::read_to_string(dir.path().join(COMPANY_DOCUMENT)).unwrap()
        );
        insta::assert_snapshot!(
            "written_ignore_file",
            std::fs::read_to_string(dir.path().join(".gitignore")).unwrap()
        );
        std::fs::write(
            dir.path().join(COMPANY_DOCUMENT),
            "company: acme\nmine: 1\n",
        )
        .unwrap();
        assert!(!write_company_settings(dir.path(), &acme()).unwrap());
        assert_eq!(
            std::fs::read_to_string(dir.path().join(COMPANY_DOCUMENT)).unwrap(),
            "company: acme\nmine: 1\n"
        );
    }
}
