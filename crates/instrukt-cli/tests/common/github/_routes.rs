//! GitHub's REST endpoints, routed by method and path.

use serde_json::{Value, json};

use super::_apps::{app_login, auth_time};
use super::_fake::StatusState;
use super::_model::{
    Artifact, ArtifactVersion, CLIENT_ASSOCIATION, Label, Pull, REVIEW_BODY_LIMIT, Repo, State,
    Status, actor,
};
use super::_projects::{field_database_id, graphql};
use super::_serve::{Reply, decode, paginate, query_param};

pub(super) fn route(state: &mut State, method: &str, path: &str, query: &str, body: &str) -> Reply {
    if path == "/app" && method == "GET" {
        let id = match state.login.as_str() {
            "instruktai-author[bot]" => 2,
            "instruktai-reviewer[bot]" => 3,
            _ => 1,
        };
        return Reply::ok(json!({"id":id,"slug":app_login(id).trim_end_matches("[bot]")}));
    }
    if let Some(slug) = path.strip_prefix("/apps/") {
        let id = match slug {
            "instruktai" => 1,
            "instruktai-author" => 2,
            "instruktai-reviewer" => 3,
            _ => return Reply::not_found(),
        };
        return Reply::ok(json!({"id":id,"slug":slug}));
    }
    if let Some(login) = path.strip_prefix("/users/") {
        return if method == "GET" {
            let mut account = actor(login);
            if let Some(kind) = state.accounts.get(login) {
                account["type"] = kind.as_str().into();
            }
            Reply::ok(account)
        } else {
            Reply::not_found()
        };
    }
    if method == "POST"
        && path.starts_with("/app/installations/")
        && path.ends_with("/access_tokens")
    {
        let id = path
            .split('/')
            .nth(3)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(1);
        let input: Value = serde_json::from_str(body).unwrap();
        let expected = match state.login.as_str() {
            "instruktai-author[bot]" => 2,
            "instruktai-reviewer[bot]" => 3,
            _ => 1,
        };
        if id != expected {
            return Reply::not_found();
        }
        let expires = auth_time(&state.auth_clock)
            .checked_add(jiff::SignedDuration::from_secs(3600))
            .unwrap();
        let token = format!("app-token-{id}-{}", state.tokens.len() + 1);
        let held = state.installation_permissions(id);
        let exceeds =
            input["permissions"]
                .as_object()
                .into_iter()
                .flatten()
                .any(|(name, wanted)| {
                    held[name] != *wanted && !(wanted == "read" && held[name] == "write")
                });
        if exceeds {
            return Reply::invalid(
                "The permissions requested are not granted to this installation.",
            );
        }
        let has_repo_access = input["permissions"].as_object().is_some_and(|perms| {
            perms
                .keys()
                .any(|key| key != "organization_projects" && key != "members")
        });
        state
            .tokens
            .insert(token.clone(), (id, expires, has_repo_access));
        return Reply::created(
            json!({"token":token,"expires_at":expires.to_string(),"permissions":input["permissions"]}),
        );
    }
    if method == "GET" && path == "/installation/repositories" {
        let has_repo_access = state
            .active_token
            .as_deref()
            .and_then(|t| state.tokens.get(t))
            .is_none_or(|(_, _, has_repo)| *has_repo);
        let repositories: Vec<Value> = if has_repo_access {
            state.repos.values().map(Repo::json).collect()
        } else {
            Vec::new()
        };
        return Reply::ok(json!({"repositories": repositories}));
    }
    if method == "POST" && path == "/graphql" {
        let input: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        return graphql(state, &input);
    }
    let segments: Vec<String> = path
        .trim_start_matches('/')
        .split('/')
        .map(decode)
        .collect();
    let segs: Vec<&str> = segments.iter().map(String::as_str).collect();
    let input: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    if let ("POST", ["orgs", owner, "projectsV2", number, "views"]) = (method, segs.as_slice()) {
        let id = state.next_id();
        let Some(project) = state
            .projects
            .iter_mut()
            .find(|p| p.owner == *owner && Some(p.number) == number.parse().ok())
        else {
            return Reply::not_found();
        };
        let Some(columns) = input
            .get("vertical_group_by")
            .map_or(Some(Vec::new()), |v| v.as_array().cloned())
        else {
            return Reply::invalid("invalid columns");
        };
        if columns.iter().any(|id| {
            !project
                .fields
                .iter()
                .any(|f| Some(field_database_id(&f.id)) == id.as_u64())
        }) {
            return Reply::invalid("unknown column field");
        }
        let view = project.add_view(
            id,
            input["name"].clone(),
            input["layout"].clone(),
            input["filter"].as_str().unwrap_or(""),
            Value::Array(columns),
        );
        return Reply::created(view);
    }
    if let ("GET", ["orgs", org, "installation"]) = (method, segs.as_slice()) {
        return if state.org_installations.contains(*org) {
            Reply::ok(json!({"id":1,"app_id":1,"permissions":state.installation_permissions(1)}))
        } else {
            Reply::not_found()
        };
    }
    if let ("POST", ["orgs", org, "repos"]) = (method, segs.as_slice()) {
        return create_repo(state, org, &input);
    }
    let ["repos", owner, name, rest @ ..] = segs.as_slice() else {
        return Reply::not_found();
    };
    let full = format!("{owner}/{name}");
    if !state.repos.contains_key(&full) {
        return Reply::not_found();
    }
    let base = state.base.clone();
    let login = state.login.clone();
    let number = |text: &str| text.parse::<u64>().ok();
    match (method, rest) {
        ("GET", ["installation"]) => {
            let id = match state.login.as_str() {
                "instruktai-author[bot]" => 2,
                "instruktai-reviewer[bot]" => 3,
                _ => 1,
            };
            Reply::ok(json!({"id":id,"app_id":id,"permissions":state.installation_permissions(id)}))
        }
        ("GET", []) => Reply::ok(state.repo(&full).json()),
        ("PATCH", []) => {
            if let Some(branch) = input["default_branch"].as_str() {
                state.repo(&full).default_branch = branch.to_owned();
            }
            Reply::ok(state.repo(&full).json())
        }
        ("PUT", ["collaborators", invited]) => {
            let permission = input["permission"].as_str().unwrap_or("push").to_owned();
            state
                .repo(&full)
                .collaborators
                .insert((*invited).to_owned(), permission.clone());
            Reply::created(json!({"invitee": actor(invited), "permissions": permission}))
        }
        ("GET", ["collaborators", login, "permission"]) => {
            let allowed = state.repo(&full).comments.values().any(|c| {
                c.login == *login
                    && matches!(c.association.as_str(), "OWNER" | "MEMBER" | "COLLABORATOR")
            });
            Reply::ok(json!({"permission":if allowed{"write"}else{"read"}}))
        }
        ("GET", ["branches", branch @ ..]) => {
            let branch = branch.join("/");
            match state.repo(&full).branch_head(&branch) {
                Some(sha) => Reply::ok(json!({"name": branch, "commit": {"sha": sha}})),
                None => Reply::not_found(),
            }
        }
        ("GET", ["commits", sha, "status"]) => Reply::ok(state.repo(&full).combined_status(sha)),
        ("GET", ["commits", sha, "check-runs"]) => {
            Reply::ok(state.repo(&full).check_runs_json(sha))
        }
        ("GET", ["commits", sha, "statuses"]) => {
            let items = state.repo(&full).statuses_json(sha);
            paginate(&base, path, query, items)
        }
        ("POST", ["statuses", sha]) => {
            let Some(status) = input["state"].as_str().and_then(StatusState::parse) else {
                return Reply::invalid("state is required");
            };
            let id = state.next_id();
            let now = state.now();
            let status = Status {
                id,
                context: input["context"].as_str().unwrap_or("default").to_owned(),
                state: status,
                description: input["description"].as_str().map(str::to_owned),
                target_url: input["target_url"].as_str().map(str::to_owned),
                creator: login.clone(),
                updated_at: now,
            };
            let value = json!({"id": id, "context": status.context, "state": status.state.as_str(), "description": status.description, "target_url": status.target_url, "creator": actor(&login)});
            match state.repo(&full).post_status(sha, status) {
                Ok(()) => Reply::created(value),
                Err(message) => Reply::invalid(&message),
            }
        }
        ("GET", ["issues"]) => list_issues(state, &base, &full, path, query),
        ("POST", ["issues"]) => {
            let Some(title) = input["title"].as_str() else {
                return Reply::invalid("title is required");
            };
            let labels = string_list(&input["labels"]);
            let body = input["body"].as_str().unwrap_or("");
            let n = state.create_issue(&full, title, body, &labels, &login);
            Reply::created(
                state
                    .repo(&full)
                    .issue_json(&base, &full, n)
                    .unwrap_or_default(),
            )
        }
        ("GET", ["issues", "comments", id]) => match number(id).and_then(|id| {
            state
                .repo(&full)
                .comments
                .get(&id)
                .map(|c| c.json(&base, &full))
        }) {
            Some(value) => Reply::ok(value),
            None => Reply::not_found(),
        },
        ("PATCH", ["issues", "comments", id]) => {
            let now = state.now();
            let r = state.repo(&full);
            let Some(comment) = number(id).and_then(|id| r.comments.get_mut(&id)) else {
                return Reply::not_found();
            };
            if let Some(body) = input["body"].as_str() {
                comment.body = body.to_owned();
                comment.updated_at = now;
            }
            Reply::ok(comment.json(&base, &full))
        }
        ("DELETE", ["issues", "comments", id]) => {
            match number(id).and_then(|id| state.repo(&full).comments.remove(&id)) {
                Some(_) => Reply::no_content(),
                None => Reply::not_found(),
            }
        }
        (_, ["issues", n, tail @ ..]) => {
            let Some(n) = number(n).filter(|n| state.repo(&full).issues.contains_key(n)) else {
                return Reply::not_found();
            };
            issue_route(
                state, method, &base, &full, n, tail, path, query, &input, &login,
            )
        }
        ("GET", ["labels"]) => {
            let items = state.repo(&full).labels.values().map(Label::json).collect();
            paginate(&base, path, query, items)
        }
        ("POST", ["labels"]) => {
            let Some(name) = input["name"].as_str() else {
                return Reply::invalid("name is required");
            };
            let id = state.next_id();
            let r = state.repo(&full);
            if r.labels.contains_key(name) {
                return Reply::invalid("already_exists");
            }
            let label = Label {
                id,
                name: name.to_owned(),
                color: input["color"].as_str().unwrap_or("ededed").to_owned(),
                description: input["description"].as_str().map(str::to_owned),
            };
            let value = label.json();
            r.labels.insert(name.to_owned(), label);
            Reply::created(value)
        }
        ("GET", ["labels", label]) => match state.repo(&full).labels.get(*label) {
            Some(label) => Reply::ok(label.json()),
            None => Reply::not_found(),
        },
        ("PATCH", ["labels", label]) => {
            let r = state.repo(&full);
            let Some(mut existing) = r.labels.remove(*label) else {
                return Reply::not_found();
            };
            if let Some(new_name) = input["new_name"].as_str() {
                existing.name = new_name.to_owned();
                for issue in r.issues.values_mut() {
                    for l in &mut issue.labels {
                        if l == label {
                            *l = new_name.to_owned();
                        }
                    }
                }
            }
            if let Some(color) = input["color"].as_str() {
                existing.color = color.to_owned();
            }
            if let Some(description) = input["description"].as_str() {
                existing.description = Some(description.to_owned());
            }
            let value = existing.json();
            r.labels.insert(existing.name.clone(), existing);
            Reply::ok(value)
        }
        ("DELETE", ["labels", label]) => {
            let r = state.repo(&full);
            if r.labels.remove(*label).is_none() {
                return Reply::not_found();
            }
            for issue in r.issues.values_mut() {
                issue.labels.retain(|l| l != label);
            }
            Reply::no_content()
        }
        ("GET", ["pulls"]) => {
            let wanted = query_param(query, "state").unwrap_or("open");
            let head = query_param(query, "head").map(decode);
            let r = state.repo(&full);
            let ascending = query_param(query, "direction") == Some("asc");
            let mut numbers: Vec<u64> = r
                .issues
                .values()
                .filter(|i| i.pull.is_some())
                .filter(|i| wanted == "all" || i.state == wanted)
                .filter(|i| {
                    head.as_deref().is_none_or(|h| {
                        let branch = h.split_once(':').map_or(h, |(_, b)| b);
                        i.pull.as_ref().is_some_and(|p| p.head == branch)
                    })
                })
                .map(|i| i.number)
                .collect();
            // GitHub's default order is newest created first.
            if !ascending {
                numbers.reverse();
            }
            let items = numbers
                .into_iter()
                .filter_map(|n| r.pull_json(&base, &full, n))
                .collect();
            paginate(&base, path, query, items)
        }
        ("POST", ["pulls"]) => {
            let (Some(title), Some(head), Some(base_ref)) = (
                input["title"].as_str(),
                input["head"].as_str(),
                input["base"].as_str(),
            ) else {
                return Reply::invalid("title, head and base are required");
            };
            let head = head.split_once(':').map_or(head, |(_, b)| b).to_owned();
            if state.repo(&full).branch_head(&head).is_none() {
                return Reply::invalid(&format!("head branch {head} does not exist"));
            }
            let body = input["body"].as_str().unwrap_or("").to_owned();
            let n = state.create_issue(&full, title, &body, &[], &login);
            let r = state.repo(&full);
            if let Some(issue) = r.issues.get_mut(&n) {
                let mut pull = Pull::new(head, base_ref.to_owned());
                pull.draft = input["draft"] == json!(true);
                issue.pull = Some(pull);
            }
            Reply::created(r.pull_json(&base, &full, n).unwrap_or_default())
        }
        (_, ["pulls", n, tail @ ..]) => {
            let Some(n) = number(n).filter(|n| {
                state
                    .repo(&full)
                    .issues
                    .get(n)
                    .is_some_and(|i| i.pull.is_some())
            }) else {
                return Reply::not_found();
            };
            pull_route(
                state, method, &base, &full, n, tail, path, query, &input, &login,
            )
        }
        _ => Reply::not_found(),
    }
}

