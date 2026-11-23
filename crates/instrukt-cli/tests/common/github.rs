//! A fake GitHub: a local HTTP server standing in for GitHub's REST API at
//! the network line, the one fake the testing policy allows
//! (`docs/project/policy/testing.md`). instrukt's client stays real; the
//! test points it here through the global layer's `github.api_url`.
//!
//! It fakes GitHub's observable HTTP contract, not instrukt: resources
//! behave as GitHub documents them, every `GET` answers with an `ETag` and
//! honours `If-None-Match` with `304`, list endpoints paginate with
//! `per_page`/`page` and a `Link` header. Served (all under
//! `/repos/{owner}/{repo}`):
//!
//! - repository `GET /`, `GET /branches/{branch}`;
//! - issues `GET|POST /issues`, `GET|PATCH /issues/{n}` (pull requests
//!   share the number space and appear in the issue list with a
//!   `pull_request` key, as on GitHub);
//! - comments `GET|POST /issues/{n}/comments`,
//!   `GET|PATCH|DELETE /issues/comments/{id}`;
//! - labels `GET|POST /labels`, `GET|PATCH|DELETE /labels/{name}`,
//!   `GET|POST|PUT|DELETE /issues/{n}/labels`,
//!   `DELETE /issues/{n}/labels/{name}` (labels used on an issue are
//!   created on the fly, as on GitHub);
//! - sub-issues `GET|POST /issues/{n}/sub_issues`,
//!   `DELETE /issues/{n}/sub_issue`, `GET /issues/{n}/parent`;
//! - dependencies `GET|POST /issues/{n}/dependencies/blocked_by`,
//!   `DELETE /issues/{n}/dependencies/blocked_by/{issue_id}`;
//! - issue artifacts (preview API) `GET|POST /issues/{n}/artifacts`,
//!   `GET|PATCH|DELETE /issues/{n}/artifacts/{a}`, the read answering
//!   `{artifact, versions}` newest first;
//! - pull requests `GET|POST /pulls`, `GET|PATCH /pulls/{n}`,
//!   `GET|POST /pulls/{n}/reviews` (creation refuses with `422` while the
//!   head branch is not on the fake);
//! - CI `GET /commits/{ref}/status` (combined status),
//!   `GET /commits/{ref}/check-runs`, `POST /statuses/{sha}`.
//!
//! A revision conflict is what GitHub shows: another writer changed the
//! resource. GitHub's REST issue and comment writes honour no `If-Match`,
//! so there is no `412`; a test simulates the other computer with
//! [`FakeGitHub::edit_issue`] or [`FakeGitHub::edit_comment`], and the
//! client meets the newer revision when it reads. Unreachability is
//! [`FakeGitHub::stop`] until [`FakeGitHub::resume`]: the address stays the
//! fake's own, an exchange still open when `stop` returns ends without the
//! rest of its response, and an exchange on a connection made while
//! stopped ends without any byte of one, so the client meets a failed
//! exchange without waiting. Response bytes the client already received
//! stay received. The fake listens on its address for its whole life and
//! relays each connection to its HTTP server on a private address while it
//! answers; a failure of that transport fails the test when the fake is
//! dropped, and a [`FakeGitHub::resume`] once nothing accepts on the
//! address fails at once with it. Rate limiting and other failures are
//! one-shot answers queued with [`FakeGitHub::respond_once`] or
//! [`FakeGitHub::rate_limit_next`].

mod _apps;
mod _fake;
mod _front;
mod _model;
mod _projects;
mod _routes;
mod _serve;

#[allow(
    unused_imports,
    reason = "each test uses the part of the fake it needs"
)]
pub use _apps::{configure_apps, sync_auth_clock};
#[allow(
    unused_imports,
    reason = "each test uses the part of the fake it needs"
)]
pub use _fake::{
    CannedResponse, CheckRun, Conclusion, FakeGitHub, IssueSeed, IssueView, RecordedRequest,
    RequestPause, StatusState,
};
