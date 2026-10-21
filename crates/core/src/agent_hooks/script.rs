#[uniffi::export]
pub fn agent_hook_script() -> String {
    let script = include_str!("../../../../scripts/agent-hooks/ssh-app-hook.py");
    // The pin of the embedded typed helper (feature 25): the script hands over to it only when
    // the helper next to it has exactly this digest.
    match crate::usage::bundled_host_helper_sha256() {
        Some(pin) => script.replacen(
            "HELPER_SHA256 = \"\"",
            &format!("HELPER_SHA256 = \"{pin}\""),
            1,
        ),
        None => script.to_string(),
    }
}

/// Which code the agents' hooks run on a host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HookScriptState {
    NotInstalled,
    /// ssh-app's script whose pin is the helper this build ships: hooks run the Rust helper.
    Rust,
    /// ssh-app's script without or with another pin: hooks run the Python code.
    Python,
    /// Not ssh-app's file.
    Foreign,
}

pub(crate) fn classify_hook_script(existing: &str, pin: Option<&str>) -> HookScriptState {
    if existing.trim().is_empty() {
        HookScriptState::NotInstalled
    } else if !is_our_hook_script(existing) {
        HookScriptState::Foreign
    } else if pin.is_some_and(|p| existing.contains(&format!("HELPER_SHA256 = \"{p}\""))) {
        HookScriptState::Rust
    } else {
        HookScriptState::Python
    }
}

/// What to do with the hook script already on a host after the helper changed (its pin is inside
/// the script, so a stale pin silently sends every hook to the Python code).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum HookScriptRefresh {
    /// No script on the host: nothing to keep in step.
    NotInstalled,
    /// Already the script this build ships, pin included.
    UpToDate,
    /// It was ssh-app's own script and was replaced.
    Refreshed,
    /// A file is there that is not ssh-app's script: only replaced when the user says so.
    NeedsConfirmation,
}

/// Whether `existing` is a script of ours (any version): the docstring and the pin slot are the
/// signature. Anything else at that path is somebody's own file.
pub(crate) fn is_our_hook_script(existing: &str) -> bool {
    let head: String = existing.chars().take(600).collect();
    head.contains("ssh-app agent hook") && existing.contains("HELPER_SHA256 = ")
}

/// The decision, without touching the host: `Refreshed` here means "write the new script".
pub(crate) fn plan_hook_script_refresh(
    existing: &str,
    new: &str,
    overwrite_foreign: bool,
) -> HookScriptRefresh {
    if existing.trim().is_empty() {
        HookScriptRefresh::NotInstalled
    } else if existing == new {
        HookScriptRefresh::UpToDate
    } else if is_our_hook_script(existing) || overwrite_foreign {
        HookScriptRefresh::Refreshed
    } else {
        HookScriptRefresh::NeedsConfirmation
    }
}
