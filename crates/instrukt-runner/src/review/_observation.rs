//! The pull-request listing of an opted-in project, read page by page under
//! conditional validators kept in `state/review-observation.db`
//! (`pull-request-review`, Observation, Observation store).

use instrukt_core::home::Home;
use instrukt_core::ids::ProjectSlug;
use instrukt_core::state_db::rusqlite::{OptionalExtension, params};
use instrukt_core::state_db::{Migration, Store, StoreDb, StoreLock};
use serde_json::Value;

use crate::ci::{Cached, CiError, Fetched, conditional_get};

/// How long a tick waits for the observation lock.
const LOCK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The listing's page size; a shorter page ends it.
const PAGE_SIZE: usize = 100;

const V1: &str = r#"
CREATE TABLE pages (
 project TEXT NOT NULL,
 page INTEGER NOT NULL,
 etag TEXT NOT NULL,
 body TEXT NOT NULL,
 PRIMARY KEY (project, page)
);
"#;

const MIGRATIONS: &[Migration] = &[Migration::Sql(V1)];

pub(crate) fn store(home: &Home) -> Store {
    Store {
        name: "the pull-request observation store",
        file: home.review_observation_db_file(),
        lock: StoreLock::Exclusive(home.review_observation_lock_file(), LOCK_TIMEOUT),
        migrations: MIGRATIONS,
    }
}

/// The page this store keeps for `project`, read without a lock.
fn kept(home: &Home, project: &ProjectSlug, page: usize) -> Result<Option<Cached>, CiError> {
    let read = || -> Result<Option<Cached>, instrukt_core::state_db::StoreError> {
        let Some(conn) = store(home).read()? else {
            return Ok(None);
        };
        conn.query_row(
            "SELECT etag, body FROM pages WHERE project=?1 AND page=?2",
            params![project.as_str(), page],
            |row| {
                Ok(Cached {
                    etag: row.get(0)?,
                    body: row.get(1)?,
                })
            },
        )
        .optional()
        .map_err(|source| instrukt_core::state_db::StoreError::Sqlite {
            store: "the pull-request observation store",
            path: home.review_observation_db_file(),
            source,
        })
    };
    read().map_err(|error| CiError::Body {
        url: home.review_observation_db_file().display().to_string(),
        message: error.to_string(),
    })
}

/// One write to the store under its lock: `page`'s validator and body, or
/// none of it.
fn keep(
    home: &Home,
    project: &ProjectSlug,
    page: usize,
    answer: Option<&Cached>,
) -> Result<(), CiError> {
    write(home, |tx| {
        match answer {
            Some(cached) => tx.execute(
                "INSERT INTO pages (project, page, etag, body) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (project, page) DO UPDATE SET etag=excluded.etag, body=excluded.body",
                params![project.as_str(), page, cached.etag, cached.body],
            ),
            None => tx.execute(
                "DELETE FROM pages WHERE project=?1 AND page=?2",
                params![project.as_str(), page],
            ),
        }
        .map(drop)
    })
}

/// Removes every page of `project` after `page`, the listing's last.
fn drop_after(home: &Home, project: &ProjectSlug, page: usize) -> Result<(), CiError> {
    write(home, |tx| {
        tx.execute(
            "DELETE FROM pages WHERE project=?1 AND page>?2",
            params![project.as_str(), page],
        )
        .map(drop)
    })
}

/// One transaction under the store's lock.
fn write(
    home: &Home,
    change: impl FnOnce(
        &instrukt_core::state_db::rusqlite::Transaction<'_>,
    ) -> Result<(), instrukt_core::state_db::rusqlite::Error>,
) -> Result<(), CiError> {
    let fail = |message: String| CiError::Body {
        url: home.review_observation_db_file().display().to_string(),
        message,
    };
    let mut db = StoreDb::open(&store(home)).map_err(|error| fail(error.to_string()))?;
    db.write(|tx| {
        change(tx).map_err(|source| instrukt_core::state_db::StoreError::Sqlite {
            store: "the pull-request observation store",
            path: home.review_observation_db_file(),
            source,
        })
    })
    .map_err(|error: instrukt_core::state_db::StoreError| fail(error.to_string()))
}

/// Every open pull request of `repository`, oldest first, read page by
/// page until a short page ends the listing. `interval` takes any poll
/// interval GitHub sent.
pub(crate) fn open_pulls(
    home: &Home,
    project: &ProjectSlug,
    api_url: &str,
    repository: &str,
    token: &str,
    interval: &mut Option<i64>,
) -> Result<Vec<Value>, CiError> {
    let mut pulls = Vec::new();
    for page in 1.. {
        let url = format!(
            "{}/repos/{repository}/pulls?state=open&sort=created&direction=asc&per_page={PAGE_SIZE}&page={page}",
            api_url.trim_end_matches('/')
        );
        let cached = kept(home, project, page)?;
        let body = match conditional_get(&url, token, cached.as_ref(), interval)? {
            Fetched::Unchanged { body } => body,
            Fetched::Fresh { etag, body } => {
                let answer = etag.map(|etag| Cached {
                    etag,
                    body: body.clone(),
                });
                keep(home, project, page, answer.as_ref())?;
                body
            }
        };
        let items: Vec<Value> = serde_json::from_str(&body).map_err(|error| CiError::Body {
            url: url.clone(),
            message: error.to_string(),
        })?;
        let full = items.len() == PAGE_SIZE;
        pulls.extend(items);
        if !full {
            drop_after(home, project, page)?;
            break;
        }
    }
    Ok(pulls)
}
