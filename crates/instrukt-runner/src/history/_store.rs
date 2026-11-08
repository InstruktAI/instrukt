use super::{Extraction, HistoryError, Match};
use instrukt_core::{
    home::Home,
    state_db::{
        StateDb, StateRead,
        rusqlite::{OptionalExtension, params},
    },
};
use serde_json::Value;

pub(super) struct Conversation {
    pub active: bool,
    pub native_id: String,
    pub computer: String,
    pub title: String,
    pub parent: Option<String>,
    pub aliases: Vec<String>,
    pub document: Value,
    pub body: String,
    pub limitations: Vec<String>,
}

pub(super) fn position(home: &Home, source: &str) -> Result<(u64, bool), HistoryError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok((0, false));
    };
    Ok(db
        .conn()
        .query_row(
            "SELECT position,imported FROM history_sources WHERE source=?1",
            [source],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
        .unwrap_or((0, false)))
}

pub(super) fn load(
    home: &Home,
    source: &str,
    position: u64,
    imported: bool,
    conversations: &[Conversation],
) -> Result<Extraction, HistoryError> {
    let mut db = StateDb::open(home)?;
    db.write(|tx| -> Result<_,HistoryError> {
        for c in conversations {
            if c.active {
                tx.execute("INSERT OR IGNORE INTO history_pending(source,native_id) VALUES(?1,?2)",params![source,c.native_id])?;
            } else {
                tx.execute("DELETE FROM history_pending WHERE source=?1 AND native_id=?2",params![source,c.native_id])?;
            }
            tx.execute("INSERT INTO history_conversations(source,native_id,computer,title,parent,aliases,document,body,limitations) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(source,native_id) DO UPDATE SET computer=excluded.computer,title=excluded.title,parent=excluded.parent,aliases=excluded.aliases,document=excluded.document,body=excluded.body,limitations=excluded.limitations",params![source,c.native_id,c.computer,c.title,c.parent,serde_json::to_string(&c.aliases)?,c.document.to_string(),c.body,serde_json::to_string(&c.limitations)?])?;
        }
        tx.execute("INSERT INTO history_sources(source,position,imported) VALUES(?1,?2,?3) ON CONFLICT(source) DO UPDATE SET position=max(history_sources.position,excluded.position),imported=max(history_sources.imported,excluded.imported)",params![source,position,imported])?;
        Ok(Extraction {loaded:conversations.len(),position})
    })
}

pub(super) fn pending(home: &Home, source: &str) -> Result<Vec<String>, HistoryError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(Vec::new());
    };
    let mut stmt = db
        .conn()
        .prepare("SELECT native_id FROM history_pending WHERE source=?1 ORDER BY native_id")?;
    Ok(stmt
        .query_map([source], |r| r.get(0))?
        .collect::<Result<Vec<_>, _>>()?)
}

pub(super) fn remove_pending(home: &Home, source: &str, id: &str) -> Result<(), HistoryError> {
    let mut db = StateDb::open(home)?;
    db.write(|tx| -> Result<(), HistoryError> {
        tx.execute(
            "DELETE FROM history_pending WHERE source=?1 AND native_id=?2",
            params![source, id],
        )?;
        Ok(())
    })
}

pub(super) fn defer(
    home: &Home,
    source: &str,
    id: &str,
    position: u64,
) -> Result<(), HistoryError> {
    let mut db = StateDb::open(home)?;
    db.write(|tx|->Result<(),HistoryError>{
        tx.execute("INSERT OR IGNORE INTO history_pending(source,native_id) VALUES(?1,?2)",params![source,id])?;
        tx.execute("INSERT INTO history_sources(source,position) VALUES(?1,?2) ON CONFLICT(source) DO UPDATE SET position=max(history_sources.position,excluded.position)",params![source,position])?;
        Ok(())
    })
}

pub(super) fn search(home: &Home, query: &str, limit: usize) -> Result<Vec<Match>, HistoryError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(Vec::new());
    };
    let mut stmt = db.conn().prepare("SELECT c.computer,c.source,c.native_id,c.title,c.parent,c.aliases,snippet(history_fts,1,'','', '…',24),c.limitations FROM history_fts JOIN history_conversations c ON c.id=history_fts.rowid WHERE history_fts MATCH ?1 ORDER BY rank,c.source,c.native_id LIMIT ?2")?;
    let rows = stmt.query_map(params![query, limit as u64], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
            r.get::<_, String>(7)?,
        ))
    })?;
    rows.map(|row| {
        let (computer, source, conversation, title, parent, aliases, excerpt, limitations) = row?;
        Ok(Match {
            store_computer: String::new(),
            computer,
            source,
            conversation,
            title,
            parent,
            aliases: serde_json::from_str(&aliases)?,
            excerpt,
            limitations: serde_json::from_str(&limitations)?,
        })
    })
    .collect()
}

pub(super) fn show(home: &Home, source: &str, conversation: &str) -> Result<Value, HistoryError> {
    let Some(db) = StateRead::open(home)? else {
        return Err(HistoryError::Source("conversation not retained".into()));
    };
    let mut statement=db.conn().prepare("SELECT native_id,document FROM history_conversations WHERE source=?1 AND (native_id=?2 OR EXISTS(SELECT 1 FROM json_each(aliases) WHERE value=?2)) ORDER BY native_id=?2 DESC LIMIT 2")?;
    let found = statement
        .query_map(params![source, conversation], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let Some((id, document)) = found.first() else {
        return Err(HistoryError::Source("conversation not retained".into()));
    };
    if found.len() > 1 && id != conversation {
        return Err(HistoryError::Source(
            "archive alias is ambiguous; use a retained conversation id".into(),
        ));
    }
    serde_json::from_str(document).map_err(HistoryError::from)
}
