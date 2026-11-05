//! Verification precedes decoding, always over original bytes.
use hmac::{Hmac, Mac};
use instrukt_core::config::{SigningSecrets, WebhookProtocol};

use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::Sha256;

pub(super) fn verify(body: &[u8], signature: Option<&str>, secrets: &SigningSecrets) -> bool {
    let Some(signature) = signature
        .and_then(|text| text.strip_prefix("sha256="))
        .and_then(|text| hex::decode(text).ok())
        .filter(|bytes| bytes.len() == 32)
    else {
        return false;
    };
    let verifies = |secret: &str| {
        let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
            return false;
        };
        mac.update(body);
        mac.verify_slice(&signature).is_ok()
    };
    // Evaluate both during rotation, independent of which one matches.
    let current = verifies(&secrets.current);
    let previous = secrets.previous.as_deref().is_some_and(verifies);
    current | previous
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Native {
    domain: String,
    name: String,
    payload: Map<String, Value>,
}

pub(super) fn decode(
    protocol: WebhookProtocol,
    body: &[u8],
    content_type: Option<&str>,
    github_type: Option<&str>,
) -> Result<(String, Map<String, Value>), ()> {
    match protocol {
        WebhookProtocol::Native => {
            let native: Native = serde_json::from_slice(body).map_err(|_| ())?;
            if native.domain.contains('.')
                || !segment(&native.domain)
                || !native.name.split('.').all(segment)
            {
                return Err(());
            }
            Ok((format!("{}.{}", native.domain, native.name), native.payload))
        }
        WebhookProtocol::Github => {
            let media = content_type
                .unwrap_or("application/json")
                .split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            let data = if media == "application/x-www-form-urlencoded" {
                strict_form(body)?
            } else {
                serde_json::from_slice::<Map<String, Value>>(body).map_err(|_| ())?
            };
            let name = github_type.unwrap_or_default().to_ascii_lowercase();
            let name = if segment(&name) {
                name.as_str()
            } else {
                "unknown"
            };
            Ok((format!("github.{name}"), data))
        }
    }
}

fn strict_form(body: &[u8]) -> Result<Map<String, Value>, ()> {
    let text = std::str::from_utf8(body).map_err(|_| ())?;
    // The URL parser deliberately tolerates malformed escapes/UTF-8. Intake does not.
    let mut decoded = Vec::new();
    let mut bytes = text.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let pair = [bytes.next().ok_or(())?, bytes.next().ok_or(())?];
            let value = hex::decode(pair).map_err(|_| ())?;
            decoded.extend(value);
        } else {
            decoded.push(byte);
        }
    }
    std::str::from_utf8(&decoded).map_err(|_| ())?;
    let fields: Vec<_> = form_urlencoded::parse(body).collect();
    if fields.len() != 1 || fields[0].0 != "payload" || text.split('&').count() != 1 {
        return Err(());
    }
    serde_json::from_str(&fields[0].1).map_err(|_| ())
}

fn segment(text: &str) -> bool {
    let mut bytes = text.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
