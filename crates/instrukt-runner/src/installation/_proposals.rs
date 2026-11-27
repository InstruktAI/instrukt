//! Knowledge proposals at the installation's entry and in its status
//! (`project/instrukt/spec/feature/runner/pull-request-review`, Knowledge
//! proposals in a company home): the proposal pull-request credential's
//! readiness asked for each company home admitted for automatic review, and
//! the unestablished entries and proposal-store conditions listed with their
//! doors, derived through the company's lookups each time.

/// The company homes among `candidate`'s projects admitted for automatic
/// review, with the GitHub repository each resolves to or why it resolves
/// to none.
fn company_homes(
    candidate: &instrukt_core::config::Candidate,
) -> Vec<(
    &instrukt_core::config::AdmittedProject,
    Result<String, String>,
)> {
    candidate
        .generation
        .projects()
        .iter()
        .filter(|project| project.review)
        .filter(|project| {
            instrukt_core::publication::is_company_repository(&project.checkout).unwrap_or(false)
        })
        .map(|project| {
            let repository = project
                .config
                .github_api_url()
                .map_err(|error| error.to_string())
                .and_then(|api_url| {
                    instrukt_core::github::repository(&project.config, api_url, &project.checkout)
                        .map_err(|error| error.to_string())
                });
            (project, repository)
        })
        .collect()
}

/// The proposal pull-request credential's readiness for each company home
/// admitted for automatic review: an unmet answer, or one GitHub cannot
/// give, is a finding with its repair door.
pub(super) fn readiness(
    request: &super::Request<'_>,
    candidate: &instrukt_core::config::Candidate,
    report: &mut super::Report,
) {
    use instrukt_core::interfaces::ReviewReadiness;
    for (project, repository) in company_homes(candidate) {
        let label = format!(
            "{} `{}` (knowledge proposals)",
            project
                .checkout
                .join(instrukt_core::config::CONFIG_FILE_NAME)
                .display(),
            instrukt_core::config::REVIEW_AUTOMATIC_KEY
        );
        let repository = match repository {
            Ok(repository) => repository,
            Err(error) => {
                report.finding(label, error);
                continue;
            }
        };
        match request.github.proposal_readiness(&repository, request.now) {
            ReviewReadiness::Met { .. } => {}
            ReviewReadiness::Unmet(message) => report.finding(
                label,
                format!("{message}; until then knowledge proposals' pull requests are not opened"),
            ),
            ReviewReadiness::CannotAsk(cause) => report.finding(
                label,
                format!("GitHub cannot be asked whether the proposal pull-request identity is ready on {repository}: {cause}"),
            ),
        }
    }
}

/// The knowledge-proposal conditions of each company home admitted for
/// automatic review, derived through the company's lookups each time:
/// unestablished entries a remote-tracking branch holds and a proposal
/// store that is not usable, each a finding with its door.
pub(super) fn findings(
    request: &super::Request<'_>,
    candidate: &instrukt_core::config::Candidate,
    report: &mut super::Report,
) {
    use crate::interfaces::ProposalStoreCondition;
    let Some(company) = request.company else {
        return;
    };
    for (project, _) in company_homes(candidate) {
        let root = &project.checkout;
        let label = format!("{} (knowledge proposals)", root.display());
        match company.unestablished_proposals(request.home, root) {
            Ok(entries) => {
                for entry in entries {
                    report.finding(
                        label.clone(),
                        format!(
                            "commit {} was recorded without an established speaker ({}), so it is not admitted and waits for a person; a person merges it, or the administrator commits the change again in their own turn",
                            entry.commit, entry.cause
                        ),
                    );
                }
            }
            Err(error) => report.finding(label.clone(), error.0),
        }
        let condition = match company.proposal_store(request.home, root) {
            Ok(condition) => condition,
            Err(error) => {
                report.finding(label.clone(), error.0);
                continue;
            }
        };
        let message = match condition {
            ProposalStoreCondition::Usable => continue,
            ProposalStoreCondition::Unreadable(cause) => format!(
                "the proposal store cannot be read ({cause}); administrators' commits are not recorded and wait for a person until it is repaired or moved aside"
            ),
            ProposalStoreCondition::Older => "the proposal store is of an older shape; the next recording migrates it".to_owned(),
            ProposalStoreCondition::UnknownShape => format!(
                "the proposal store is of a shape newer than this instrukt knows; upgrade with `{}`",
                instrukt_core::INSTALLER_COMMAND
            ),
            ProposalStoreCondition::Unwritable => "the proposal store or its folder refuses a writer; administrators' commits are not recorded and wait for a person until its permissions are repaired".to_owned(),
        };
        report.finding(label.clone(), message);
    }
}
