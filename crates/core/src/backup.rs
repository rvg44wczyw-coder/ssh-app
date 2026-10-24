//! Backup file: export and import of the app's data (spec 56 section 5).
//!
//! The Rust side does the format and the cryptography and treats a group's content as opaque JSON
//! text that the app builds and reads; what a group means is the app's business. A file is
//!
//! * plain JSON (`format: "SSHBK"`, a SHA-256 checksum over the groups) when no group is secret, or
//! * an `SSHB` container (Argon2id + ChaCha20-Poly1305, `vault_crypto`) around the same JSON when any
//!   group is secret (private keys, secret values, hook commands).
//!
//! Import is for untrusted files: a size cap, a depth cap, KDF parameters checked before the
//! derivation runs, a plain file must not hold secret groups (it could not have come from our
//! export), group ids and counts bounded. Errors never carry passphrase or content text.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::error::SshCoreError;
use crate::vault_crypto::{open_container, require_acceptable, seal_container, KdfParams, Kind};

/// Largest file or decrypted body accepted (spec: 4 MB).
pub const MAX_BACKUP_BYTES: usize = 4 * 1024 * 1024;
const MAX_DEPTH: usize = 24;
const MAX_GROUPS: usize = 32;
const SCHEMA: u64 = 1;
const FORMAT: &str = "SSHBK";
/// Groups that are secret whatever a file says about them: the `secret` flag in a file is the file's
/// own claim, and a forged plain file must not smuggle keys or commands past the encryption rule.
pub const SECRET_GROUP_IDS: [&str; 3] = ["private-keys", "secret-values", "hook-commands"];

fn is_secret_group(id: &str, flag: bool) -> bool {
    flag || SECRET_GROUP_IDS.contains(&id)
}

/// One group of a backup: `id` names it (`servers`, `profiles`, ...), `json` is its content as JSON text.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BackupGroup {
    pub id: String,
    /// Holds secrets: forces the whole file to be encrypted.
    pub secret: bool,
    pub json: String,
}

/// What a backup file holds, after it was checked.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct BackupContents {
    pub created_at_ms: u64,
    pub app_version: String,
    /// The file was encrypted (a passphrase was needed to read it).
    pub encrypted: bool,
    pub groups: Vec<BackupGroup>,
}

fn bad(reason: &str) -> SshCoreError {
    SshCoreError::VaultDataInvalid {
        reason: reason.to_string(),
    }
}

fn valid_group_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 40
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn depth_of(v: &Value, depth: usize) -> usize {
    match v {
        Value::Array(a) => a
            .iter()
            .map(|x| depth_of(x, depth + 1))
            .max()
            .unwrap_or(depth + 1),
        Value::Object(o) => o
            .values()
            .map(|x| depth_of(x, depth + 1))
            .max()
            .unwrap_or(depth + 1),
        _ => depth,
    }
}

/// SHA-256 over the groups as they are written (id order is the map order, which is stable).
fn checksum(groups: &Map<String, Value>) -> String {
    hex::encode(Sha256::digest(
        serde_json::to_string(groups).unwrap_or_default().as_bytes(),
    ))
}

fn envelope(
    groups: &[BackupGroup],
    created_at_ms: u64,
    app_version: &str,
) -> Result<Map<String, Value>, SshCoreError> {
    if groups.is_empty() || groups.len() > MAX_GROUPS {
        return Err(bad("a backup holds between 1 and 32 groups"));
    }
    let mut map = Map::new();
    for g in groups {
        if !valid_group_id(&g.id) || map.contains_key(&g.id) {
            return Err(bad("a group id is invalid or repeated"));
        }
        let data: Value =
            serde_json::from_str(&g.json).map_err(|_| bad("a group is not valid JSON"))?;
        if depth_of(&data, 0) > MAX_DEPTH {
            return Err(bad("a group is nested too deeply"));
        }
        map.insert(
            g.id.clone(),
            json!({"secret": is_secret_group(&g.id, g.secret), "data": data}),
        );
    }
    let mut env = Map::new();
    env.insert("format".into(), json!(FORMAT));
    env.insert("schema".into(), json!(SCHEMA));
    env.insert("created".into(), json!(created_at_ms));
    env.insert(
        "app_version".into(),
        json!(app_version.chars().take(40).collect::<String>()),
    );
    env.insert("checksum".into(), json!(checksum(&map)));
    env.insert("groups".into(), Value::Object(map));
    Ok(env)
}

