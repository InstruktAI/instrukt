//! Delivering a verdict to its destinations and recognising the verdict
//! the reviewer bot already published (`github-apps`, Verdict publication).
//! Every read and write here is GitHub's; each record of it is one short
//! write to the publication store, never held across a request.

use instrukt_core::github::Bot;
use instrukt_core::home::Home;
use instrukt_core::ids::{CommitSha, ProjectSlug};
use jiff::Timestamp;
use serde_json::{Value, json};

use super::_publication::{
    Published, STATUS_CONTEXT, decided_last, description, parse_body, parse_description,
    parse_link, review_event, review_verdict, status_state, status_verdict,
};
use super::_published::{self, Delivery, DeliveryState, Destination, Recognised};
use super::ReviewError;
use crate::work::_github::{GitHub, GitHubError};

/// A verdict to deliver: what it is and the review body that carries it,
/// when this computer has one.
pub(super) struct Target {
    pub published: Published,
    pub body: Option<String>,
    /// A derived approval: its status is written first, and a pull request
    /// receives its review only once GitHub shows that status.
    pub status_first: bool,
}

/// One publication: where a verdict's destinations are and who may show it.
pub(super) struct Publication<'a> {
    pub home: &'a Home,
    pub gh: &'a GitHub,
    pub project: &'a ProjectSlug,
    pub repository: &'a str,
    pub reviewer: &'a Bot,
    pub now: Timestamp,
}

