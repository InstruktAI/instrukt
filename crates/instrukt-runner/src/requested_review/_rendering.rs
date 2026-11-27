//! The requested reviewer's assignment
//! (`project/instrukt/spec/feature/runner/requested-review`, The reviewer's
//! assignment): one fixed wording rendered from the admitted coordinates. It
//! holds nothing the requester composed: no requester identity, no timestamp,
//! no other envelope.

use instrukt_core::config::REVIEW_PROCEDURE;
use instrukt_core::ids::ProjectSlug;

use super::_admission::Coordinates;

/// A rendering and its SHA-256 digest: the same coordinates always render
/// the same text. `legacy_digest` matches pre-retirement asks stored with the
/// `[Instrukt Assignment]` wrapper so in-flight retries remain recoverable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Rendering {
    pub text: String,
    pub digest: String,
    pub legacy_digest: String,
}

/// Renders the assignment for `coordinates` in `project`: the procedure,
/// the role and the project, then each coordinate under its name in the
/// Coordinates table's order and the source pointers in the order given.
pub(super) fn render(project: &ProjectSlug, coordinates: &Coordinates) -> Rendering {
    use sha2::{Digest, Sha256};
    let mut body = format!(
        "Run `instrukt docs get {REVIEW_PROCEDURE} --closure` and execute that procedure for the one-off worktree review it describes.\nrole: {}\nproject: {project}\n",
        instrukt_core::config::REVIEW_ROLE
    );
    for (name, value) in coordinates.named() {
        body.push_str(&format!("{name}: {value}\n"));
    }
    let text = body.trim_end_matches('\n').to_owned();
    let digest = format!("{:x}", Sha256::digest(text.as_bytes()));
    let legacy = format!("[Instrukt Assignment]\n{text}\n[/Instrukt Assignment]");
    let legacy_digest = format!("{:x}", Sha256::digest(legacy.as_bytes()));
    Rendering {
        text,
        digest,
        legacy_digest,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "test: a failed step fails the test")]

    use instrukt_core::ids::ProjectSlug;

    use super::{Coordinates, render};
    use crate::requested_review::RawCoordinates;

    #[test]
    fn the_same_coordinates_render_the_same_assignment_in_the_fixed_order() {
        let head = "0123456789abcdef0123456789abcdef01234567";
        let base = "89abcdef0123456789abcdef0123456789abcdef";
        let coordinates = Coordinates::parse(&RawCoordinates {
            worktree: "/work/demo-feature".into(),
            branch: "feature/one".into(),
            head: head.into(),
            base: base.into(),
            report: "/reports/one.md".into(),
            prior_report: Some("/reports/zero.md".into()),
            disposition: Some("/reports/dispositions.md".into()),
            sources: vec![
                "sdlc/policy/rust/core".into(),
                format!("instrukt@{base}:docs/x.md").into(),
            ],
        })
        .unwrap();
        let project = ProjectSlug::parse("demo").unwrap();

        let first = render(&project, &coordinates);
        let second = render(&project, &coordinates);

        assert_eq!(first, second);
        insta::assert_snapshot!("requested-review-assignment", first.text);
    }
}