/// Builds a backup file. A secret group needs `passphrase` (the same minimum as the vault's).
#[uniffi::export]
pub fn backup_export(
    groups: Vec<BackupGroup>,
    passphrase: Option<Vec<u8>>,
    created_at_ms: u64,
    app_version: String,
) -> Result<Vec<u8>, SshCoreError> {
    export_with(
        groups,
        passphrase,
        created_at_ms,
        app_version,
        KdfParams::DEFAULT,
    )
}

fn export_with(
    groups: Vec<BackupGroup>,
    passphrase: Option<Vec<u8>>,
    created_at_ms: u64,
    app_version: String,
    params: KdfParams,
) -> Result<Vec<u8>, SshCoreError> {
    let env = envelope(&groups, created_at_ms, &app_version)?;
    let body = serde_json::to_vec(&Value::Object(env))
        .map_err(|_| bad("the backup could not be written"))?;
    if body.len() > MAX_BACKUP_BYTES {
        return Err(bad("the backup is too large"));
    }
    if groups.iter().any(|g| is_secret_group(&g.id, g.secret)) {
        let phrase =
            zeroize::Zeroizing::new(passphrase.ok_or_else(|| SshCoreError::InvalidInput {
                reason: "a backup with secrets needs a passphrase".to_string(),
            })?);
        require_acceptable(&phrase)?;
        seal_container(Kind::Backup, &phrase, &body, created_at_ms, params)
    } else {
        Ok(body)
    }
}

/// Whether a file is an encrypted backup (so the app knows to ask for the passphrase).
#[uniffi::export]
pub fn backup_is_encrypted(file: Vec<u8>) -> bool {
    file.starts_with(b"SSHB")
}