impl Publication<'_> {
    fn record(&self, commit: &CommitSha, delivery: &Delivery) -> Result<(), ReviewError> {
        let mut db = _published::open(self.home)?;
        db.write(|tx| _published::deliver(tx, self.project, commit, delivery))
    }

    fn delivery(
        &self,
        target: &Target,
        destination: Destination,
        state: DeliveryState,
        link: Option<String>,
        cause: Option<String>,
    ) -> Delivery {
        Delivery {
            destination,
            name: target.published.name.clone(),
            moment: target.published.moment,
            state,
            link,
            cause,
            updated_at: self.now,
        }
    }

    /// The reviewer bot's latest review of exactly `commit` on pull request
    /// `number`, the one GitHub lists last there.
    fn latest_review(&self, number: u64, commit: &CommitSha) -> Result<Option<Value>, GitHubError> {
        Ok(self
            .gh
            .list(&format!("/pulls/{number}/reviews"))?
            .into_iter()
            .rfind(|review| {
                self.reviewer.matches(&review["user"]) && review["commit_id"] == commit.as_str()
            }))
    }

    /// The reviewer bot's newest review status on `commit`.
    fn newest_status(&self, commit: &CommitSha) -> Result<Option<Value>, GitHubError> {
        Ok(self
            .gh
            .list(&format!("/commits/{commit}/statuses"))?
            .into_iter()
            .find(|status| {
                status["context"] == STATUS_CONTEXT && self.reviewer.matches(&status["creator"])
            }))
    }

    /// The verdict a review carries when it counts: by the reviewer bot, of
    /// exactly `commit`, approving or change-requesting, its body headed by
    /// a name and moment.
    fn review_counts(&self, review: &Value, commit: &CommitSha) -> Option<Published> {
        if !self.reviewer.matches(&review["user"]) || review["commit_id"] != commit.as_str() {
            return None;
        }
        let verdict = review_verdict(review["state"].as_str()?)?;
        let (name, moment) = parse_body(review["body"].as_str()?)?;
        Some(Published {
            verdict,
            name,
            moment,
        })
    }

    /// Delivers `target` to `commit`'s status and to a review on each of
    /// `pulls`, the eligible pull requests whose head it is. Reviews are
    /// written before the status, which links the lowest-numbered one.
    pub(super) fn deliver(
        &self,
        commit: &CommitSha,
        target: &Target,
        pulls: &[u64],
    ) -> Result<(), ReviewError> {
        let held = match _published::reader(self.home)? {
            Some(conn) => _published::deliveries(&conn, self.project, commit)?,
            None => Vec::new(),
        };
        let settled = |destination: &Destination| {
            held.iter().find(|d| {
                &d.destination == destination
                    && d.name == target.published.name
                    && d.moment == target.published.moment
                    && d.state != DeliveryState::Outstanding
            })
        };
        let mut links: Vec<(u64, String)> = Vec::new();
        let mut pulls = if target.status_first && settled(&Destination::Status).is_none() {
            Vec::new()
        } else {
            pulls.to_vec()
        };
        pulls.sort_unstable();
        for number in pulls {
            let destination = Destination::Pull(number);
            if let Some(done) = settled(&destination) {
                if let Some(link) = &done.link {
                    links.push((number, link.clone()));
                }
                continue;
            }
            let Some(body) = &target.body else {
                self.record(
                    commit,
                    &self.delivery(
                        target,
                        destination,
                        DeliveryState::Outstanding,
                        None,
                        Some("owed and not deliverable from this computer: no review carrying this verdict was read".into()),
                    ),
                )?;
                continue;
            };
            let delivered = self.latest_review(number, commit).and_then(|latest| {
                match latest.filter(|review| {
                    self.review_counts(review, commit).as_ref() == Some(&target.published)
                }) {
                    Some(review) => Ok(review),
                    None => self.gh.post(
                        &format!("/pulls/{number}/reviews"),
                        &json!({
                            "commit_id": commit.as_str(),
                            "event": review_event(target.published.verdict),
                            "body": body,
                        }),
                    ),
                }
            });
            match delivered {
                Ok(review) => {
                    let link = review["html_url"].as_str().map(str::to_owned);
                    if let Some(link) = &link {
                        links.push((number, link.clone()));
                    }
                    self.record(
                        commit,
                        &self.delivery(target, destination, DeliveryState::Shown, link, None),
                    )?;
                    tracing::info!(event = "review_publication_written", project = %self.project, commit = %commit, pull = number);
                }
                Err(error) => {
                    let state = failure_state(&error, false);
                    self.record(
                        commit,
                        &self.delivery(target, destination, state, None, Some(error.to_string())),
                    )?;
                    tracing::warn!(event = "review_publication_failed", project = %self.project, commit = %commit, pull = number, state = state.as_str(), error = %error);
                    if state == DeliveryState::Outstanding {
                        return Ok(());
                    }
                }
            }
        }
        let link = links
            .into_iter()
            .min_by_key(|(number, _)| *number)
            .map(|(_, link)| link);
        if settled(&Destination::Status).is_some_and(|done| done.link == link || link.is_none()) {
            return Ok(());
        }
        let wanted_link = link.as_deref().and_then(|l| parse_link(l, self.repository));
        let written = self.newest_status(commit).and_then(|newest| {
            let shows = newest.as_ref().is_some_and(|status| {
                status_verdict(status["state"].as_str().unwrap_or_default())
                    == Some(target.published.verdict)
                    && status["description"].as_str().and_then(parse_description)
                        == Some((target.published.name.clone(), target.published.moment))
                    && (wanted_link.is_none()
                        || status["target_url"]
                            .as_str()
                            .and_then(|l| parse_link(l, self.repository))
                            == wanted_link)
            });
            if shows {
                return Ok(());
            }
            let mut status = json!({
                "state": status_state(target.published.verdict),
                "context": STATUS_CONTEXT,
                "description": description(&target.published.name, target.published.moment),
            });
            if let Some(link) = &link {
                status["target_url"] = json!(link);
            }
            self.gh
                .post(&format!("/statuses/{commit}"), &status)
                .map(drop)
        });
        match written {
            Ok(()) => {
                self.record(
                    commit,
                    &self.delivery(
                        target,
                        Destination::Status,
                        DeliveryState::Shown,
                        link,
                        None,
                    ),
                )?;
                tracing::info!(event = "review_status_written", project = %self.project, commit = %commit);
            }
            Err(error) => {
                let state = failure_state(&error, true);
                self.record(
                    commit,
                    &self.delivery(
                        target,
                        Destination::Status,
                        state,
                        None,
                        Some(error.to_string()),
                    ),
                )?;
                tracing::warn!(event = "review_status_failed", project = %self.project, commit = %commit, state = state.as_str(), error = %error);
            }
        }
        Ok(())
    }

    /// The verdict the reviewer bot published last for `commit`, from its
    /// newest status, the review that status links and its latest review of
    /// the commit on each of `pulls`; none when none of them counts.
    pub(super) fn recognise(
        &self,
        commit: &CommitSha,
        pulls: &[u64],
    ) -> Result<Option<Recognised>, GitHubError> {
        let mut found: Vec<(Published, Option<String>)> = Vec::new();
        if let Some(status) = self.newest_status(commit)? {
            if let (Some(verdict), Some((name, moment))) = (
                status_verdict(status["state"].as_str().unwrap_or_default()),
                status["description"].as_str().and_then(parse_description),
            ) {
                found.push((
                    Published {
                        verdict,
                        name,
                        moment,
                    },
                    None,
                ));
            }
            if let Some((number, id)) = status["target_url"]
                .as_str()
                .and_then(|link| parse_link(link, self.repository))
            {
                match self.gh.read(&format!("/pulls/{number}/reviews/{id}")) {
                    Ok(review) => {
                        if let Some(published) = self.review_counts(&review, commit) {
                            found.push((published, review["body"].as_str().map(str::to_owned)));
                        }
                    }
                    Err(GitHubError::NotFound { .. }) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        for number in pulls {
            if let Some(review) = self.latest_review(*number, commit)?
                && let Some(published) = self.review_counts(&review, commit)
            {
                found.push((published, review["body"].as_str().map(str::to_owned)));
            }
        }
        let published: Vec<Published> = found.iter().map(|(p, _)| p.clone()).collect();
        let Some(last) = decided_last(&published).cloned() else {
            return Ok(None);
        };
        let body = found
            .into_iter()
            .find(|(p, body)| p == &last && body.is_some())
            .and_then(|(_, body)| body);
        Ok(Some(Recognised {
            published: last,
            body,
            recognised_at: self.now,
        }))
    }
}

/// What a failed delivery leaves: outstanding while GitHub cannot be
/// reached or has not the commit yet, refused otherwise.
fn failure_state(error: &GitHubError, status: bool) -> DeliveryState {
    match error {
        GitHubError::Status { status: 422, .. } if status => DeliveryState::Outstanding,
        GitHubError::Status { status, .. } if *status >= 500 => DeliveryState::Outstanding,
        error if error.is_reach() => DeliveryState::Outstanding,
        _ => DeliveryState::Refused,
    }
}
