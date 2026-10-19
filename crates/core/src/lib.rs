uniffi::setup_scaffolding!();

pub mod agent_hooks;
pub mod agents;
pub mod ai;
pub mod approval;
pub mod approval_queue;
pub mod buffer;
pub mod config;
pub mod error;
pub mod file_browser;
pub mod gitbranch;
pub mod gitcommit;
pub mod gitdiff;
pub mod githistory;
pub mod githunks;
pub mod gitindex;
pub mod gitlog;
pub mod gitops;
pub mod gitstatus;
pub mod gitworktree;
pub mod hook_safety;
pub mod hook_templates;
pub(crate) mod host_api;
pub mod host_guard;
pub(crate) mod hostprobe;
pub mod hosttarget;
pub mod keys;
pub mod multiplexer;
pub mod pane_context;
pub mod permissions;
pub mod port_forward;
pub mod push;
pub mod remote_fs;
pub mod session;
pub mod snippets;
pub mod telemetry;
pub mod tmux;
pub mod transcript;
pub(crate) mod transport;
pub mod usage;
pub mod windows_install;
pub mod wsl;

pub use agents::{
    agent_config_dir_env, agent_hook_script, agent_hooks_config_path, agent_hooks_installed,
    build_agent_launch_command, build_binary_probe_command, claude_statusline_installed,
    default_hook_preset, default_launch_profiles, hook_events_for, hook_preset_uses_script,
    hooks_drift, hooks_section_of, installed_hook_entries, merge_agent_hooks,
    merge_claude_statusline, merge_planned_hooks, parse_resolved_binary, plan_agent_hooks,
    profile_config_dir, remove_agent_hooks, remove_claude_statusline, validate_launch_profile,
    AgentEnvCheck, AgentHooksStatus, AgentKind, AgentLaunchProfile, EnvVar, HookPreset,
    HookPresetKind, PlannedHook,
};
pub use ai::{
    assemble_ai_system_prompt, assemble_ai_user_prompt, build_ollama_probe_command,
    classify_command_risk, parse_ai_response, parse_ollama_generate_response, parse_ollama_probe,
    sanitize_shell_command, AiCommandRequest, AiCommandSuggestion, AiRiskAssessment, AiRiskLevel,
    OllamaHostStatus,
};
pub use approval::{
    create_canonical_signing_bytes, hash_command_sha256, verify_approval_freshness,
    verify_approval_signature, CanonicalSigningPayload,
};
pub use approval_queue::{
    approval_request_digest, sign_approval_sshsig, ApprovalList, ApprovalStatus, PendingApproval,
};
pub use config::{RemoteServerConfig, SessionConfig, SessionState, TerminalSize};
pub use error::{SshCoreError, TranscriptError};
pub use file_browser::{
    parse_remote_directory_listing, quote_remote_path_for_shell, RemoteDirectoryListing,
    RemoteFileEntry, RemoteFileKind, RemoteFilePreview,
};
pub use gitbranch::{GitBranch, GitBranchOp, GitBranchesResult};
pub use gitcommit::{GitCommitOp, GitResetMode};
pub use gitdiff::{GitDiffResult, GitDiffStatus};
pub use githistory::{GitHistoryOp, GitStash, GitStashesResult};
pub use gitindex::{GitChangeEntry, GitChangesResult, GitIndexOp};
pub use gitlog::{GitLogEntry, GitLogResult};
pub use gitops::GitOpResult;
pub use gitstatus::{GitFileChange, GitFileState, GitStatusResult};
pub use gitworktree::{GitWorktree, GitWorktreeOp, GitWorktreesResult};
pub use hook_safety::{analyze_command, lint_custom_hook_command, CommandAnalysis};
pub use hook_templates::{hook_templates, hook_templates_for, HookTemplate};
pub use host_api::{HostAgentListener, HostAgentWatch};
pub use host_guard::{
    host_guard_agent_support, host_guard_from_json, host_guard_presets, host_guard_to_json,
    host_guard_warnings, parse_agent_sessions, validate_host_guard, AgentSessionInfo, HostGuard,
    HostGuardPreset,
};
pub use hosttarget::{parse_host_target, HostTarget};
pub use keys::{derive_public_key, generate_keypair, KeypairResult};
pub use pane_context::{shell_command_for_pane, PaneContext, PaneKind};
pub use permissions::{
    merge_permissions, permission_rule_templates, permission_warnings, permissions_from_settings,
    permissions_from_settings_checked, permissions_section_of, suggest_permission_changes,
    validate_agent_permissions, AgentPermissions, CheckedPermissions, InvalidPermissionRule,
    PermissionSuggestion, PermissionTemplate, SessionPermissionsReport, SuggestionKind,
};
pub use port_forward::{PortForwardHandle, PortForwardInfo};
pub use remote_fs::{RemoteEntry, RemoteFsClient, TransferProgress, TransferResult};
pub use session::{
    execute_remote_command, fetch_host_telemetry_once, fetch_host_usage, kill_remote_tmux_session,
    list_remote_tmux_sessions, SshSessionCallback, SshSessionHandle,
};
pub use snippets::{
    default_developer_snippets, extract_history_commands, extract_snippet_placeholders,
    fuzzy_search_snippets, FuzzyMatchResult, SnippetItem,
};
pub use telemetry::{
    classify_process_category, parse_host_telemetry, HostTelemetrySnapshot, ProcessCategory,
    RemoteProcessInfo,
};
pub use tmux::TmuxSessionInfo;
pub use transcript::{
    parse_transcript_jsonl, strip_ansi_codes, TranscriptStatus, TranscriptToolCall, TranscriptTurn,
    TranscriptTurnType,
};

#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[uniffi::export]
pub fn generate_ssh_keypair() -> Result<KeypairResult, SshCoreError> {
    keys::generate_keypair()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_version() {
        assert_eq!(core_version(), "0.1.0");
    }

    #[test]
    fn test_generate_ssh_keypair_export() {
        let keys = generate_ssh_keypair().expect("Should generate keypair");
        assert!(keys.public_key_openssh.starts_with("ssh-ed25519 "));
    }
}
