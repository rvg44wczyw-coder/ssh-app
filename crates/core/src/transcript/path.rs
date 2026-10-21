use super::*;

/// Validates whether a remote path is safe to read.
pub fn validate_transcript_path(path: &str) -> Result<(), TranscriptError> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(TranscriptError::InvalidPath {
            path: path.to_string(),
            reason: "Path cannot be empty".to_string(),
        });
    }
    if trimmed.starts_with('-') {
        return Err(TranscriptError::InvalidPath {
            path: path.to_string(),
            reason: "Path cannot start with '-'".to_string(),
        });
    }
    if trimmed.contains("..") {
        return Err(TranscriptError::InvalidPath {
            path: path.to_string(),
            reason: "Path traversal ('..') is prohibited".to_string(),
        });
    }
    if trimmed.chars().any(|c| c.is_control()) || trimmed.contains('\'') {
        return Err(TranscriptError::InvalidPath {
            path: path.to_string(),
            reason: "Path contains illegal characters".to_string(),
        });
    }
    // Block sensitive system and credential directories (both prefix and substring)
    let forbidden_prefixes = [
        "/etc/",
        "/var/",
        "/dev/",
        "/proc/",
        "/sys/",
        "/private/etc/",
        "/private/var/",
        "~/.ssh/",
        "~/.gnupg/",
        "~/.aws/",
    ];
    for prefix in forbidden_prefixes {
        if trimmed.starts_with(prefix) {
            return Err(TranscriptError::InvalidPath {
                path: path.to_string(),
                reason: format!("Access to '{}' is prohibited", prefix),
            });
        }
    }
    let forbidden_segments = ["/.ssh/", "/.gnupg/", "/.aws/", "/Library/Keychains/"];
    for segment in forbidden_segments {
        if trimmed.contains(segment) {
            return Err(TranscriptError::InvalidPath {
                path: path.to_string(),
                reason: format!("Access to '{}' is prohibited", segment),
            });
        }
    }
    Ok(())
}