/// Reads and checks a backup file. For an encrypted one `passphrase` is needed; a wrong one and a
/// damaged file give the same error. Nothing is applied: the app shows the preview first.
#[uniffi::export]
pub fn backup_open(
    file: Vec<u8>,
    passphrase: Option<Vec<u8>>,
) -> Result<BackupContents, SshCoreError> {
    if file.len() > MAX_BACKUP_BYTES + 4096 {
        return Err(bad("the file is too large to be a backup"));
    }
    let encrypted = backup_is_encrypted(file.clone());
    let body: zeroize::Zeroizing<Vec<u8>> = if encrypted {
        let phrase =
            zeroize::Zeroizing::new(passphrase.ok_or_else(|| SshCoreError::InvalidInput {
                reason: "this backup is encrypted; a passphrase is needed".to_string(),
            })?);
        open_container(Kind::Backup, &file, &phrase)?
    } else {
        zeroize::Zeroizing::new(file)
    };
    if body.len() > MAX_BACKUP_BYTES {
        return Err(bad("the backup is too large"));
    }
    let v: Value = serde_json::from_slice(&body).map_err(|_| bad("this is not a backup file"))?;
    let obj = v
        .as_object()
        .ok_or_else(|| bad("this is not a backup file"))?;
    if obj.get("format").and_then(Value::as_str) != Some(FORMAT) {
        return Err(bad("this is not a backup file"));
    }
    if obj.get("schema").and_then(Value::as_u64) != Some(SCHEMA) {
        return Err(bad("unsupported backup version"));
    }
    if depth_of(&v, 0) > MAX_DEPTH + 4 {
        return Err(bad("the backup is nested too deeply"));
    }
    let groups_obj = obj
        .get("groups")
        .and_then(Value::as_object)
        .ok_or_else(|| bad("this is not a backup file"))?;
    if groups_obj.is_empty() || groups_obj.len() > MAX_GROUPS {
        return Err(bad("a backup holds between 1 and 32 groups"));
    }
    if obj.get("checksum").and_then(Value::as_str) != Some(checksum(groups_obj).as_str()) {
        return Err(bad("the backup is damaged (checksum)"));
    }
    let mut groups = Vec::with_capacity(groups_obj.len());
    for (id, g) in groups_obj {
        if !valid_group_id(id) {
            return Err(bad("a group id is invalid"));
        }
        let secret = is_secret_group(id, g.get("secret").and_then(Value::as_bool).unwrap_or(true));
        // A plain file cannot hold secrets: our export never writes one, so it is forged or damaged.
        if secret && !encrypted {
            return Err(bad("a plain backup must not hold secret groups"));
        }
        let data = g.get("data").ok_or_else(|| bad("a group has no data"))?;
        groups.push(BackupGroup {
            id: id.clone(),
            secret,
            json: serde_json::to_string(data).map_err(|_| bad("a group could not be read"))?,
        });
    }
    Ok(BackupContents {
        created_at_ms: obj.get("created").and_then(Value::as_u64).unwrap_or(0),
        app_version: obj
            .get("app_version")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(40)
            .collect(),
        encrypted,
        groups,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PHRASE: &[u8] = b"correct horse battery staple";
    const FAST: KdfParams = KdfParams {
        m_kib: 32 * 1024,
        t: 2,
        p: 1,
    };

    fn group(id: &str, secret: bool, json: &str) -> BackupGroup {
        BackupGroup {
            id: id.into(),
            secret,
            json: json.into(),
        }
    }

    fn export(groups: Vec<BackupGroup>, phrase: Option<&[u8]>) -> Result<Vec<u8>, SshCoreError> {
        export_with(
            groups,
            phrase.map(<[u8]>::to_vec),
            1_700_000_000_000,
            "1.0".into(),
            FAST,
        )
    }

    #[test]
    fn a_plain_backup_round_trips_and_is_readable_json() {
        let file = export(vec![group("snippets", false, r#"[{"t":"ls"}]"#)], None).unwrap();
        assert!(!backup_is_encrypted(file.clone()));
        assert!(String::from_utf8(file.clone()).unwrap().contains("SSHBK"));
        let c = backup_open(file, None).unwrap();
        assert!(!c.encrypted);
        assert_eq!(c.groups, vec![group("snippets", false, r#"[{"t":"ls"}]"#)]);
        assert_eq!(c.app_version, "1.0");
        assert_eq!(c.created_at_ms, 1_700_000_000_000);
    }

    #[test]
    fn secrets_force_encryption_and_hide_their_content() {
        let key = r#"{"pem":"-----BEGIN-----SECRETKEY"}"#;
        let file = export(
            vec![group("keys", true, key), group("servers", false, "[]")],
            Some(PHRASE),
        )
        .unwrap();
        assert!(backup_is_encrypted(file.clone()));
        assert!(!String::from_utf8_lossy(&file).contains("SECRETKEY"));
        let c = backup_open(file, Some(PHRASE.to_vec())).unwrap();
        assert!(c.encrypted);
        assert_eq!(c.groups.len(), 2);
        assert!(c
            .groups
            .iter()
            .any(|g| g.id == "keys" && g.secret && g.json.contains("SECRETKEY")));
    }

    #[test]
    fn secrets_without_a_passphrase_or_with_a_weak_one_are_refused() {
        let g = || vec![group("keys", true, "{}")];
        assert!(matches!(
            export(g(), None),
            Err(SshCoreError::InvalidInput { .. })
        ));
        assert!(matches!(
            export(g(), Some(b"short")),
            Err(SshCoreError::InvalidInput { .. })
        ));
    }

    #[test]
    fn wrong_passphrase_and_damage_look_the_same() {
        let file = export(vec![group("keys", true, "{}")], Some(PHRASE)).unwrap();
        assert!(matches!(
            backup_open(file.clone(), Some(b"another long phrase here".to_vec())),
            Err(SshCoreError::VaultWrongPassphrase)
        ));
        let mut cut = file.clone();
        let last = cut.len() - 1;
        cut[last] ^= 1;
        assert!(matches!(
            backup_open(cut, Some(PHRASE.to_vec())),
            Err(SshCoreError::VaultWrongPassphrase)
        ));
        assert!(matches!(
            backup_open(file, None),
            Err(SshCoreError::InvalidInput { .. })
        ));
    }

    #[test]
    fn a_plain_file_claiming_secrets_a_bad_checksum_or_a_foreign_format_are_refused() {
        let plain = |secret: bool| {
            let mut env = envelope(&[group("keys", secret, "{}")], 1, "1").unwrap();
            let g = env["groups"].as_object().unwrap().clone();
            env.insert("checksum".into(), json!(checksum(&g)));
            serde_json::to_vec(&Value::Object(env)).unwrap()
        };
        assert!(backup_open(plain(false), None).is_ok());
        assert!(
            backup_open(plain(true), None).is_err(),
            "forged: secrets in plain text"
        );
        let mut v: Value = serde_json::from_slice(&plain(false)).unwrap();
        v["groups"]["keys"]["data"] = json!({"x": 1});
        assert!(
            backup_open(serde_json::to_vec(&v).unwrap(), None).is_err(),
            "checksum"
        );
        assert!(backup_open(
            br#"{"format":"other","schema":1,"groups":{}}"#.to_vec(),
            None
        )
        .is_err());
        assert!(backup_open(
            br#"{"format":"SSHBK","schema":2,"groups":{"a":{}}}"#.to_vec(),
            None
        )
        .is_err());
        assert!(backup_open(b"not json".to_vec(), None).is_err());
    }

    #[test]
    fn hostile_files_are_bounded() {
        // Too large, before any parsing.
        assert!(backup_open(vec![b' '; MAX_BACKUP_BYTES + 5000], None).is_err());
        // Too deep.
        let deep = format!("{}1{}", "[".repeat(40), "]".repeat(40));
        assert!(export(vec![group("a", false, &deep)], None).is_err());
        // Too many groups, bad ids.
        let many: Vec<BackupGroup> = (0..33)
            .map(|i| group(&format!("g{i}"), false, "1"))
            .collect();
        assert!(export(many, None).is_err());
        assert!(export(vec![group("Bad Id", false, "1")], None).is_err());
        assert!(export(vec![group("a", false, "1"), group("a", false, "2")], None).is_err());
        // KDF parameters in a hostile container are refused before any derivation runs.
        let mut file = export(vec![group("keys", true, "{}")], Some(PHRASE)).unwrap();
        file[6..10].copy_from_slice(&u32::MAX.to_be_bytes()); // m_kib huge
        let started = std::time::Instant::now();
        assert!(backup_open(file.clone(), Some(PHRASE.to_vec())).is_err());
        file[6..10].copy_from_slice(&1u32.to_be_bytes()); // m_kib tiny
        assert!(backup_open(file, Some(PHRASE.to_vec())).is_err());
        assert!(started.elapsed().as_secs() < 2, "no derivation ran");
    }

    #[test]
    fn known_secret_groups_are_secret_whatever_the_flag_says() {
        // Export: a flag left off still forces encryption.
        assert!(matches!(
            export(vec![group("private-keys", false, "{}")], None),
            Err(SshCoreError::InvalidInput { .. })
        ));
        // Import: a plain file that calls the keys group harmless is refused.
        let mut env = envelope(&[group("servers", false, "[]")], 1, "1").unwrap();
        let mut groups = env["groups"].as_object().unwrap().clone();
        groups.insert("private-keys".into(), json!({"secret": false, "data": {}}));
        env.insert("checksum".into(), json!(checksum(&groups)));
        env.insert("groups".into(), Value::Object(groups));
        assert!(backup_open(serde_json::to_vec(&Value::Object(env)).unwrap(), None).is_err());
    }

    #[test]
    fn a_vault_container_does_not_open_as_a_backup() {
        let vault = seal_container(Kind::Vault, PHRASE, b"{}", 1, FAST).unwrap();
        assert!(backup_open(vault, Some(PHRASE.to_vec())).is_err());
    }
}
