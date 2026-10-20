# Feature 46: Modular Configuration Engine & Granular Subsystem Settings

- **Slug**: `modular-configuration-and-power-settings`
- **Target Release**: Milestone 35 (Deep Configurability & Power-User Controls)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (config validation, path overrides, privilege boundaries, secret sanitization)
- **Status**: Reviewed and corrected (`rev` + `hacker`, 2026-10-20); implementation pending. A module's settings are added together with its feature (section 6).

---

## 1. Problem Statement & Scope

### 1.1 The User Problem
Advanced engineers, DevOps specialists, and security-conscious developers have vastly different environments:
- One developer uses Podman on Linux (`/run/podman/podman.sock`) instead of Docker on macOS.
- Another operates on metered satellite or cellular 5G and needs to restrict image upload quality and log buffer sizes.
- A third user works in high-security corporate environments requiring strict FIDO2 biometric PIN verification, disabling cloud syncing, and setting custom TTL retention policies.
- Hardcoded timeouts, fixed buffer sizes, or unchangeable default paths alienate power users.

### 1.2 In Scope
1. **Universal Configuration Engine (`crates/core/src/config_engine.rs`)**:
   - Strongly-typed Rust configuration structs with hand-written validation (a `validate()` per module returning a typed error that names the key; no new crate).
   - Layered precedence: Default Settings $\rightarrow$ User Global Settings $\rightarrow$ Per-Host Overrides.
   - Atomic persistence (write to a temporary file, rename) with a SHA-256 **integrity check**: it detects damage only, because whoever edits the file recomputes it; it is not a signature and the UI does not call it one.
2. **Granular Control Across All Subsystems**:
   - **Docker & Container Runtime**: Custom socket paths, shell selection, log line limits, auto-refresh intervals.
   - **Multimodal Uploads**: Compression format/quality, max dimension clamp, custom upload directory, retention TTL, EXIF stripping toggle.
   - **Live Activities & Dynamic Island**: Update throttle frequency, Face ID protection for abort, phase filters, dismissal delays.
   - **Voice Dictation**: Recognition locale, syntax conversion toggles, custom project vocabulary dictionary, auto-insert vs review sheet.
   - **Mermaid & Artifacts**: Theme overrides, line count safety threshold, zoom scale limits, diff presentation style.
   - **iPadOS & Keyboard**: Hardware key command remapping, Stage Manager debounce timing, external display modes.
   - **Hardware Keys & Vault**: FIDO2 User Verification policy, granular iCloud sync toggles, memory zeroization timeouts.
   - **Multi-Agent Mesh**: Turn quotas, token burn alarms, workspace lease duration.
   - **Core SSH**: connect timeout, `$TERM` from a short allow-list, TrueColor. There is no background grace period: the socket closes on `sceneDidEnterBackground` (Zero Battery Drain) and iOS suspends the process anyway.
3. **Backup, Export & Reset**:
   - Export sanitized configuration to JSON/YAML (private keys and credentials strictly excluded).
   - 1-tap "Reset to Defaults" per module and globally.

### 1.3 Out of Scope
- Syncing private cryptographic keys in exported config files (strictly prohibited).

---

## 2. Domain Models & Subsystem Configurations

### 2.1 Rust Core Configuration Root (`crates/core/src/config_engine.rs`)

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppConfigRoot {
    pub version: u32,
    pub docker: DockerModuleConfig,
    pub uploads: UploadsModuleConfig,
    pub live_activities: LiveActivityModuleConfig,
    pub voice_dictation: VoiceDictationModuleConfig,
    pub artifacts: ArtifactsModuleConfig,
    pub keyboard: KeyboardModuleConfig,
    pub vault: VaultModuleConfig,
    pub mesh: MeshModuleConfig,
    pub ssh_core: SshCoreModuleConfig,
}

// -----------------------------------------------------------------------------
// 1. Docker Runtime Configuration
// -----------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DockerModuleConfig {
    /// Name of a `docker context` to use (letters, digits, `-`, `_`, 1..64). The helper runs the
    /// `docker` CLI (feature 45), which finds its own socket; the app never takes a socket path.
    pub docker_context: Option<String>,
    pub default_exec_shell: DockerShellMode,
    pub log_tail_lines: u32,                  // 50..5000 (default: 200)
    pub log_buffer_limit_bytes: usize,        // 256 KB..10 MB (default: 1 MB)
    pub auto_refresh_seconds: u32,            // 0 = manual, 3..60 (default: 10)
    pub max_active_container_sessions: u32,   // 1..20 (default: 5)
    pub kill_tmux_on_container_stop: bool,    // default: true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DockerShellMode {
    AutoProbe, // probe bash, fallback sh
    BashOnly,
    ZshOnly,
    ShOnly,
}

