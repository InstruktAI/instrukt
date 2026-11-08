//! Content-sensitive per-conversation discovery in short, read-only row scopes.
use super::conversations::SourceError;
use instrukt_core::state_db::rusqlite::{
    self, Connection,
    functions::{Aggregate, Context, FunctionFlags},
    types::ValueRef,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub(super) const FUNCTION: &str = "instrukt_source_fingerprint";
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Fingerprint {
    rows: u64,
    digest: [u8; 32],
}
impl Fingerprint {
    pub(super) fn add(&mut self, other: &Self) {
        self.rows += other.rows;
        for (target, value) in self.digest.iter_mut().zip(other.digest) {
            *target ^= value;
        }
    }
    pub(super) fn row(values: [ValueRef<'_>; 8]) -> Self {
        let mut hash = Sha256::new();
        for value in values {
            hash_value(&mut hash, value);
        }
        Self {
            rows: 1,
            digest: hash.finalize().into(),
        }
    }
}
fn hash_value(hash: &mut Sha256, value: ValueRef<'_>) {
    match value {
        ValueRef::Null => hash.update([0]),
        ValueRef::Integer(n) => {
            hash.update([1]);
            hash.update(n.to_le_bytes());
        }
        ValueRef::Real(n) => {
            hash.update([2]);
            hash.update(n.to_bits().to_le_bytes());
        }
        ValueRef::Text(bytes) => {
            hash.update([3]);
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
        ValueRef::Blob(bytes) => {
            hash.update([4]);
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
        }
    }
}
struct ContentFingerprint;
impl Aggregate<Fingerprint, String> for ContentFingerprint {
    fn init(&self, _: &mut Context<'_>) -> rusqlite::Result<Fingerprint> {
        Ok(Fingerprint::default())
    }
    fn step(&self, ctx: &mut Context<'_>, state: &mut Fingerprint) -> rusqlite::Result<()> {
        if ctx.get_raw(5).as_str()?.len() > super::_t3_conversations::MAX_RECORD_BYTES {
            return Err(rusqlite::Error::UserFunctionError(Box::new(
                std::io::Error::other("source record exceeds size bound"),
            )));
        }
        let mut hash = Sha256::new();
        for index in 0..ctx.len() {
            hash_value(&mut hash, ctx.get_raw(index));
        }
        state.add(&Fingerprint {
            rows: 1,
            digest: hash.finalize().into(),
        });
        Ok(())
    }
    fn finalize(
        &self,
        _: &mut Context<'_>,
        state: Option<Fingerprint>,
    ) -> rusqlite::Result<String> {
        serde_json::to_string(&state.unwrap_or_default())
            .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))
    }
}
pub(super) fn register(db: &Connection) -> Result<(), SourceError> {
    db.create_aggregate_function(
        FUNCTION,
        8,
        FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
        ContentFingerprint,
    )?;
    Ok(())
}
pub(super) fn merge(map: &mut BTreeMap<String, Fingerprint>, thread: String, value: Fingerprint) {
    map.entry(thread).or_default().add(&value);
}
