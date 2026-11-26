//! The owning organisation's value rule: `github.org` names a GitHub
//! organisation (`project/instrukt/spec/feature/configuration/company-homes`,
//! Generation). It is checked where it enters, a patch of the global layer,
//! and the kind of account is asked before the layer's write lock is taken.

use serde_yaml_ng::Value;

use super::_error::{ConfigError, Finding, FindingKind, Findings};
use super::_layer::{Layer, Revision, patched, read_layer};
use crate::home::Home;
use crate::interfaces::{AccountKind, GitHubAccounts};

/// The key the rule judges.
pub const GITHUB_ORG_KEY: &str = "github.org";

/// Refuses a patch of `layer` at `expected` that sets or changes
/// `github.org` to a value that does not name a GitHub organisation: an
/// empty one, a person's account, a login no account has, or any value while
/// `accounts` cannot be asked. Nothing is written by this check; a patch it
/// lets through applies under the layer's lock against `expected` as every
/// patch does. Only the global layer declares the key; a patch that leaves
/// it as it is, removes it, or is no longer at `expected` is left to the
/// patch itself.
pub fn admit_github_org(
    home: &Home,
    layer: &Layer,
    expected: &Revision,
    patch: &Value,
    accounts: Option<&dyn GitHubAccounts>,
) -> Result<(), ConfigError> {
    if *layer != Layer::Global {
        return Ok(());
    }
    let Some((path, doc)) = read_layer(home, layer)? else {
        return Ok(());
    };
    if &Revision::of(&doc.bytes) != expected {
        return Ok(());
    }
    let (before, next) = patched(layer, &path, &doc, patch)?;
    let declared = |value: &Value| {
        value
            .get("github")
            .and_then(|github| github.get("org"))
            .cloned()
    };
    let Some(Value::String(org)) = declared(&next) else {
        return Ok(());
    };
    if declared(&before).as_ref().and_then(Value::as_str) == Some(org.as_str()) {
        return Ok(());
    }
    let answer = (!org.trim().is_empty())
        .then(|| accounts.map(|accounts| accounts.account_kind(org.trim())));
    match refusal(org.trim(), answer) {
        None => Ok(()),
        Some(message) => Err(ConfigError::Invalid(Findings(vec![Finding {
            document: path,
            key: GITHUB_ORG_KEY.to_owned(),
            kind: FindingKind::InvalidValue,
            message,
        }]))),
    }
}

