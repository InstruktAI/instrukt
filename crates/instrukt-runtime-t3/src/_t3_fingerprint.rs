//! Content-sensitive per-conversation discovery in short, read-only row scopes (cold text source only).
use std::collections::BTreeMap;

use instrukt_core::state_db::rusqlite::types::ValueRef;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Fingerprint {
    rows: u64,
    digest: [u8; 32],
}

impl Fingerprint {
    pub(crate) fn add(&mut self, other: &Self) {
        self.rows += other.rows;
        for (target, value) in self.digest.iter_mut().zip(other.digest) {
            *target ^= value;
        }
    }

    pub(crate) fn row(values: [ValueRef<'_>; 8]) -> Self {
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

pub(crate) fn merge(map: &mut BTreeMap<String, Fingerprint>, thread: String, value: Fingerprint) {
    map.entry(thread).or_default().add(&value);
}