/// `POST /orgs/{org}/repos`: a private repository, empty, its bare remote
/// initialised where the test serves remotes; a name the organisation
/// already holds is GitHub's 422.
fn create_repo(state: &mut State, org: &str, input: &Value) -> Reply {
    let Some(name) = input["name"].as_str() else {
        return Reply::invalid("name is missing");
    };
    let full = format!("{org}/{name}");
    if state.repos.contains_key(&full) {
        return Reply::json(
            422,
            json!({"message":"Repository creation failed.","errors":[{"resource":"Repository","code":"custom","field":"name","message":"name already exists on this account"}]}),
        );
    }
    let id = state.next_id();
    let mut repo = Repo::new(id, org, name);
    // GitHub creates a public repository unless `private` is true.
    repo.public = input["private"] != true;
    let body = repo.json();
    state.repos.insert(full, repo);
    if let Some(remotes) = &state.remotes {
        let remote = remotes.join(org).join(format!("{name}.git"));
        std::fs::create_dir_all(&remote).unwrap();
        let initialised = std::process::Command::new("git")
            .args(["init", "--bare", "--quiet", "--initial-branch=main"])
            .arg(&remote)
            .status()
            .unwrap();
        assert!(initialised.success());
    }
    Reply::created(body)
}

fn string_list(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_str()
                        .or_else(|| item["name"].as_str())
                        .map(str::to_owned)
                })
                .collect()
        })
        .unwrap_or_default()
}

