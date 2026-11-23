//! GitHub App authentication: the test Apps' keys, JWT and installation
//! token checks, and the clock GitHub judges tokens by.

use serde_json::{Value, json};

use super::super::TestEnv;
use super::_model::actor;

pub(super) fn app_login(id: u64) -> &'static str {
    match id {
        2 => "instruktai-author[bot]",
        3 => "instruktai-reviewer[bot]",
        _ => "instruktai[bot]",
    }
}

/// Publicly committed RSA fixtures used only by the isolated fake App.
pub(super) fn test_keys() -> (&'static str, &'static str) {
    (
        include_str!("../../fixtures/test-app-private.pem"),
        include_str!("../../fixtures/test-app-public.pem"),
    )
}
pub(super) fn jwt_app(authorization: &str, now: jiff::Timestamp) -> Option<u64> {
    let jwt = authorization.strip_prefix("Bearer ")?;
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.validate_exp = false;
    let claims = jsonwebtoken::decode::<Value>(
        jwt,
        &jsonwebtoken::DecodingKey::from_rsa_pem(test_keys().1.as_bytes()).ok()?,
        &validation,
    )
    .ok()?
    .claims;
    let issued = claims["iat"].as_i64()?;
    let expires = claims["exp"].as_i64()?;
    let now = now.as_second();
    if issued > now
        || issued < now - 660
        || expires <= now
        || expires > now + 600
        || expires <= issued
        || expires - issued > 660
    {
        return None;
    }
    claims["iss"]
        .as_str()?
        .parse::<u64>()
        .ok()
        .filter(|id| (1..=3).contains(id))
}

pub fn configure_apps(env: &TestEnv, api: &str, clock: &std::path::Path) {
    std::fs::write(
        env.home().join(".fake-github-clock"),
        clock.to_str().unwrap(),
    )
    .unwrap();
    let file = env.instrukt_home().join("instrukt.yaml");
    let mut layer: serde_yaml_ng::Value = std::fs::read_to_string(&file)
        .ok()
        .map(|s| serde_yaml_ng::from_str(&s).unwrap())
        .unwrap_or_else(|| serde_yaml_ng::from_str("schema_version: 4").unwrap());
    let mut profiles = serde_json::Map::new();
    for (id, name) in [(1, "project"), (2, "author"), (3, "reviewer")] {
        let login = app_login(id);
        profiles.insert(name.into(),json!({"app_id":id,"slug":login.trim_end_matches("[bot]"),"bot":actor(login).as_object().unwrap().iter().filter(|(k,_)|k.as_str()!="type").map(|(k,v)|(k.clone(),v.clone())).collect::<serde_json::Map<_,_>>(),"private_key":"${TEST_APP_KEY}"}));
    }
    layer["github"]["api_url"] = api.into();
    layer["github"]["project_app"] = "project".into();
    layer["github"]["apps"] = serde_yaml_ng::to_value(profiles).unwrap();
    std::fs::write(file, serde_yaml_ng::to_string(&layer).unwrap()).unwrap();
    let home = instrukt_core::home::Home::at_user_home(
        env.instrukt_home().parent().unwrap().to_path_buf(),
    );
    instrukt_core::config::set_secret(&home, "TEST_APP_KEY", test_keys().0).unwrap();
}

/// The time GitHub judges tokens by: the system clock, the time instrukt
/// was given, or the time in the clock file it was given (`@<path>`), read
/// at each request as instrukt reads it.
pub(super) fn auth_time(file: &std::path::Path) -> jiff::Timestamp {
    let value = std::fs::read_to_string(file).unwrap();
    if value == "system" {
        jiff::Timestamp::now()
    } else if let Some(clock) = value.strip_prefix('@') {
        std::fs::read_to_string(clock)
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    } else {
        value.parse().unwrap()
    }
}
pub fn sync_auth_clock(home: &std::path::Path, now: Option<&str>) {
    if let Ok(file) = std::fs::read_to_string(home.join(".fake-github-clock"))
        && std::path::Path::new(&file).exists()
    {
        // Replaced whole, so the fake's reader never finds it half written.
        let next = format!("{file}.next");
        std::fs::write(&next, now.unwrap_or("system")).unwrap();
        std::fs::rename(&next, &file).unwrap();
    }
}
