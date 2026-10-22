//! The app's module settings as one typed document (feature 46). Only modules whose feature
//! exists are in it (Docker, uploads); a module is added together with its feature. The document
//! is stored by the app as JSON, can be exported and imported, and an imported file is hostile
//! input: unknown keys, out-of-range values and anything that would loosen a security switch are
//! refused or kept at their current value, and nothing is silently clamped.

use crate::error::SshCoreError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const CONFIG_VERSION: u32 = 1;
/// A file larger than this is not a settings file.
const MAX_FILE_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields, default)]
pub struct DockerModuleConfig {
    /// Lines of a container log fetched (50..=1000).
    pub log_tail_lines: u32,
    /// Seconds between refreshes of the container list while it is open; 0 = only when asked.
    pub auto_refresh_secs: u32,
}

impl Default for DockerModuleConfig {
    fn default() -> Self {
        Self {
            log_tail_lines: 200,
            auto_refresh_secs: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields, default)]
pub struct UploadsModuleConfig {
    /// Longest side of a picture sent to the host, in pixels (256..=4096).
    pub max_dimension: u32,
    /// JPEG quality in percent (30..=100).
    pub jpeg_quality_percent: u32,
    /// Remove location and device data from pictures before they leave the phone. Security
    /// switch: an import can never turn it off.
    pub strip_metadata: bool,
}

impl Default for UploadsModuleConfig {
    fn default() -> Self {
        Self {
            max_dimension: 2048,
            jpeg_quality_percent: 85,
            strip_metadata: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
#[serde(deny_unknown_fields, default)]
pub struct AppConfigRoot {
    pub version: u32,
    pub docker: DockerModuleConfig,
    pub uploads: UploadsModuleConfig,
}

impl Default for AppConfigRoot {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            docker: DockerModuleConfig::default(),
            uploads: UploadsModuleConfig::default(),
        }
    }
}

fn bad(key: &str, rule: &str) -> SshCoreError {
    SshCoreError::InvalidInput {
        reason: format!("{key} {rule}"),
    }
}

impl AppConfigRoot {
    /// Names the first key that is out of range.
    pub fn validate(&self) -> Result<(), SshCoreError> {
        if self.version != CONFIG_VERSION {
            return Err(bad("version", &format!("must be {CONFIG_VERSION}")));
        }
        if !(50..=1000).contains(&self.docker.log_tail_lines) {
            return Err(bad("docker.log_tail_lines", "must be 50 to 1000"));
        }
        let r = self.docker.auto_refresh_secs;
        if r != 0 && !(5..=300).contains(&r) {
            return Err(bad("docker.auto_refresh_secs", "must be 0 or 5 to 300"));
        }
        if !(256..=4096).contains(&self.uploads.max_dimension) {
            return Err(bad("uploads.max_dimension", "must be 256 to 4096"));
        }
        if !(30..=100).contains(&self.uploads.jpeg_quality_percent) {
            return Err(bad("uploads.jpeg_quality_percent", "must be 30 to 100"));
        }
        Ok(())
    }
}

/// The default document.
#[uniffi::export]
pub fn default_app_config() -> AppConfigRoot {
    AppConfigRoot::default()
}

/// Reads the app's own stored document. Damage or a stranger's edit falls back to the defaults
/// (the app keeps working) rather than failing to start.
#[uniffi::export]
pub fn load_app_config(stored_json: String) -> AppConfigRoot {
    if stored_json.len() > MAX_FILE_BYTES {
        return AppConfigRoot::default();
    }
    serde_json::from_str::<AppConfigRoot>(&stored_json)
        .ok()
        .filter(|c| c.validate().is_ok())
        .unwrap_or_default()
}

/// The document as the app stores it.
#[uniffi::export]
pub fn save_app_config(config: AppConfigRoot) -> Result<String, SshCoreError> {
    config.validate()?;
    serde_json::to_string(&config).map_err(|e| SshCoreError::InvalidInput {
        reason: e.to_string(),
    })
}

fn checksum(config_json: &str) -> String {
    hex::encode(Sha256::digest(config_json.as_bytes()))
}

/// The file the user can keep: `{"config": {...}, "sha256": "..."}`. The checksum detects damage;
/// it is not a signature, and nobody should read it as one. The document holds no keys or
/// secrets by construction (the schema has no such field).
#[uniffi::export]
pub fn export_app_config(config: AppConfigRoot) -> Result<String, SshCoreError> {
    config.validate()?;
    let body = serde_json::to_string(&config).map_err(|e| SshCoreError::InvalidInput {
        reason: e.to_string(),
    })?;
    let sum = checksum(&body);
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| SshCoreError::InvalidInput {
            reason: e.to_string(),
        })?;
    serde_json::to_string_pretty(&serde_json::json!({"config": value, "sha256": sum})).map_err(
        |e| SshCoreError::InvalidInput {
            reason: e.to_string(),
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ConfigImport {
    pub config: AppConfigRoot,
    /// Things the user should be told: a security switch that was kept as it is.
    pub notes: Vec<String>,
}

/// Reads an exported file against the current document. The whole file is refused (the error
/// names the key) for: size, syntax, unknown keys, a wrong checksum, a wrong type or an
/// out-of-range value. Security switches are never loosened: they stay as they are and a note says
/// so.
#[uniffi::export]
pub fn import_app_config(
    current: AppConfigRoot,
    file_json: String,
) -> Result<ConfigImport, SshCoreError> {
    if file_json.len() > MAX_FILE_BYTES {
        return Err(bad("the file", "is too large to be a settings file"));
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct File {
        config: AppConfigRoot,
        sha256: Option<String>,
    }
    let file: File = serde_json::from_str(&file_json).map_err(|e| SshCoreError::InvalidInput {
        reason: format!("not a settings file: {e}"),
    })?;
    let mut incoming = file.config;
    if let Some(sum) = file.sha256 {
        let body = serde_json::to_string(&incoming).map_err(|e| SshCoreError::InvalidInput {
            reason: e.to_string(),
        })?;
        if !sum.eq_ignore_ascii_case(&checksum(&body)) {
            return Err(bad(
                "the checksum",
                "does not match: the file is damaged or was edited",
            ));
        }
    }
    incoming.validate()?;
    let mut notes = Vec::new();
    if current.uploads.strip_metadata && !incoming.uploads.strip_metadata {
        incoming.uploads.strip_metadata = true;
        notes.push("uploads.strip_metadata stays on: an import cannot turn it off".to_string());
    }
    Ok(ConfigImport {
        config: incoming,
        notes,
    })
}

/// One module back to its defaults, the rest untouched (`docker` or `uploads`).
#[uniffi::export]
pub fn reset_app_config_module(
    mut config: AppConfigRoot,
    module: String,
) -> Result<AppConfigRoot, SshCoreError> {
    match module.as_str() {
        "docker" => config.docker = DockerModuleConfig::default(),
        "uploads" => config.uploads = UploadsModuleConfig::default(),
        _ => return Err(bad("module", "is not one of: docker, uploads")),
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_with(config: &str) -> String {
        format!("{{\"config\": {config}}}")
    }

    #[test]
    fn the_default_document_is_valid_and_round_trips() {
        let d = AppConfigRoot::default();
        d.validate().unwrap();
        let stored = save_app_config(d.clone()).unwrap();
        assert_eq!(load_app_config(stored), d);
    }

    #[test]
    fn a_damaged_stored_document_means_the_defaults() {
        for stored in [
            "",
            "not json",
            "{\"version\":1,\"nope\":3}",
            "{\"version\":2}",
            "{\"version\":1,\"docker\":{\"log_tail_lines\":9}}",
        ] {
            assert_eq!(
                load_app_config(stored.into()),
                AppConfigRoot::default(),
                "{stored}"
            );
        }
        assert_eq!(
            load_app_config("x".repeat(MAX_FILE_BYTES + 1)),
            AppConfigRoot::default()
        );
    }

    #[test]
    fn an_out_of_range_value_rejects_the_file_and_names_the_key() {
        let cur = AppConfigRoot::default();
        let cases = [
            (
                r#"{"version":1,"docker":{"log_tail_lines":9999999}}"#,
                "docker.log_tail_lines",
            ),
            (
                r#"{"version":1,"docker":{"auto_refresh_secs":2}}"#,
                "docker.auto_refresh_secs",
            ),
            (
                r#"{"version":1,"uploads":{"max_dimension":100000}}"#,
                "uploads.max_dimension",
            ),
            (
                r#"{"version":1,"uploads":{"jpeg_quality_percent":0}}"#,
                "uploads.jpeg_quality_percent",
            ),
            (r#"{"version":7}"#, "version"),
        ];
        for (cfg, key) in cases {
            let e = import_app_config(cur.clone(), file_with(cfg))
                .unwrap_err()
                .to_string();
            assert!(e.contains(key), "{key}: {e}");
        }
    }

    #[test]
    fn hostile_files_are_refused() {
        let cur = AppConfigRoot::default();
        for hostile in [
            file_with(r#"{"version":1,"docker":{"socket":"/var/run/docker.sock"}}"#),
            file_with(r#"{"version":1,"private_key":"-----BEGIN"}"#),
            file_with(r#"{"version":1,"docker":{"log_tail_lines":"200"}}"#),
            file_with(r#"{"version":1,"docker":{"log_tail_lines":-5}}"#),
            r#"{"config":{"version":1},"extra":1}"#.to_string(),
            "[]".to_string(),
            "{".to_string(),
            "x".repeat(MAX_FILE_BYTES + 1),
        ] {
            assert!(
                import_app_config(cur.clone(), hostile.clone()).is_err(),
                "{hostile:.60}"
            );
        }
    }

    #[test]
    fn an_import_cannot_turn_off_the_metadata_stripping() {
        let cur = AppConfigRoot::default();
        let r = import_app_config(
            cur,
            file_with(r#"{"version":1,"uploads":{"strip_metadata":false,"max_dimension":1024}}"#),
        )
        .unwrap();
        assert!(r.config.uploads.strip_metadata);
        assert_eq!(
            r.config.uploads.max_dimension, 1024,
            "other values are taken"
        );
        assert_eq!(r.notes.len(), 1);
        // a user who already turned it off in the app is not forced back on by an import
        let mut off = AppConfigRoot::default();
        off.uploads.strip_metadata = false;
        let r = import_app_config(
            off,
            file_with(r#"{"version":1,"uploads":{"strip_metadata":false}}"#),
        )
        .unwrap();
        assert!(!r.config.uploads.strip_metadata && r.notes.is_empty());
    }

    #[test]
    fn export_then_import_gives_the_same_document_and_the_checksum_catches_damage() {
        let mut c = AppConfigRoot::default();
        c.docker.log_tail_lines = 500;
        c.uploads.jpeg_quality_percent = 70;
        let file = export_app_config(c.clone()).unwrap();
        assert_eq!(
            import_app_config(AppConfigRoot::default(), file.clone())
                .unwrap()
                .config,
            c
        );
        let damaged = file.replace("500", "600");
        let e = import_app_config(AppConfigRoot::default(), damaged)
            .unwrap_err()
            .to_string();
        assert!(e.contains("checksum"), "{e}");
        // a hand-written file without a checksum is accepted
        assert!(import_app_config(AppConfigRoot::default(), file_with(r#"{"version":1}"#)).is_ok());
    }

    #[test]
    fn the_export_holds_no_secrets() {
        let file = export_app_config(AppConfigRoot::default())
            .unwrap()
            .to_ascii_lowercase();
        for word in ["key", "token", "secret", "password", "pem", "begin"] {
            assert!(!file.contains(word), "{word} in {file}");
        }
    }

    #[test]
    fn one_module_resets_alone() {
        let mut c = AppConfigRoot::default();
        c.docker.log_tail_lines = 900;
        c.uploads.max_dimension = 512;
        let r = reset_app_config_module(c.clone(), "docker".into()).unwrap();
        assert_eq!(r.docker, DockerModuleConfig::default());
        assert_eq!(r.uploads.max_dimension, 512);
        assert!(reset_app_config_module(c, "mesh".into()).is_err());
    }
}