fn list_issues(state: &mut State, base: &str, full: &str, path: &str, query: &str) -> Reply {
    let wanted = query_param(query, "state").unwrap_or("open");
    let labels: Vec<String> = query_param(query, "labels")
        .map(|l| decode(l).split(',').map(str::to_owned).collect())
        .unwrap_or_default();
    let r = state.repo(full);
    let mut numbers: Vec<u64> = r
        .issues
        .values()
        .filter(|i| wanted == "all" || i.state == wanted)
        .filter(|i| labels.iter().all(|l| i.labels.contains(l)))
        .map(|i| i.number)
        .collect();
    // GitHub's default order: newest created first.
    numbers.reverse();
    let items = numbers
        .into_iter()
        .filter_map(|n| r.issue_json(base, full, n))
        .collect();
    paginate(base, path, query, items)
}

#[allow(
    clippy::too_many_arguments,
    reason = "one dispatch over the request's parts"
)]
fn issue_route(
    state: &mut State,
    method: &str,
    base: &str,
    full: &str,
    n: u64,
    tail: &[&str],
    path: &str,
    query: &str,
    input: &Value,
    login: &str,
) -> Reply {
    let issue_reply = |state: &mut State| {
        Reply::ok(
            state
                .repo(full)
                .issue_json(base, full, n)
                .unwrap_or_default(),
        )
    };
    match (method, tail) {
        ("GET", []) => issue_reply(state),
        ("PATCH", []) => {
            let now = state.now();
            let id_seed = state.next_id();
            let r = state.repo(full);
            if let Some(labels) = input.get("labels") {
                let labels = string_list(labels);
                r.ensure_labels(&labels, id_seed);
                if let Some(issue) = r.issues.get_mut(&n) {
                    issue.labels = labels;
                }
            }
            let Some(issue) = r.issues.get_mut(&n) else {
                return Reply::not_found();
            };
            if let Some(title) = input["title"].as_str() {
                issue.title = title.to_owned();
            }
            if let Some(body) = input["body"].as_str() {
                issue.body = body.to_owned();
            }
            if let Some(new_state) = input["state"].as_str() {
                if new_state != issue.state {
                    issue.closed_at = (new_state == "closed").then(|| now.clone());
                }
                issue.state = new_state.to_owned();
                issue.state_reason = match (new_state, input["state_reason"].as_str()) {
                    (_, Some(reason)) => Some(reason.to_owned()),
                    ("closed", None) => Some("completed".into()),
                    ("open", None) => Some("reopened".into()),
                    _ => None,
                };
            }
            issue.updated_at = now;
            issue_reply(state)
        }
        ("GET", ["comments"]) => {
            // `since`: only comments updated at or after it.
            let since = query_param(query, "since").map(decode);
            let items = state
                .repo(full)
                .comments
                .values()
                .filter(|c| c.issue == n)
                .filter(|c| {
                    since
                        .as_deref()
                        .is_none_or(|since| c.updated_at.as_str() >= since)
                })
                .map(|c| c.json(base, full))
                .collect();
            paginate(base, path, query, items)
        }
        ("POST", ["comments"]) => {
            let Some(body) = input["body"].as_str() else {
                return Reply::invalid("body is required");
            };
            match state.create_comment(full, n, login, CLIENT_ASSOCIATION, body) {
                Some(id) => Reply::created(
                    state
                        .repo(full)
                        .comments
                        .get(&id)
                        .map(|c| c.json(base, full))
                        .unwrap_or_default(),
                ),
                None => Reply::not_found(),
            }
        }
        ("GET", ["events"]) => {
            let items = state.repo(full).issues[&n].events.clone();
            paginate(base, path, query, items)
        }
        ("GET", ["labels"]) => {
            let r = state.repo(full);
            let items = r.issues[&n]
                .labels
                .iter()
                .map(|l| r.label_json(l))
                .collect();
            paginate(base, path, query, items)
        }
        ("POST" | "PUT", ["labels"]) => {
            let labels = string_list(input.get("labels").unwrap_or(input));
            let id_seed = state.next_id();
            let now = state.now();
            let r = state.repo(full);
            r.ensure_labels(&labels, id_seed);
            if let Some(issue) = r.issues.get_mut(&n) {
                if method == "PUT" {
                    issue.labels.clear();
                }
                for label in labels {
                    if !issue.labels.contains(&label) {
                        issue.labels.push(label);
                    }
                }
                issue.updated_at = now;
            }
            let items = r.issues[&n]
                .labels
                .iter()
                .map(|l| r.label_json(l))
                .collect();
            Reply::ok(Value::Array(items))
        }
        ("DELETE", ["labels"]) => {
            let now = state.now();
            if let Some(issue) = state.repo(full).issues.get_mut(&n) {
                issue.labels.clear();
                issue.updated_at = now;
            }
            Reply::no_content()
        }
        ("DELETE", ["labels", label]) => {
            let now = state.now();
            let r = state.repo(full);
            let Some(issue) = r.issues.get_mut(&n) else {
                return Reply::not_found();
            };
            if !issue.labels.iter().any(|l| l == label) {
                return Reply::not_found();
            }
            issue.labels.retain(|l| l != label);
            issue.updated_at = now;
            let items = r.issues[&n]
                .labels
                .iter()
                .map(|l| r.label_json(l))
                .collect();
            Reply::ok(Value::Array(items))
        }
        ("GET", ["sub_issues"]) => {
            let r = state.repo(full);
            let items = r.issues[&n]
                .sub_issues
                .iter()
                .filter_map(|child| r.issue_json(base, full, *child))
                .collect();
            paginate(base, path, query, items)
        }
        ("POST", ["sub_issues"]) => {
            let Some(child) = input["sub_issue_id"]
                .as_u64()
                .and_then(|id| number_of(state.repo(full), id))
            else {
                return Reply::invalid("sub_issue_id names no issue of this repository");
            };
            let r = state.repo(full);
            if r.issues[&child].parent.is_some() && input["replace_parent"] != json!(true) {
                return Reply::invalid("the issue already has a parent");
            }
            if let Some(old) = r.issues[&child].parent
                && let Some(old) = r.issues.get_mut(&old)
            {
                old.sub_issues.retain(|c| *c != child);
            }
            if let Some(issue) = r.issues.get_mut(&child) {
                issue.parent = Some(n);
            }
            if let Some(issue) = r.issues.get_mut(&n)
                && !issue.sub_issues.contains(&child)
            {
                issue.sub_issues.push(child);
            }
            Reply::created(r.issue_json(base, full, n).unwrap_or_default())
        }
        ("DELETE", ["sub_issue"]) => {
            let Some(child) = input["sub_issue_id"]
                .as_u64()
                .and_then(|id| number_of(state.repo(full), id))
            else {
                return Reply::not_found();
            };
            let r = state.repo(full);
            if let Some(issue) = r.issues.get_mut(&n) {
                issue.sub_issues.retain(|c| *c != child);
            }
            if let Some(issue) = r.issues.get_mut(&child) {
                issue.parent = None;
            }
            issue_reply(state)
        }
        ("GET", ["parent"]) => {
            let r = state.repo(full);
            match r.issues[&n]
                .parent
                .and_then(|p| r.issue_json(base, full, p))
            {
                Some(value) => Reply::ok(value),
                None => Reply::not_found(),
            }
        }
        ("GET", ["dependencies", "blocked_by"]) => {
            let r = state.repo(full);
            let items = r.issues[&n]
                .blocked_by
                .iter()
                .filter_map(|b| r.issue_json(base, full, *b))
                .collect();
            paginate(base, path, query, items)
        }
        ("POST", ["dependencies", "blocked_by"]) => {
            let Some(blocker) = input["issue_id"]
                .as_u64()
                .and_then(|id| number_of(state.repo(full), id))
            else {
                return Reply::invalid("issue_id names no issue of this repository");
            };
            if let Some(issue) = state.repo(full).issues.get_mut(&n)
                && !issue.blocked_by.contains(&blocker)
            {
                issue.blocked_by.push(blocker);
            }
            Reply::created(
                state
                    .repo(full)
                    .issue_json(base, full, n)
                    .unwrap_or_default(),
            )
        }
        ("DELETE", ["dependencies", "blocked_by", id]) => {
            let Some(blocker) = id
                .parse::<u64>()
                .ok()
                .and_then(|id| number_of(state.repo(full), id))
            else {
                return Reply::not_found();
            };
            if let Some(issue) = state.repo(full).issues.get_mut(&n) {
                issue.blocked_by.retain(|b| *b != blocker);
            }
            issue_reply(state)
        }
        ("GET", ["artifacts"]) => {
            let kind = query_param(query, "type");
            let items = state.repo(full).issues[&n]
                .artifacts
                .values()
                .filter(|a| kind.is_none_or(|k| a.kind == k))
                .map(|a| a.json(full, n))
                .collect();
            paginate(base, path, query, items)
        }
        ("POST", ["artifacts"]) => {
            let (Some(kind), Some(name)) = (input["type"].as_str(), input["name"].as_str()) else {
                return Reply::invalid("type and name are required");
            };
            if !matches!(kind, "generic" | "plan" | "embedded_html" | "link") {
                return Reply::invalid("type is not an artifact type");
            }
            let now = state.now();
            let r = state.repo(full);
            let Some(issue) = r.issues.get_mut(&n) else {
                return Reply::not_found();
            };
            issue.next_artifact += 1;
            let number = issue.next_artifact;
            let artifact = Artifact {
                number,
                kind: kind.to_owned(),
                name: name.to_owned(),
                creator: login.to_owned(),
                updated_by: login.to_owned(),
                created_at: now.clone(),
                updated_at: now.clone(),
                versions: vec![ArtifactVersion {
                    version: 1,
                    name: name.to_owned(),
                    body: input["body"].as_str().unwrap_or("").to_owned(),
                    created_at: now,
                    login: login.to_owned(),
                }],
            };
            let value = artifact.json(full, n);
            issue.artifacts.insert(number, artifact);
            Reply::created(value)
        }
        (_, ["artifacts", a]) => {
            let now = state.now();
            let r = state.repo(full);
            let Some(issue) = r.issues.get_mut(&n) else {
                return Reply::not_found();
            };
            let Some(a) = a
                .parse::<u64>()
                .ok()
                .filter(|a| issue.artifacts.contains_key(a))
            else {
                return Reply::not_found();
            };
            match method {
                "GET" => {
                    // Versions newest first, paged as GitHub pages them.
                    let artifact = &issue.artifacts[&a];
                    let per_page = query_param(query, "per_page")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(30)
                        .clamp(1, 100);
                    let page = query_param(query, "page")
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(1)
                        .max(1);
                    let Value::Array(all) = artifact.versions_json() else {
                        return Reply::not_found();
                    };
                    let total = all.len();
                    let versions: Vec<Value> = all
                        .into_iter()
                        .skip((page - 1) * per_page)
                        .take(per_page)
                        .collect();
                    Reply::ok(json!({
                        "artifact": artifact.json(full, n),
                        "versions": versions,
                        "has_next_page": page * per_page < total,
                        "total_count": total,
                    }))
                }
                "PATCH" => {
                    let Some(artifact) = issue.artifacts.get_mut(&a) else {
                        return Reply::not_found();
                    };
                    let version = artifact.versions.last().map_or(0, |v| v.version) + 1;
                    let name = input["name"].as_str().unwrap_or(&artifact.name).to_owned();
                    artifact.name.clone_from(&name);
                    artifact.versions.push(ArtifactVersion {
                        version,
                        name,
                        body: input["body"].as_str().unwrap_or("").to_owned(),
                        created_at: now.clone(),
                        login: login.to_owned(),
                    });
                    artifact.updated_at = now;
                    artifact.updated_by = login.to_owned();
                    Reply::ok(artifact.json(full, n))
                }
                "DELETE" => {
                    issue.artifacts.remove(&a);
                    Reply::no_content()
                }
                _ => Reply::not_found(),
            }
        }
        _ => Reply::not_found(),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "one dispatch over the request's parts"
)]
fn pull_route(
    state: &mut State,
    method: &str,
    base: &str,
    full: &str,
    n: u64,
    tail: &[&str],
    path: &str,
    query: &str,
    input: &Value,
    login: &str,
) -> Reply {
    match (method, tail) {
        ("GET", []) => Reply::ok(
            state
                .repo(full)
                .pull_json(base, full, n)
                .unwrap_or_default(),
        ),
        ("PATCH", []) => {
            let now = state.now();
            let r = state.repo(full);
            if let Some(issue) = r.issues.get_mut(&n) {
                if let Some(title) = input["title"].as_str() {
                    issue.title = title.to_owned();
                }
                if let Some(body) = input["body"].as_str() {
                    issue.body = body.to_owned();
                }
                if let Some(new_state) = input["state"].as_str() {
                    issue.closed_at = (new_state == "closed").then(|| now.clone());
                    issue.state = new_state.to_owned();
                }
                issue.updated_at = now;
            }
            Reply::ok(r.pull_json(base, full, n).unwrap_or_default())
        }
        ("GET", ["reviews"]) => {
            let reviews = state.repo(full).issues[&n]
                .pull
                .as_ref()
                .map(|p| p.reviews.clone())
                .unwrap_or_default();
            paginate(base, path, query, reviews)
        }
        ("GET", ["reviews", id]) => {
            let review = id.parse::<u64>().ok().and_then(|id| {
                state.repo(full).issues[&n]
                    .pull
                    .as_ref()
                    .and_then(|p| p.reviews.iter().find(|r| r["id"] == id).cloned())
            });
            match review {
                Some(review) => Reply::ok(review),
                None => Reply::not_found(),
            }
        }
        ("POST", ["reviews"]) => {
            if matches!(input["event"].as_str(), Some("APPROVE" | "REQUEST_CHANGES"))
                && state.repo(full).issues[&n].login == login
            {
                return Reply::invalid("Can not review your own pull request");
            }
            if input["body"]
                .as_str()
                .is_some_and(|body| body.chars().count() > REVIEW_BODY_LIMIT)
            {
                return Reply::invalid("Body is too long (maximum is 65536 characters)");
            }
            let id = state.next_id();
            let now = state.now();
            let review_state = match input["event"].as_str() {
                Some("APPROVE") => "APPROVED",
                Some("REQUEST_CHANGES") => "CHANGES_REQUESTED",
                Some("COMMENT") => "COMMENTED",
                _ => "PENDING",
            };
            let review = json!({
                "id": id,
                "user": actor(login),
                "body": input["body"].as_str().unwrap_or(""),
                "state": review_state,
                "commit_id": input["commit_id"],
                "submitted_at": now,
                "html_url": format!("https://github.test/{full}/pull/{n}#pullrequestreview-{id}"),
                "pull_request_url": format!("{base}/repos/{full}/pulls/{n}"),
            });
            if let Some(pull) = state
                .repo(full)
                .issues
                .get_mut(&n)
                .and_then(|i| i.pull.as_mut())
            {
                pull.reviews.push(review.clone());
            }
            Reply::ok(review)
        }
        _ => Reply::not_found(),
    }
}

/// The number of the issue whose REST `id` is `id`.
fn number_of(repo: &Repo, id: u64) -> Option<u64> {
    repo.issues.values().find(|i| i.id == id).map(|i| i.number)
}