// -----------------------------------------------------------------------------
// 2. Multimodal Uploads Configuration
// -----------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UploadsModuleConfig {
    /// Fixed root `~/.ssh-app/run/uploads`; only a sub-folder name (letters, digits, `-`, `_`) may be set.
    pub upload_subdir: Option<String>,
    pub image_max_dimension: u32,             // 1024, 2048, 4096, 0 = unlimited
    pub image_compression: ImageCompressionPreference,
    pub strip_exif_metadata: bool,            // default: true
    pub retention_ttl_hours: u32,             // 1..168, 0 = infinite (default: 24)
    /// Fixed placeholders only (`{{remote_path}}`); any other `{{...}}`, backtick, `$(` or control
    /// character is rejected. Default "@{{remote_path}}".
    pub prompt_injection_template: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ImageCompressionPreference {
    AutoOptimal,
    LosslessPng,
    JpegHighQuality,   // 85%
    JpegBalanced,      // 70%
    Webp,
}

// -----------------------------------------------------------------------------
// 3. Live Activities & Dynamic Island Configuration
// -----------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LiveActivityModuleConfig {
    pub min_push_interval_seconds: u32,       // 4..60 (default: 8); updates are also sent only on a state change (feature 39)
    pub require_face_id_for_abort: bool,      // default: true
    pub auto_dismissal_minutes: u32,          // 0 = manual, 5, 15, 60 (default: 15)
    pub notify_on_thinking: bool,             // default: false
    pub notify_on_tools: bool,                // default: true
    pub notify_on_approvals: bool,            // default: true
    // no "tests" phase: progress of a test run cannot be derived from hooks (feature 39)
}

// -----------------------------------------------------------------------------
// 4. Voice Dictation Configuration
// -----------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VoiceDictationModuleConfig {
    pub locale_identifier: String,            // e.g. "en-US", "ru-RU", "system"
    pub normalize_code_syntax: bool,          // default: true
    pub custom_vocabulary: Vec<String>,       // e.g. ["gRPC", "K8s", "PostgreSQL", "Ollama"]
    pub review_before_send: bool,             // default: true (composer review)
}

// -----------------------------------------------------------------------------
// 5. Visual Artifacts & Mermaid Configuration
// -----------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArtifactsModuleConfig {
    pub mermaid_theme: MermaidThemePreference,
    pub max_line_threshold: usize,            // 100..5000 (default: 500)
    pub enable_pinch_to_zoom: bool,           // default: true
    pub split_diff_layout: SplitDiffLayoutMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum MermaidThemePreference {
    AutoAdaptive,
    Dark,
    Neutral,
    Forest,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SplitDiffLayoutMode {
    SideBySide,
    Unified,
    AutoByViewport,
}

// -----------------------------------------------------------------------------
// 6. iPadOS & Hardware Keyboard Configuration
// -----------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KeyboardModuleConfig {
    pub keymap_preset: KeymapPreset,
    pub stage_manager_debounce_ms: u64,       // 50..500 ms (default: 100 ms)
    pub external_display_mode: ExternalDisplayMode,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum KeymapPreset {
    StandardMac,
    EmacsFriendly,
    VimFriendly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ExternalDisplayMode {
    MirrorScreen,
    DedicatedFullscreenTerminal,
}

// -----------------------------------------------------------------------------
// 7. Hardware Keys & Vault Sync Configuration
// -----------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VaultModuleConfig {
    pub fido2_user_verification_required: bool, // default: false
    pub sync_servers_to_icloud: bool,           // default: true (names and addresses)
    pub sync_host_fingerprints_to_icloud: bool, // default: FALSE: syncing pins transfers trust (feature 43)
    pub sync_snippets_to_icloud: bool,          // default: true
    pub sync_agent_profiles_to_icloud: bool,    // default: true
    pub zeroize_memory_delay_seconds: u32,      // 0 = immediate, 60, 300
}

// -----------------------------------------------------------------------------
// 8. Multi-Agent Mesh Configuration
// -----------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeshModuleConfig {
    pub max_turns_per_round: u32,               // 1..50 (default: 5)
    pub workspace_lease_timeout_seconds: u32,   // 30..600 (default: 120)
    pub token_burn_warning_limit: u32,          // default: 50000 tokens/hr
    pub enforce_readonly_reviewers: bool,       // default: true
}

