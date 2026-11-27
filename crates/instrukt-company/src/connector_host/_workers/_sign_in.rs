//! Private MCP sign-in link and outcome direct-message delivery worker for a
//! hosted connector. Polls `mcp_sign_ins` read-only via `StateRead` (never
//! taking the writer lock when idle) and never holds `StateDb` across connector
//! network calls.
use std::collections::HashSet;
use std::time::{Duration, Instant};

use instrukt_connector::DirectMessageError;
use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::state_db::rusqlite::params;
use instrukt_core::state_db::{StateDb, StateDbError, StateRead};
use jiff::Timestamp;

use super::super::{LOG_TARGET, Reach};

/// Dedicated worker delivering private MCP sign-in links and outcome notices
/// by direct message through the conversation's connector. Polls read-only via
/// `StateRead` (never taking the writer lock when idle) and never holds `StateDb`
/// across connector network calls.
pub(super) fn worker(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    stop: &dyn Fn() -> bool,
) {
    let mut sent_links = HashSet::<String>::new();
    let mut sent_outcomes = HashSet::<String>::new();
    while !stop() {
        let now = reach.connectors.now();
        deliver_pending_sign_in_dms(
            home,
            reach,
            company,
            key,
            now,
            &mut sent_links,
            &mut sent_outcomes,
        );
        let until = Instant::now() + Duration::from_millis(250);
        while Instant::now() < until && !stop() {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Sanitize an OAuth error code for plain-text direct messages per `mcp-access`:
/// up to 64 printable RFC 6749 NQCHAR bytes (`0x21..=0x7E`, excluding `"` and `\`),
/// or `"unknown"` otherwise.
fn sanitize_oauth_error_code(raw: Option<&str>) -> &str {
    match raw.map(str::trim) {
        Some(code)
            if !code.is_empty()
                && code.len() <= 64
                && code
                    .bytes()
                    .all(|b| (0x21..=0x7E).contains(&b) && b != b'"' && b != b'\\') =>
        {
            code
        }
        _ => "unknown",
    }
}

struct PendingLink {
    state_digest: String,
    server: String,
    recipient: String,
    link: String,
    expires_at: String,
}

struct PendingOutcome {
    state_digest: String,
    server: String,
    recipient: String,
    state: String,
    failure: Option<String>,
}

fn deliver_pending_sign_in_dms(
    home: &Home,
    reach: Reach<'_>,
    company: &ProjectSlug,
    key: &str,
    now: Timestamp,
    sent_links: &mut HashSet<String>,
    sent_outcomes: &mut HashSet<String>,
) {
    let inspection = home.for_inspection();
    let (pending_links, pending_outcomes): (Vec<PendingLink>, Vec<PendingOutcome>) =
        match StateRead::open(&inspection) {
            Ok(Some(read)) => {
                let links = match read.conn().prepare(
                    "SELECT state_digest, server, recipient, link, expires_at
                     FROM mcp_sign_ins
                     WHERE company = ?1 AND connector = ?2 AND state = 'open'
                       AND link_sent_at IS NULL AND link IS NOT NULL",
                ) {
                    Ok(mut stmt) => match stmt.query_map(params![company.as_str(), key], |r| {
                        Ok(PendingLink {
                            state_digest: r.get(0)?,
                            server: r.get(1)?,
                            recipient: r.get(2)?,
                            link: r.get(3)?,
                            expires_at: r.get(4)?,
                        })
                    }) {
                        Ok(rows) => {
                            let mut out = Vec::new();
                            for row in rows {
                                match row {
                                    Ok(item) => out.push(item),
                                    Err(error) => tracing::warn!(
                                        target: LOG_TARGET,
                                        event = "mcp_sign_in_link_row_error",
                                        company = %company,
                                        connector = key,
                                        %error
                                    ),
                                }
                            }
                            out
                        }
                        Err(error) => {
                            tracing::warn!(
                                target: LOG_TARGET,
                                event = "mcp_sign_in_link_query_failed",
                                company = %company,
                                connector = key,
                                %error
                            );
                            Vec::new()
                        }
                    },
                    Err(error) => {
                        tracing::warn!(
                            target: LOG_TARGET,
                            event = "mcp_sign_in_link_prepare_failed",
                            company = %company,
                            connector = key,
                            %error
                        );
                        Vec::new()
                    }
                };
                let outcomes = match read.conn().prepare(
                    "SELECT state_digest, server, recipient, state, failure
                     FROM mcp_sign_ins
                     WHERE company = ?1 AND connector = ?2
                       AND state IN ('completed', 'refused', 'declined', 'failed')
                       AND told_at IS NULL",
                ) {
                    Ok(mut stmt) => match stmt.query_map(params![company.as_str(), key], |r| {
                        Ok(PendingOutcome {
                            state_digest: r.get(0)?,
                            server: r.get(1)?,
                            recipient: r.get(2)?,
                            state: r.get(3)?,
                            failure: r.get(4)?,
                        })
                    }) {
                        Ok(rows) => {
                            let mut out = Vec::new();
                            for row in rows {
                                match row {
                                    Ok(item) => out.push(item),
                                    Err(error) => tracing::warn!(
                                        target: LOG_TARGET,
                                        event = "mcp_sign_in_outcome_row_error",
                                        company = %company,
                                        connector = key,
                                        %error
                                    ),
                                }
                            }
                            out
                        }
                        Err(error) => {
                            tracing::warn!(
                                target: LOG_TARGET,
                                event = "mcp_sign_in_outcome_query_failed",
                                company = %company,
                                connector = key,
                                %error
                            );
                            Vec::new()
                        }
                    },
                    Err(error) => {
                        tracing::warn!(
                            target: LOG_TARGET,
                            event = "mcp_sign_in_outcome_prepare_failed",
                            company = %company,
                            connector = key,
                            %error
                        );
                        Vec::new()
                    }
                };
                (links, outcomes)
            }
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    target: LOG_TARGET,
                    event = "mcp_sign_in_state_read_failed",
                    company = %company,
                    connector = key,
                    %error
                );
                return;
            }
        };

    // Prune local de-duplication sets once DB rows are settled.
    sent_links.retain(|digest| pending_links.iter().any(|p| p.state_digest == *digest));
    sent_outcomes.retain(|digest| pending_outcomes.iter().any(|p| p.state_digest == *digest));

    if pending_links.is_empty() && pending_outcomes.is_empty() {
        return;
    }

    let now_str = now.to_string();
    let connector = match reach.connectors.open(&reach.current.get(), company, key) {
        Ok(connector) => connector,
        Err(error) => {
            tracing::warn!(
                target: LOG_TARGET,
                event = "mcp_sign_in_connector_unavailable",
                company = %company,
                connector = key,
                error = %error
            );
            return;
        }
    };

    for item in pending_links {
        if item
            .expires_at
            .parse::<Timestamp>()
            .is_ok_and(|exp| exp <= now)
        {
            match StateDb::open(home) {
                Ok(mut db) => {
                    let path = db.path().to_path_buf();
                    if let Err(error) = db.write::<_, StateDbError>(|tx| {
                        tx.execute(
                            "UPDATE mcp_sign_ins
                             SET state = 'expired', closed_at = ?1, code_verifier = NULL, nonce = NULL, link = NULL
                             WHERE state_digest = ?2 AND state = 'open'",
                            params![now_str, item.state_digest],
                        )
                        .map_err(StateDbError::sqlite(&path))?;
                        Ok(())
                    }) {
                        tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_expire_write_failed", company = %company, %error);
                    }
                }
                Err(error) => {
                    tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_expire_open_failed", company = %company, %error);
                }
            }
            continue;
        }

        let already_sent = sent_links.contains(&item.state_digest);
        let send_outcome = if already_sent {
            Ok(())
        } else {
            let message = format!(
                "To let the agent use {} as you, sign in here. The link works once, for ten minutes: {}",
                item.server, item.link
            );
            instrukt_outside::runtime::block_on(connector.direct_message(&item.recipient, &message))
                .map_err(|e| DirectMessageError::Retryable(e.to_string()))
                .and_then(|res| res.map(|_| ()))
        };
        match send_outcome {
            Ok(()) => {
                sent_links.insert(item.state_digest.clone());
                match StateDb::open(home) {
                    Ok(mut db) => {
                        let path = db.path().to_path_buf();
                        if let Err(error) = db.write::<_, StateDbError>(|tx| {
                            tx.execute(
                                "UPDATE mcp_sign_ins
                                 SET link_sent_at = ?1, link = NULL
                                 WHERE state_digest = ?2 AND state = 'open'",
                                params![now_str, item.state_digest],
                            )
                            .map_err(StateDbError::sqlite(&path))?;
                            Ok(())
                        }) {
                            tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_sent_write_failed", company = %company, server = %item.server, %error);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_sent_open_failed", company = %company, server = %item.server, %error);
                    }
                }
            }
            // The outside gate reports the wait; the next poll retries.
            Err(DirectMessageError::Retryable(_)) => {}
            Err(DirectMessageError::Final(cause)) => {
                tracing::warn!(
                    target: LOG_TARGET,
                    event = "mcp_sign_in_dm_undelivered",
                    company = %company,
                    connector = key,
                    server = %item.server,
                    cause = %cause
                );
                match StateDb::open(home) {
                    Ok(mut db) => {
                        let path = db.path().to_path_buf();
                        if let Err(error) = db.write::<_, StateDbError>(|tx| {
                            tx.execute(
                                "UPDATE mcp_sign_ins
                                 SET state = 'undelivered', failure = ?1, closed_at = ?2, code_verifier = NULL, nonce = NULL, link = NULL
                                 WHERE state_digest = ?3 AND state = 'open'",
                                params![cause, now_str, item.state_digest],
                            )
                            .map_err(StateDbError::sqlite(&path))?;
                            Ok(())
                        }) {
                            tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_undelivered_write_failed", company = %company, server = %item.server, %error);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_undelivered_open_failed", company = %company, server = %item.server, %error);
                    }
                }
            }
        }
    }

    for item in pending_outcomes {
        let already_sent = sent_outcomes.contains(&item.state_digest);
        let send_outcome = if already_sent {
            Ok(())
        } else {
            let text = match item.state.as_str() {
                "completed" => format!(
                    "You are signed in to {}. Ask the agent again to continue.",
                    item.server
                ),
                "refused" => format!(
                    "That sign-in to {} was not for your address and was not kept.",
                    item.server
                ),
                "declined" | "failed" => {
                    let code = sanitize_oauth_error_code(item.failure.as_deref());
                    format!("The sign-in to {} did not complete: {code}.", item.server)
                }
                _ => continue,
            };
            instrukt_outside::runtime::block_on(connector.direct_message(&item.recipient, &text))
                .map_err(|e| DirectMessageError::Retryable(e.to_string()))
                .and_then(|res| res.map(|_| ()))
        };
        match send_outcome {
            Ok(()) => {
                sent_outcomes.insert(item.state_digest.clone());
                match StateDb::open(home) {
                    Ok(mut db) => {
                        let path = db.path().to_path_buf();
                        if let Err(error) = db.write::<_, StateDbError>(|tx| {
                            tx.execute(
                                "UPDATE mcp_sign_ins SET told_at = ?1 WHERE state_digest = ?2 AND told_at IS NULL",
                                params![now_str, item.state_digest],
                            )
                            .map_err(StateDbError::sqlite(&path))?;
                            Ok(())
                        }) {
                            tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_told_write_failed", company = %company, server = %item.server, %error);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_told_open_failed", company = %company, server = %item.server, %error);
                    }
                }
            }
            // The outside gate reports the wait; the next poll retries.
            Err(DirectMessageError::Retryable(_)) => {}
            Err(DirectMessageError::Final(cause)) => {
                tracing::warn!(
                    target: LOG_TARGET,
                    event = "mcp_sign_in_outcome_dm_failed",
                    company = %company,
                    connector = key,
                    server = %item.server,
                    cause = %cause
                );
                if let Ok(mut db) = StateDb::open(home) {
                    let path = db.path().to_path_buf();
                    if let Err(error) = db.write::<_, StateDbError>(|tx| {
                        tx.execute(
                            "UPDATE mcp_sign_ins SET told_at = ?1 WHERE state_digest = ?2 AND told_at IS NULL",
                            params![now_str, item.state_digest],
                        )
                        .map_err(StateDbError::sqlite(&path))?;
                        Ok(())
                    }) {
                        tracing::warn!(target: LOG_TARGET, event = "mcp_sign_in_told_final_write_failed", company = %company, %error);
                    }
                }
            }
        }
    }
}
