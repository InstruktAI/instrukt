//! Sidecar-free SQLite inspection. A stable main/WAL image is reconstructed in
//! memory through the last checksum-valid commit, then opened by SQLite itself.
//! WAL framing/checksums: https://www.sqlite.org/fileformat2.html#walformat
use super::{Connection, StoreError};
use std::path::{Path, PathBuf};
pub(super) fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    name.into()
}
fn bytes(store: &'static str, path: &Path) -> Result<Vec<u8>, StoreError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(StoreError::invalid(store, path, e.to_string())),
    }
}
pub(super) fn read(store: &'static str, path: &Path) -> Result<Connection, StoreError> {
    let wal_path = sidecar(path, "-wal");
    let journal = sidecar(path, "-journal");
    for _ in 0..8 {
        let main = bytes(store, path)?;
        let wal = bytes(store, &wal_path)?;
        let rollback = bytes(store, &journal)?;
        if main != bytes(store, path)?
            || wal != bytes(store, &wal_path)?
            || rollback != bytes(store, &journal)?
        {
            continue;
        }
        if rollback.iter().any(|b| *b != 0) {
            return Err(StoreError::invalid(
                store,
                path,
                "rollback recovery is pending; inspect after the writing operation completes",
            ));
        }
        let image = fold(store, path, main, &wal)?;
        let sql = StoreError::sqlite(store, path);
        let mut conn = Connection::open_in_memory().map_err(&sql)?;
        conn.pragma_update(None, "temp_store", "MEMORY")
            .map_err(&sql)?;
        conn.deserialize_read_exact(
            super::rusqlite::MAIN_DB,
            image.as_slice(),
            image.len(),
            false,
        )
        .map_err(&sql)?;
        return Ok(conn);
    }
    Err(StoreError::invalid(
        store,
        path,
        "state changed during inspection; retry the read",
    ))
}
fn be(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
}
fn checksum(bytes: &[u8], big: bool, mut sum: (u32, u32)) -> (u32, u32) {
    for words in bytes.as_chunks::<8>().0 {
        let word = |part: &[u8]| {
            if big {
                be(part)
            } else {
                u32::from_le_bytes([part[0], part[1], part[2], part[3]])
            }
        };
        sum.0 = sum.0.wrapping_add(word(&words[..4])).wrapping_add(sum.1);
        sum.1 = sum.1.wrapping_add(word(&words[4..])).wrapping_add(sum.0);
    }
    sum
}
fn fold(
    store: &'static str,
    path: &Path,
    mut main: Vec<u8>,
    wal: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let invalid = || {
        StoreError::invalid(
            store,
            path,
            "invalid SQLite/WAL snapshot; retry after the writer completes recovery",
        )
    };
    if main.len() < 100 || &main[..16] != b"SQLite format 3\0" {
        return Err(invalid());
    }
    if !wal.is_empty() {
        if wal.len() < 32 {
            return Err(invalid());
        }
        let magic = be(wal);
        if !matches!(magic, 0x377f0682 | 0x377f0683) || be(&wal[4..]) != 3_007_000 {
            return Err(invalid());
        }
        let big = magic & 1 != 0;
        let size = be(&wal[8..]) as usize;
        if !(512..=65536).contains(&size) || !size.is_power_of_two() {
            return Err(invalid());
        }
        let mut sum = checksum(&wal[..24], big, (0, 0));
        if sum != (be(&wal[24..]), be(&wal[28..])) {
            return Err(invalid());
        }
        let mut pending = Vec::new();
        for frame in wal[32..].chunks_exact(size + 24) {
            if frame[8..16] != wal[16..24] {
                break;
            }
            let pg = be(frame) as usize;
            if pg == 0 {
                break;
            }
            let next = checksum(&frame[24..], big, checksum(&frame[..8], big, sum));
            if next != (be(&frame[16..]), be(&frame[20..])) {
                break;
            }
            sum = next;
            pending.push((pg, &frame[24..]));
            let pages = be(&frame[4..]) as usize;
            if pages != 0 {
                let length = pages.checked_mul(size).ok_or_else(invalid)?;
                if length > main.len().saturating_add(wal.len()) {
                    return Err(invalid());
                }
                main.resize(length, 0);
                for (pg, data) in pending.drain(..) {
                    if pg <= pages {
                        let offset = (pg - 1).checked_mul(size).ok_or_else(invalid)?;
                        main[offset..offset + size].copy_from_slice(data);
                    }
                }
            }
        }
    }
    if main.len() < 100 {
        return Err(invalid());
    }
    // The WAL has already been applied; the deserialized image has no disk journal.
    main[18] = 1;
    main[19] = 1;
    Ok(main)
}