// -----------------------------------------------------------------------------
// 9. Zero Battery Drain & Core SSH Configuration
// -----------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SshCoreModuleConfig {
    pub connect_timeout_seconds: u32,             // 3..60 (default: 10)
    pub terminal_type: TerminalType,              // allow-list: Xterm256Color (default), Xterm, Screen256Color, TmuxCompatible
    pub enable_truecolor: bool,                   // default: true
    pub tcp_keepalive_interval_seconds: u32,      // 5..120 (default: 15), foreground only
}

// -----------------------------------------------------------------------------
// 10. Existing Subsystems Optimization & Tuning (Features 05-13, 28, 31)
// -----------------------------------------------------------------------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubsystemOptimizationsConfig {
    // buffer.rs / SwiftTerm rendering & battery profile
    pub rendering_profile: RenderingBatteryProfile,
    pub max_scrollback_lines: usize,              // 1000..50000 (default: 10000)

    // file_browser.rs / remote_fs.rs SFTP tuning
    pub max_file_preview_bytes: usize,            // 128 KB..2 MB (default: 140000)
    pub sftp_chunk_size_bytes: usize,             // 32 KB..512 KB (default: 64 KB)
    pub persist_dotfiles_visibility: bool,        // default: true

    // ai.rs Local Ollama & Assistant tuning
    pub custom_ollama_endpoint: Option<String>,   // https, or http to loopback only; no credentials in the URL; default None ("http://127.0.0.1:11434")
    pub default_assistant_model: Option<String>,  // e.g. "qwen2.5-coder:7b"
    pub assistant_temperature: f32,               // 0.0..1.0 (default: 0.2)

    // approval_queue.rs / agent_hooks.rs
    pub approval_wait_secs: u64,                  // 5..120: the EXISTING setting; the hook timeout in agent configs is 130 s, so nothing longer is possible
    pub approval_alert_haptic_style: AlertStyle,
    pub auto_dismiss_approved_notices_sec: u32,   // 0 = manual, 3, 10

    // port_forward.rs Quick ports
    pub custom_quick_ports: Vec<u16>,             // each 1024..65535, at most 12 (default: [3000, 5173, 8000, 8080])
    pub auto_open_devtools_on_error: bool,        // default: false

    // git.rs Project View
    pub diff_context_lines: u32,                  // 3, 5, 10 (default: 3)
    pub auto_fetch_interval_minutes: u32,         // 0 = manual, 5, 15

    // transcript.rs Agent Feed
    pub transcript_render_depth_turns: usize,     // 50, 100, 200 (default: 100)
    pub auto_collapse_thinking_blocks: bool,      // default: true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RenderingBatteryProfile {
    BatterySaver,  // 33-50ms flush (20-30 FPS), 64KB batch
    Balanced60Fps, // 16ms flush (60 FPS), 32KB batch (default)
    ProMotion120Fps, // 8ms flush (120 FPS), 16KB batch
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum AlertStyle {
    GentleHaptic,
    SoundAndHaptic,
    ProminentAlert,
}
```

---

## 3. UI Navigation & Settings Architecture

Settings are grouped in a dedicated, multi-tiered hierarchy in `ios/Sources/SSHApp/Views/SettingsView.swift`:

```text
┌────────────────────────────────────────────────────────┐
│ Settings                                       [ Done ]│
├────────────────────────────────────────────────────────┤
│ GENERAL & HARDWARE                                     │
│   [server.rack] Servers & Connection Rules             │
│   [key.fill] SSH & FIDO2 Security Keys                 │
│   [icloud.fill] iCloud Vault Sync Options              │
│   [keyboard] Hardware Keyboard & Shortcuts             │
├────────────────────────────────────────────────────────┤
│ AGENT & DEVELOPMENT MODULES                            │
│   [shippingbox.fill] Docker & Container Runtime        │
│   [photo.fill] Multimodal Uploads & Compression        │
│   [bell.badge.fill] Live Activities & Push Relay       │
│   [mic.fill] Voice Input & Dictation Normalizer        │
│   [chart.xyaxis.line] Artifacts & Mermaid Diagrams     │
│   [person.3.fill] Multi-Agent Mesh Policies            │
├────────────────────────────────────────────────────────┤
│ CORE ENGINE & APPEARANCE                               │
│   [terminal.fill] Terminal Buffer & Zero Battery Drain │
│   [paintpalette.fill] Themes & Nerd Fonts              │
├────────────────────────────────────────────────────────┤
│ BACKUP & RESET                                         │
│   [square.and.arrow.up] Export Configuration (JSON)    │
│   [arrow.clockwise] Reset All Settings to Default      │
└────────────────────────────────────────────────────────┘
```

---

## 4. Security Considerations (Hacker Gate)

An imported file is hostile input, as if a stranger had sent it.

| Threat | Mitigation |
| :--- | :--- |
| **Malicious config import** | Fixed allow-list of keys (an unknown key rejects the file); every value is checked by type and range; an out-of-range value rejects the whole file and names the key (no silent clamping). Endpoints: `https`, or `http` to `127.0.0.1`/`::1` only, no credentials in the URL. Prompt templates: fixed placeholders only. |
| **Path override** | There is no free path setting: the upload root is fixed and only a sub-folder name may be set; Docker takes a context name, not a socket path. Where a path is unavoidable it must be relative or under the user's home, with no `..`, no absolute path outside the home, and symlinks resolved and re-checked (rejecting `..` alone does not catch `/etc/shadow`). |
| **Private key leaks in export** | The schema contains no key material; export excludes anything that looks like a token or key, and a test greps the output. |
| **Integrity file is not a signature** | The SHA-256 detects damage only; import never trusts it as authenticity. |
| **DoS via extreme values** | Every numeric field has a range checked in `validate()`; list lengths and string lengths are capped. |
| **Unsafe defaults by import** | Security-relevant switches (`sync_host_fingerprints_to_icloud`, `enforce_readonly_reviewers`, `require_face_id_for_abort`) cannot be loosened by an import; an import that tries is applied with those keys at their current value and the user is told. |

---

## 5. Verification & Test Suite

1. **Rust Core Tests**:
   - `test_default_config_validity`: ensures default config parses and passes all validation constraints.
   - `test_out_of_range_is_rejected_with_the_key_named`: `log_tail_lines: 9999999` rejects the file and names `docker.log_tail_lines`.
   - `test_import_rejects_hostile_values`: `docker_context` with `;`, `upload_subdir` `../x`, an `http://evil` Ollama endpoint, a template with `$(id)`, an unknown key.
   - `test_import_cannot_loosen_security_switches`.
   - `test_approval_wait_matches_the_existing_setting`: range 5..120.
   - `test_config_serialization_roundtrip`: ensures full JSON serialization and deserialization integrity.
   - `test_export_excludes_secrets`: verifies serialized JSON contains no key or token fields.
2. **Swift UI Tests**:
   - Navigation through each module settings page.
   - Persistence in `UserDefaults` / `AppStorage` with immediate effect on active sessions.
   - 1-tap "Reset to Defaults" restoring initial configuration states.

---

## 6. Persistence, migration and order of work

- Today's settings live in `UserDefaults`/`@AppStorage`. The first module that lands reads the old keys once, writes the new document, and keeps the old keys for one release so a downgrade still works.
- The Settings screen shows only the modules whose feature exists; the rows in section 3 appear with their feature, and nothing is built ahead of it. Of the ten blocks above, only the existing-subsystem tuning (`SubsystemOptimizationsConfig`) and the Core SSH block have features today.
- Reset to Defaults is per module and global and asks for confirmation.

## Review (`rev`, 2026-10-20) - applied

Folded into the text above: hostile-import model with allow-lists and no free paths; the approval wait is the existing 5..120 setting; checksum described as damage detection; background grace removed; hand-written validation instead of the `validator` crate; `UserDefaults` migration; settings per feature; Docker context instead of a socket path; fingerprints sync default off (feature 43); no "tests" notification phase (feature 39).
