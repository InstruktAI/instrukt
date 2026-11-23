//! One HTTP exchange: authentication, canned and paused answers,
//! `ETag`s, the request record, and the helpers every route shares.

use std::collections::BTreeMap;
use std::sync::Mutex;

use serde_json::{Value, json};

use super::_apps::{app_login, auth_time, jwt_app};
use super::_fake::RecordedRequest;
use super::_front::{Front, lock_front};
use super::_model::State;
use super::_routes::route;

pub(super) struct Reply {
    pub(super) status: u16,
    pub(super) body: Option<Value>,
    pub(super) headers: Vec<(String, String)>,
}

impl Reply {
    pub(super) fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: Some(body),
            headers: Vec::new(),
        }
    }

    pub(super) fn ok(body: Value) -> Self {
        Self::json(200, body)
    }

    pub(super) fn created(body: Value) -> Self {
        Self::json(201, body)
    }

    pub(super) fn no_content() -> Self {
        Self {
            status: 204,
            body: None,
            headers: Vec::new(),
        }
    }

    pub(super) fn not_found() -> Self {
        Self::json(
            404,
            json!({"message": "Not Found", "documentation_url": "https://docs.github.com/rest"}),
        )
    }

    pub(super) fn invalid(message: &str) -> Self {
        Self::json(
            422,
            json!({"message": "Validation Failed", "errors": [{"message": message}]}),
        )
    }
}

pub(super) fn serve(mut request: tiny_http::Request, state: &Mutex<State>, front: &Mutex<Front>) {
    let method = request.method().as_str().to_owned();
    let url = request.url().to_owned();
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path.to_owned(), query.to_owned()),
        None => (url.clone(), String::new()),
    };
    let headers: BTreeMap<String, String> = request
        .headers()
        .iter()
        .map(|h| {
            (
                h.field.as_str().as_str().to_ascii_lowercase(),
                h.value.to_string(),
            )
        })
        .collect();
    let mut body = String::new();
    let _read = request.as_reader().read_to_string(&mut body);
    let peer = request.remote_addr().copied();

    let mut guard = state.lock().unwrap();
    // Only a request on a connection the fake still relays reaches its
    // state. One whose connection `stop` closed is dropped unanswered; the
    // answer the HTTP server then writes meets a closed connection.
    if !peer.is_some_and(|peer| lock_front(front).relays_from(peer)) {
        return;
    }
    let canned = guard
        .canned
        .iter()
        .position(|c| {
            (c.method.is_empty() || c.method == method)
                && (path.ends_with(&c.path_suffix) || url.ends_with(&c.path_suffix))
        })
        .and_then(|at| guard.canned.remove(at));
    let authorization = headers
        .get("authorization")
        .map(String::as_str)
        .unwrap_or("");
    let now = auth_time(&guard.auth_clock);
    let jwt = jwt_app(authorization, now);
    let token = authorization.strip_prefix("Bearer ").and_then(|t| {
        guard
            .tokens
            .get(t)
            .filter(|(_, expiry, _)| *expiry > now)
            .map(|(id, _, _)| (*id, t.to_owned()))
    });
    let app_id = jwt.or(token.as_ref().map(|(id, _)| *id));
    let previous_login = guard.login.clone();
    let previous_token = guard.active_token.clone();
    guard.active_token = token.as_ref().map(|(_, token)| token.clone());
    if let Some(id) = app_id {
        guard.login = app_login(id).into();
    }
    let jwt_endpoint = path == "/app"
        || path.starts_with("/app/installations/")
        || path.ends_with("/installation");
    let public_app = path.starts_with("/apps/");
    let mut reply =
        if jwt_endpoint && jwt.is_none() || (!jwt_endpoint && !public_app && token.is_none()) {
            Reply::json(401, json!({"message":"Bad credentials"}))
        } else if public_app && jwt.is_none() {
            Reply::not_found()
        } else {
            match canned {
                Some(canned) => Reply {
                    status: canned.response.status,
                    body: Some(canned.response.body),
                    headers: canned.response.headers,
                },
                None => route(&mut guard, &method, &path, &query, &body),
            }
        };
    guard.active_token = previous_token;
    guard.login = previous_login;
    if let Some(seconds) = guard.poll_interval {
        reply
            .headers
            .push(("X-Poll-Interval".into(), seconds.to_string()));
    }
    let text = reply
        .body
        .as_ref()
        .map(Value::to_string)
        .unwrap_or_default();
    let mut status = reply.status;
    if method == "GET" && status == 200 {
        let etag = etag_of(&text);
        if headers
            .get("if-none-match")
            .is_some_and(|given| given.split(',').any(|tag| same_etag(tag, &etag)))
        {
            status = 304;
        }
        reply.headers.push(("ETag".into(), etag));
    }
    let pause = guard
        .pauses
        .iter()
        .position(|p| p.method == method && path.ends_with(&p.path_suffix))
        .and_then(|at| guard.pauses.remove(at));
    guard.requests.push(RecordedRequest {
        method,
        path,
        query,
        headers: headers
            .into_iter()
            .map(|(key, value)| {
                let value = if key == "authorization" {
                    "<redacted>".into()
                } else {
                    value
                };
                (key, value)
            })
            .collect(),
        body,
        status,
    });
    drop(guard);
    if let Some(pause) = pause
        && pause.reached.send(()).is_ok()
    {
        // The test owns the response boundary; disconnect releases it
        // even if an assertion in the test unwinds.
        let _released = pause.released.recv();
    }

    let mut response = if status == 304 || reply.body.is_none() {
        tiny_http::Response::from_string(String::new()).with_status_code(status)
    } else {
        tiny_http::Response::from_string(text)
            .with_status_code(status)
            .with_header(header("Content-Type", "application/json; charset=utf-8"))
    };
    if !reply
        .headers
        .iter()
        .any(|(name, _)| name.eq_ignore_ascii_case("x-ratelimit-remaining"))
    {
        response.add_header(header("X-RateLimit-Limit", "5000"));
        response.add_header(header("X-RateLimit-Remaining", "4999"));
    }
    for (name, value) in &reply.headers {
        response.add_header(header(name, value));
    }
    let _sent = request.respond(response);
}

