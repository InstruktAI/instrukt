//! Validation findings and their stable codes.

use std::fmt;
use std::path::PathBuf;

use serde::{Serialize, Serializer};

macro_rules! codes {
    ($($(#[$meta:meta])* $variant:ident => $code:literal,)*) => {
        /// A stable finding code. The text form is the contract: it never
        /// changes for an existing finding.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum FindingCode {
            $($(#[$meta])* $variant,)*
        }

        impl FindingCode {
            pub fn as_str(self) -> &'static str {
                match self {
                    $(FindingCode::$variant => $code,)*
                }
            }
        }
    };
}

codes! {
    // Identity
    /// The taxonomy directory is not one of the seven.
    UnknownTaxonomy => "identity-unknown-taxonomy",
    /// A published file without a domain segment, or a reserved one.
    MissingDomain => "identity-missing-domain",
    /// A local snippet in a repository that declares no `project`.
    UndeclaredProject => "identity-undeclared-project",
    /// A Markdown file below `docs/` outside every place a document lives.
    Unplaced => "identity-unplaced",
    /// `id`, `type`, `scope` or `domain` authored in frontmatter.
    ClassificationAuthored => "identity-classification-authored",

    // Frontmatter
    /// The frontmatter block is not a YAML mapping.
    FrontmatterInvalid => "frontmatter-invalid",
    DescriptionMissing => "frontmatter-description-missing",
    VisibilityInvalid => "frontmatter-visibility-invalid",
    EvalInvalid => "frontmatter-eval-invalid",
    /// A procedure-only field on another type.
    ProcedureFieldMisplaced => "frontmatter-procedure-field-misplaced",
    /// `subscriptions` and `activation_trigger` both declared.
    ActivationConflict => "frontmatter-activation-conflict",
    ActivationTriggerInvalid => "frontmatter-activation-trigger-invalid",
    SubscriptionsInvalid => "frontmatter-subscriptions-invalid",
    /// A procedure declaring `subscriptions` without a `role`.
    SubscriptionsWithoutRole => "frontmatter-subscriptions-without-role",
    /// `event_delivery` or `interruptible` without `subscriptions`.
    DeliverySettingMisplaced => "frontmatter-delivery-setting-misplaced",
    DeliverySettingInvalid => "frontmatter-delivery-setting-invalid",
    RoleInvalid => "frontmatter-role-invalid",
    ParametersWithoutRole => "frontmatter-parameters-without-role",
    ParametersInvalid => "frontmatter-parameters-invalid",
    DispatchWithoutRole => "frontmatter-dispatch-without-role",
    DispatchInvalid => "frontmatter-dispatch-invalid",
    /// An `adr` without a well-formed `date`.
    AdrDateInvalid => "frontmatter-adr-date-invalid",
    /// An `adr` without a `number` matching its file name prefix.
    AdrNumberInvalid => "frontmatter-adr-number-invalid",

    // Procedures
    /// With todos on, a role the selected flavor does not declare.
    RoleUndeclared => "procedure-role-undeclared",
    /// Two procedures declare the same role.
    RoleShared => "procedure-role-shared",
    /// An entry procedure requires a sub-procedure of its own group.
    SubprocedureRequired => "procedure-subprocedure-required",

    // Structure
    H1Missing => "structure-h1-missing",
    H1NotFirst => "structure-h1-not-first",
    H1TypeMismatch => "structure-h1-type-mismatch",
    SectionMissing => "structure-section-missing",
    SectionUnknown => "structure-section-unknown",
    SeeAlsoNotLast => "structure-see-also-not-last",
    RequiredReadsNotFirst => "structure-required-reads-not-first",

    // References
    ReferenceOutsideRequiredReads => "reference-outside-required-reads",
    RequiredReadsLevel => "reference-required-reads-not-h2",
    ReferenceForm => "reference-form-invalid",
    ReferenceUnresolved => "reference-unresolved",
    /// A published snippet referencing local material.
    ReferenceLocalFromPublished => "reference-local-from-published",

    // Baselines
    BaselineLineInvalid => "baseline-line-invalid",
    BaselineOrder => "baseline-out-of-order",
    /// A manifest naming a snippet its domain's bundle already carries.
    BaselineCarried => "baseline-already-carried",

    // Pending
    DeliveredByInvalid => "pending-delivered-by-invalid",
    PendingSlugInvalid => "pending-slug-invalid",
    /// With todos on, a slug that names no existing todo.
    PendingSlugUnknown => "pending-slug-unknown",
    MarkerInline => "pending-marker-inline",
    MarkerNested => "pending-marker-nested",
    MarkerUnbalanced => "pending-marker-unbalanced",
    MarkerOverlapping => "pending-marker-overlapping",

    // Third-party
    SourceInvalid => "third-party-source-invalid",
}

impl fmt::Display for FindingCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for FindingCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// One way a file breaks the snippet contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// The file, relative to the repository root.
    pub path: PathBuf,
    pub code: FindingCode,
    /// The offending value, where there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Other files a cross-snippet finding involves; a scoped run keeps
    /// the finding when any of them is in scope.
    #[serde(skip)]
    pub(super) involves: Vec<PathBuf>,
}

impl Finding {
    pub(super) fn new(path: impl Into<PathBuf>, code: FindingCode, value: Option<String>) -> Self {
        Self {
            path: path.into(),
            code,
            value,
            involves: Vec::new(),
        }
    }
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.code)?;
        if let Some(value) = &self.value {
            write!(f, ": {value}")?;
        }
        Ok(())
    }
}
