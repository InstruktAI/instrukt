//! The fleet membership record (`project/instrukt/spec/domain/runner`,
//! "Fleet query and membership"): one hash in cluster Redis, a field per
//! member's computer identity holding when it was last seen. Each computer
//! writes only its own field; nothing removes one.

use instrukt_core::machine::MachineId;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use super::_query::Fleet;
use crate::transport::{Endpoint, EventScope, TransportError};

/// The record's key.
const MEMBERS: &str = "instrukt:fleet:members";

/// One member's entry value.
#[derive(Serialize, Deserialize)]
struct Entry {
    last_seen: Timestamp,
}

/// The membership record as read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Members {
    /// Every entry that names a computer identity, by identity.
    pub members: Vec<Member>,
    /// How many entries name no computer identity.
    pub unidentified: usize,
}

/// One member on the record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub computer: MachineId,
    pub last_seen: LastSeen,
}

/// When a member was last seen, as its entry says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastSeen {
    At(Timestamp),
    /// The entry's time cannot be read.
    Unknown,
}

/// Why the record could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembersUnread {
    /// The cluster store could not be reached.
    Unreachable,
    /// The store answered but refused the read.
    Refused,
}

/// Sets `computer`'s own entry: last seen at `now`. One command, its own
/// field only.
pub(crate) fn write(
    endpoint: &Endpoint,
    computer: &MachineId,
    now: Timestamp,
) -> Result<(), TransportError> {
    let value = serde_json::to_string(&Entry { last_seen: now }).map_err(|error| {
        TransportError::Redis {
            source_id: endpoint.identity.clone(),
            kind: format!("membership entry not encoded: {error}"),
            class: crate::transport::RedisFailure::Reply,
        }
    })?;
    endpoint.command::<()>(
        redis::cmd("HSET")
            .arg(MEMBERS)
            .arg(computer.as_str())
            .arg(value),
    )
}

/// The record from its fields as found: an entry naming no computer
/// identity is counted, one whose time cannot be read is a member last seen
/// at an unknown time. This is the one place record fields are read.
fn decode(fields: Vec<(String, String)>) -> Members {
    let mut members = Members::default();
    for (field, value) in fields {
        let Some(computer) = MachineId::parse(&field) else {
            members.unidentified += 1;
            continue;
        };
        let last_seen = serde_json::from_str::<Entry>(&value)
            .map_or(LastSeen::Unknown, |entry| LastSeen::At(entry.last_seen));
        members.members.push(Member {
            computer,
            last_seen,
        });
    }
    members.members.sort_by(|a, b| a.computer.cmp(&b.computer));
    members
}

impl Fleet {
    /// Keeps this computer's entry on the membership record: `computer`
    /// last seen at `now`.
    pub fn keep_member(&self, computer: &MachineId, now: Timestamp) -> Result<(), TransportError> {
        write(self.endpoint(), computer, now)
    }

    /// The whole membership record, in one read.
    pub fn members(&self) -> Result<Members, MembersUnread> {
        self.endpoint()
            .command::<Vec<(String, String)>>(redis::cmd("HGETALL").arg(MEMBERS))
            .map(decode)
            .map_err(|error| {
                tracing::warn!(event = "fleet_members_unread", %error);
                if error.is_connection_failure() {
                    MembersUnread::Unreachable
                } else {
                    MembersUnread::Refused
                }
            })
    }

    fn endpoint(&self) -> &Endpoint {
        self.transport().endpoint(EventScope::Cluster)
    }
}

#[cfg(test)]
mod tests {
    use super::{LastSeen, Member, Members, decode};
    use instrukt_core::machine::MachineId;
    use jiff::Timestamp;

    #[test]
    fn entries_are_read_as_found_and_never_fail_the_read() {
        let seen: Timestamp = "2026-11-26T10:00:00Z".parse().unwrap();
        let members = decode(vec![
            (
                "bbbbbbbbbbbbbbbb".into(),
                r#"{"last_seen":"2026-11-26T10:00:00Z"}"#.into(),
            ),
            ("not a computer".into(), r#"{"last_seen":"x"}"#.into()),
            ("aaaaaaaaaaaaaaaa".into(), "{".into()),
            (
                "cccccccccccccccc".into(),
                r#"{"last_seen":"2026-11-26T10:00:00Z","later":1}"#.into(),
            ),
        ]);
        let id = |id: &str| MachineId::parse(id).unwrap();
        assert_eq!(
            members,
            Members {
                members: vec![
                    Member {
                        computer: id("aaaaaaaaaaaaaaaa"),
                        last_seen: LastSeen::Unknown,
                    },
                    Member {
                        computer: id("bbbbbbbbbbbbbbbb"),
                        last_seen: LastSeen::At(seen),
                    },
                    Member {
                        computer: id("cccccccccccccccc"),
                        last_seen: LastSeen::At(seen),
                    },
                ],
                unidentified: 1,
            }
        );
    }
}
