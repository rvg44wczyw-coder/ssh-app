//! Merge of the synced vault (feature 43): the server list and anything else that travels through
//! the end-to-end encrypted iCloud Keychain item. The core does not look inside a payload; it only
//! decides, per entry, which side wins, so two devices that merge each other's copy end identical.
//! Private keys never come here.

use std::collections::BTreeMap;

/// A payload above this is dropped: the other side is not trusted to be well-formed.
pub const MAX_PAYLOAD_BYTES: usize = 16 * 1024;
pub const MAX_ENTRIES: usize = 1000;
pub const MAX_ID_LEN: usize = 64;
/// A deletion marker is kept this long so a device that was offline does not bring the entry back.
/// A remote entry stamped further ahead than this is ignored: a wrong clock, or someone trying to
/// make their copy win every later edit.
pub const MAX_CLOCK_SKEW_MS: u64 = 24 * 60 * 60 * 1000;
pub const TOMBSTONE_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultEntry {
    pub id: String,
    pub updated_at_ms: u64,
    pub deleted: bool,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VaultMerge {
    /// The converged list, ordered by id, tombstones included until they expire.
    pub merged: Vec<VaultEntry>,
    /// Ids whose winning copy came from the other side and differs from the local one.
    pub taken_from_remote: Vec<String>,
}

fn acceptable(e: &VaultEntry) -> bool {
    !e.id.is_empty()
        && e.id.len() <= MAX_ID_LEN
        && e.id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && e.payload.len() <= MAX_PAYLOAD_BYTES
}

/// Last writer wins by `updated_at_ms`; on a tie a deletion wins, then the larger payload, so the
/// result does not depend on which side is "local".
fn newer(a: &VaultEntry, b: &VaultEntry) -> bool {
    (a.updated_at_ms, a.deleted, &a.payload) > (b.updated_at_ms, b.deleted, &b.payload)
}

/// Merges `remote` into `local`. Entries that are malformed or beyond the limits are ignored.
#[uniffi::export]
pub fn merge_vault(local: Vec<VaultEntry>, remote: Vec<VaultEntry>, now_ms: u64) -> VaultMerge {
    let mut by_id: BTreeMap<String, VaultEntry> = BTreeMap::new();
    for e in local.iter().filter(|e| acceptable(e)) {
        by_id.insert(e.id.clone(), e.clone());
    }
    let mut taken = Vec::new();
    let latest = now_ms.saturating_add(MAX_CLOCK_SKEW_MS);
    for r in remote
        .into_iter()
        .filter(|r| acceptable(r) && r.updated_at_ms <= latest)
        .take(MAX_ENTRIES)
    {
        match by_id.get(&r.id) {
            Some(l) if !newer(&r, l) => {}
            Some(_) => {
                taken.push(r.id.clone());
                by_id.insert(r.id.clone(), r);
            }
            None if by_id.len() < MAX_ENTRIES => {
                taken.push(r.id.clone());
                by_id.insert(r.id.clone(), r);
            }
            None => {}
        }
    }
    let merged = by_id
        .into_values()
        .filter(|e| !(e.deleted && now_ms.saturating_sub(e.updated_at_ms) > TOMBSTONE_TTL_MS))
        .collect();
    taken.sort();
    VaultMerge {
        merged,
        taken_from_remote: taken,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(id: &str, at: u64, deleted: bool, payload: &str) -> VaultEntry {
        VaultEntry {
            id: id.into(),
            updated_at_ms: at,
            deleted,
            payload: payload.into(),
        }
    }

    #[test]
    fn the_later_write_wins_and_new_entries_arrive() {
        let local = vec![e("a", 10, false, "old"), e("b", 5, false, "b")];
        let remote = vec![e("a", 20, false, "new"), e("c", 1, false, "c")];
        let m = merge_vault(local, remote, 100);
        let ids: Vec<_> = m
            .merged
            .iter()
            .map(|x| (x.id.as_str(), x.payload.as_str()))
            .collect();
        assert_eq!(ids, [("a", "new"), ("b", "b"), ("c", "c")]);
        assert_eq!(m.taken_from_remote, ["a", "c"]);
    }

    #[test]
    fn an_older_remote_copy_changes_nothing() {
        let m = merge_vault(
            vec![e("a", 20, false, "mine")],
            vec![e("a", 10, false, "theirs")],
            100,
        );
        assert_eq!(m.merged[0].payload, "mine");
        assert!(m.taken_from_remote.is_empty());
    }

    #[test]
    fn both_sides_converge_whichever_is_local() {
        let x = vec![e("a", 10, false, "x"), e("b", 7, true, "")];
        let y = vec![e("a", 10, false, "y"), e("b", 7, false, "b")];
        let one = merge_vault(x.clone(), y.clone(), 50).merged;
        let two = merge_vault(y, x, 50).merged;
        assert_eq!(one, two);
        assert!(one.iter().find(|v| v.id == "b").unwrap().deleted);
    }

    #[test]
    fn a_deletion_beats_an_older_edit_and_expires_later() {
        let m = merge_vault(vec![e("a", 5, false, "p")], vec![e("a", 9, true, "")], 10);
        assert!(m.merged[0].deleted);
        let gone = merge_vault(m.merged, vec![], 9 + TOMBSTONE_TTL_MS + 1);
        assert!(gone.merged.is_empty());
    }

    #[test]
    fn hostile_remote_entries_are_dropped() {
        let big = "x".repeat(MAX_PAYLOAD_BYTES + 1);
        let remote = vec![
            e("", 1, false, "p"),
            e("../etc", 1, false, "p"),
            e(&"i".repeat(MAX_ID_LEN + 1), 1, false, "p"),
            e("big", 1, false, &big),
            e("ok", 1, false, "p"),
        ];
        let m = merge_vault(vec![], remote, 10);
        assert_eq!(m.merged.len(), 1);
        assert_eq!(m.merged[0].id, "ok");
    }

    #[test]
    fn an_entry_stamped_in_the_far_future_is_ignored() {
        let local = vec![e("a", 10, false, "mine")];
        let remote = vec![
            e("a", u64::MAX, false, "pinned"),
            e("b", 11 + MAX_CLOCK_SKEW_MS, false, "p"),
        ];
        let m = merge_vault(local, remote, 10);
        assert_eq!(m.merged.len(), 1);
        assert_eq!(m.merged[0].payload, "mine");
    }

    #[test]
    fn the_entry_count_is_capped() {
        let remote: Vec<_> = (0..MAX_ENTRIES + 50)
            .map(|i| e(&format!("id{i}"), 1, false, "p"))
            .collect();
        assert_eq!(merge_vault(vec![], remote, 10).merged.len(), MAX_ENTRIES);
    }
}
