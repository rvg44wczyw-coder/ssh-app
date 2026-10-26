//! Installation of the ssh-app approval hooks into the agents' own config files (Claude Code,
//! Antigravity, Codex, Grok): presets, the planned entries, idempotent merge and removal.
//! Launch profiles live in `agents`.

use crate::agents::{has_control, invalid, remote_path_expr, AgentKind};
use crate::error::SshCoreError;
use crate::tmux::shell_quote;
use serde_json::{json, Map, Value};

mod opencode;
mod statusline;

use opencode::*;
pub use statusline::*;

pub const HOOK_MARKER: &str = "ssh-app-hook";
pub const DEFAULT_HOOK_DIR: &str = "~/.ssh-app/bin";
const HOOK_TIMEOUT_SECS: u64 = 130;
const ANTIGRAVITY_HOOK_KEY: &str = "ssh-app";
/// Only shell commands go to the phone by default: `PreToolUse` fires for every tool call (unlike
/// Claude's `PermissionRequest`), so file tools would cause approval fatigue.
const ANTIGRAVITY_TOOL_MATCHER: &str = "run_command";
const GROK_TOOL_MATCHER: &str = "Bash";
const CRUSH_TOOL_MATCHER: &str = "^bash$";
const OPENCODE_TOOL_MATCHER: &str = "bash";

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AgentHooksStatus {
    pub config_path: String,
    pub hooks_installed: bool,
    pub script_installed: bool,
    /// The ssh-app entries found in the config, to compare with the plan (`hooks_drift`).
    pub installed: Vec<PlannedHook>,
}

// ---------------------------------------------------------------------------------------------
// Hooks
// ---------------------------------------------------------------------------------------------

mod script;
pub use script::*;
mod config;
pub use config::*;
mod overview;
pub use overview::*;
mod preset;
pub use preset::*;
mod plan;
pub use plan::*;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_extra;