/// Why `org` cannot be the owning organisation, from what was asked: `None`
/// for an empty value, `Some(None)` when no account lookup is wired, and
/// the lookup's answer otherwise. `None` when it is an organisation.
fn refusal(
    org: &str,
    answer: Option<Option<Result<AccountKind, crate::interfaces::InterfaceError>>>,
) -> Option<String> {
    const NEEDED: &str =
        "github.org names the GitHub organisation that owns this computer's company repositories";
    match answer {
        None => Some(format!(
            "is empty; {NEEDED}, so give it the login of a GitHub organisation"
        )),
        Some(None) => Some(format!(
            "`{org}` cannot be checked: this command has no GitHub account lookup; {NEEDED}, and nothing was written"
        )),
        Some(Some(Ok(AccountKind::Organisation))) => None,
        Some(Some(Ok(AccountKind::Personal))) => Some(format!(
            "`{org}` is a personal GitHub account; {NEEDED}, so give it the login of a GitHub organisation"
        )),
        Some(Some(Ok(AccountKind::Absent))) => Some(format!(
            "`{org}` is no GitHub account; {NEEDED}, so give it the login of a GitHub organisation"
        )),
        Some(Some(Err(error))) => Some(format!(
            "GitHub cannot be asked whether `{org}` is an organisation ({error}); {NEEDED}, so nothing was written; try again once GitHub answers"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AccountKind, ConfigError, FindingKind, Findings, GITHUB_ORG_KEY, GitHubAccounts, Home,
        Layer, Revision, Value, admit_github_org,
    };
    use crate::interfaces::InterfaceError;

    struct Stub(Result<AccountKind, &'static str>);

    impl GitHubAccounts for Stub {
        fn account_kind(&self, _login: &str) -> Result<AccountKind, InterfaceError> {
            self.0.map_err(|message| InterfaceError {
                module: "todos",
                message: message.to_owned(),
            })
        }
    }

    fn machine(text: &str) -> (tempfile::TempDir, Home, Revision) {
        let root = tempfile::tempdir().unwrap();
        let home = Home::at_user_home(root.path().to_path_buf()).for_writing();
        std::fs::create_dir_all(home.global_config().parent().unwrap()).unwrap();
        std::fs::write(home.global_config(), text).unwrap();
        let revision = Revision::of(text.as_bytes());
        (root, home, revision)
    }

    const MACHINE: &str = "schema_version: 6\n";

    fn admit(
        home: &Home,
        revision: &Revision,
        patch: &str,
        accounts: Option<&dyn GitHubAccounts>,
    ) -> Result<(), ConfigError> {
        let patch: Value = serde_yaml_ng::from_str(patch).unwrap();
        admit_github_org(home, &Layer::Global, revision, &patch, accounts)
    }

    #[test]
    fn an_organisation_is_admitted() {
        let (_root, home, revision) = machine(MACHINE);
        admit(
            &home,
            &revision,
            "github: {org: homes}",
            Some(&Stub(Ok(AccountKind::Organisation))),
        )
        .unwrap();
    }

    #[test]
    fn a_personal_owner_is_refused_naming_the_key_and_nothing_is_written() {
        insta::assert_snapshot!(
            "refused_personal",
            refused("github: {org: ana}", Ok(AccountKind::Personal))
        );
    }

    #[test]
    fn an_owner_no_account_has_is_refused() {
        insta::assert_snapshot!(
            "refused_absent",
            refused("github: {org: nobody}", Ok(AccountKind::Absent))
        );
    }

    #[test]
    fn an_empty_owner_is_refused() {
        insta::assert_snapshot!(
            "refused_empty",
            refused("github: {org: ''}", Ok(AccountKind::Organisation))
        );
    }

    #[test]
    fn an_owner_github_cannot_be_asked_about_is_refused() {
        insta::assert_snapshot!(
            "refused_unanswered",
            refused("github: {org: homes}", Err("unreachable"))
        );
    }

    /// The one finding a patch `patch` of the machine layer is refused with,
    /// the lookup answering `kind`, checking that nothing was written.
    fn refused(patch: &str, kind: Result<AccountKind, &'static str>) -> String {
        let (_root, home, revision) = machine(MACHINE);
        let error = admit(&home, &revision, patch, Some(&Stub(kind))).unwrap_err();
        let ConfigError::Invalid(Findings(findings)) = &error else {
            panic!("{error}");
        };
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].key, GITHUB_ORG_KEY);
        assert_eq!(findings[0].kind, FindingKind::InvalidValue);
        assert_eq!(
            std::fs::read_to_string(home.global_config()).unwrap(),
            MACHINE
        );
        findings[0].message.clone()
    }

    #[test]
    fn without_a_lookup_a_new_owner_is_refused() {
        let (_root, home, revision) = machine(MACHINE);
        assert!(admit(&home, &revision, "github: {org: homes}", None).is_err());
    }

    #[test]
    fn a_patch_that_leaves_the_owner_or_removes_it_asks_nothing() {
        let declared = "schema_version: 6\ngithub: {org: homes}\n";
        let (_root, home, revision) = machine(declared);
        let refusing = Stub(Ok(AccountKind::Personal));
        admit(&home, &revision, "modules: {events: true}", Some(&refusing)).unwrap();
        admit(&home, &revision, "github: {org: homes}", Some(&refusing)).unwrap();
        admit(&home, &revision, "github: {org: null}", Some(&refusing)).unwrap();
    }

    #[test]
    fn a_machine_layer_without_an_owner_validates() {
        let (_root, home, _revision) = machine(MACHINE);
        assert_eq!(
            super::super::validate(&home.for_inspection(), &Layer::Global)
                .unwrap()
                .iter()
                .filter(|finding| finding.key == GITHUB_ORG_KEY)
                .count(),
            0
        );
    }
}