pub(super) fn header(name: &str, value: &str) -> tiny_http::Header {
    tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
}

pub(super) fn etag_of(text: &str) -> String {
    use std::hash::{DefaultHasher, Hash as _, Hasher as _};
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    format!("W/\"{:016x}\"", hasher.finish())
}

pub(super) fn same_etag(given: &str, etag: &str) -> bool {
    let bare = |tag: &str| tag.trim().trim_start_matches("W/").to_owned();
    given.trim() == "*" || bare(given) == bare(etag)
}

pub(super) fn query_param<'q>(query: &'q str, name: &str) -> Option<&'q str> {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value)
}

/// Percent-decodes a path segment or query value.
pub(super) fn decode(segment: &str) -> String {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| std::str::from_utf8(hex).ok())
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Slices `items` by `per_page`/`page` and adds GitHub's `Link` header.
pub(super) fn paginate(base: &str, path: &str, query: &str, items: Vec<Value>) -> Reply {
    let per_page = query_param(query, "per_page")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(30)
        .clamp(1, 100);
    let page = query_param(query, "page")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let last = items.len().div_ceil(per_page).max(1);
    let slice: Vec<Value> = items
        .into_iter()
        .skip((page - 1) * per_page)
        .take(per_page)
        .collect();
    let mut reply = Reply::ok(Value::Array(slice));
    if last > 1 {
        let link_to = |n: usize| {
            let mut params: Vec<String> = query
                .split('&')
                .filter(|p| !p.is_empty() && !p.starts_with("page=") && !p.starts_with("per_page="))
                .map(str::to_owned)
                .collect();
            params.push(format!("per_page={per_page}"));
            params.push(format!("page={n}"));
            format!("<{base}{path}?{}>", params.join("&"))
        };
        let mut links = Vec::new();
        if page < last {
            links.push(format!("{}; rel=\"next\"", link_to(page + 1)));
            links.push(format!("{}; rel=\"last\"", link_to(last)));
        }
        if page > 1 {
            links.push(format!("{}; rel=\"first\"", link_to(1)));
            links.push(format!("{}; rel=\"prev\"", link_to(page - 1)));
        }
        if !links.is_empty() {
            reply.headers.push(("Link".into(), links.join(", ")));
        }
    }
    reply
}
