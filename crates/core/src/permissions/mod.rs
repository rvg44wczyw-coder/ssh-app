//! Permission rules of a coding agent (Claude Code: `permissions.allow|ask|deny|defaultMode` in
//! the profile's `settings.json`), edited from the phone with validation and a preview.
//!
//! Only those four fields are read or written; every other key is preserved and never exposed.

use crate::agents::{invalid, parse_object, render, AgentKind};
use crate::error::SshCoreError;
use serde_json::{json, Map, Value};

mod suggest;

pub use suggest::*;

const MAX_RULES: usize = 200;
const MAX_RULE: usize = 400;
const MAX_SPEC: usize = 300;

#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct AgentPermissions {
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
    /// `None` leaves (or removes) the agent's default mode.
    pub default_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PermissionTemplate {
    pub id: String,
    pub title: String,
    pub description: String,
    pub allow: Vec<String>,
    pub ask: Vec<String>,
    pub deny: Vec<String>,
    /// Default mode the template also sets (applied to the editor like the rules).
    pub default_mode: Option<String>,
}

mod templates;
pub use templates::*;
mod validate;
pub use validate::*;
mod inspect;
pub use inspect::*;
mod settings;
pub use settings::*;
#[cfg(test)]
mod tests;
