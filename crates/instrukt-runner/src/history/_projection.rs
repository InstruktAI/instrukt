//! Commits normalized changes with the runtime-opaque source position.
use super::{
    _store::{self, Conversation},
    HistoryError,
};
use crate::runtime::conversations::{ConversationChanges, RecordKind, SourcePosition};
use instrukt_core::{
    home::Home,
    state_db::{
        StateDb, StateRead,
        rusqlite::{OptionalExtension, params},
    },
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(super) fn position(home: &Home, source: &str) -> Result<Option<SourcePosition>, HistoryError> {
    let Some(db) = StateRead::open(home)? else {
        return Ok(None);
    };
    let stored: Option<String> = db
        .conn()
        .query_row(
            "SELECT position_json FROM history_source_positions WHERE source=?1",
            [source],
            |r| r.get(0),
        )
        .optional()?;
    stored
        .map(|s| serde_json::from_str(&s).map_err(HistoryError::from))
        .transpose()
}
pub(super) fn apply(
    home: &Home,
    source: &str,
    batch: &ConversationChanges,
) -> Result<usize, HistoryError> {
    let computer = instrukt_core::machine::machine_id(home)?.to_string();
    let mut db = StateDb::open(home)?;
    db.write(|tx|->Result<usize,HistoryError>{
        let mut affected=BTreeSet::new();
        for record in &batch.records {
            tx.execute("INSERT INTO history_projection_records VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(source,projection,native_id) DO UPDATE SET thread_id=excluded.thread_id,updated_at=excluded.updated_at,payload_json=excluded.payload_json",params![source,record.kind.name(),record.id,record.conversation_id,record.updated_at.map(|t|t.to_string()).unwrap_or_default(),serde_json::to_string(record)?])?;
            affected.insert(record.conversation_id.clone());
        }
        for thread in &affected {
            let mut stmt=tx.prepare("SELECT payload_json FROM history_projection_records WHERE source=?1 AND thread_id=?2 ORDER BY updated_at,native_id")?;
            let raw=stmt.query_map(params![source,thread],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
            let mut document=json!({"messages":[],"runs":[],"subagents":[],"turnItems":[]});
            let mut title=None;let mut parent=None;let mut text=Vec::new();
            for payload in raw {
                let value:Value=serde_json::from_str(&payload)?;
                let kind:RecordKind=serde_json::from_value(value["kind"].clone())?;
                match kind {
                    RecordKind::Conversation=>{document["thread"]=value["native"].clone();title=value["title"].as_str().map(str::to_owned);parent=value["parent_id"].as_str().map(str::to_owned);},
                    _=>{
                        let key=match kind {RecordKind::Message=>"messages",RecordKind::Run=>"runs",RecordKind::Subagent=>"subagents",RecordKind::TurnItem=>"turnItems",RecordKind::Conversation=>unreachable!()};
                        document[key].as_array_mut().ok_or_else(||HistoryError::Source("invalid retained record collection".into()))?.push(value["native"].clone());
                        if let Some(s)=value["text"].as_str().filter(|s|!s.is_empty()){text.push(s.to_owned());}
                    },
                }
            }
            let Some(title)=title else{return Err(HistoryError::Source("source conversation metadata is not available yet".into()));};
            _store::write_conversation(tx,source,&Conversation{native_id:thread.clone(),computer:computer.clone(),title,parent,aliases:vec![thread.clone()],document,body:text.join("\n\n"),limitations:vec!["Only content retained by the source runtime is available.".into()]})?;
        }
        tx.execute("INSERT INTO history_source_positions VALUES(?1,?2) ON CONFLICT(source) DO UPDATE SET position_json=excluded.position_json",params![source,serde_json::to_string(&batch.position)?])?;
        Ok(affected.len())
    })
}
