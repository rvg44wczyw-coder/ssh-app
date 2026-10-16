# Feature 08: Smart Snippets & Fuzzy History Search (Milestone M11)

**Document Slug:** `08-smart-snippets-and-fuzzy-search`  
**Target Milestone:** Milestone M11  
**Participating Roles:** `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`  
**Hacker Gate Required:** Yes (Mandatory: touches remote shell history ingestion, secret scrubbing, and command template interpolation)  

---

## 1. Problem Statement & Scope

### 1.1 The Problem
Mobile terminal users frequently execute repetitive developer workflows (attaching to agents, inspecting git branches, checking listening ports, restarting Docker containers, running `cargo test`). Typing these commands repeatedly on an iOS virtual keyboard is slow, and searching remote shell history via `Ctrl+R` on a tiny screen is cumbersome.

### 1.2 Scope & Boundaries
- **In Scope**:
  1. **Rust Core Snippets & Fuzzy Engine (`crates/core/src/snippets.rs`)**:
     - Subsequence fuzzy matching algorithm with word-boundary, prefix, contiguous run, and usage-frequency scoring (`fuzzy_search_snippets`).
     - Built-in developer snippet catalog (`default_developer_snippets`) covering AI Agents, Git, Tmux, Rust/Cargo, Docker, and System diagnostics.
     - Template placeholder extraction (`{{variable}}`) via `extract_snippet_placeholders`.
     - Remote shell history parser & secret scrubber (`extract_history_commands`) supporting both standard and `zsh` extended history formats (`: <timestamp>:0;<cmd>`), automatically redacting lines containing API keys, tokens, or passwords.
     - Remote history fetcher on `SshSessionHandle::fetch_remote_shell_history(limit)`.
  2. **UniFFI Bridge**:
     - Export `SnippetItem`, `FuzzyMatchResult`, and helper functions to Swift.
  3. **iOS (SwiftUI) UI**:
     - `SnippetsSheetView.swift`: Instant fuzzy search bar, category filter chips (`All`, `AI Agents`, `Git`, `Cargo`, `System`, `History`), custom snippet creation sheet, placeholder input prompt, and one-tap `[ Insert ]` / `[ Run ]` buttons.
     - Quick access button in `KeyboardAccessoryView` and `MainTerminalView` toolbar.
  4. **Zero Battery Drain**:
     - Local snippets are stored on-device; remote history is fetched on-demand over an ephemeral SSH channel with zero background polling.

---

## 2. Domain Models & Protocol Contracts

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct SnippetItem {
    pub id: String,
    pub title: String,
    pub command: String,
    pub category: String,
    pub tags: Vec<String>,
    pub use_count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, uniffi::Record)]
pub struct FuzzyMatchResult {
    pub item: SnippetItem,
    pub score: i64,
    pub matched_indices: Vec<u32>,
}

#[uniffi::export]
pub fn default_developer_snippets() -> Vec<SnippetItem>;

#[uniffi::export]
pub fn fuzzy_search_snippets(
    query: String,
    items: Vec<SnippetItem>,
    limit: u32,
) -> Vec<FuzzyMatchResult>;

#[uniffi::export]
pub fn extract_history_commands(raw_history: String, limit: u32) -> Vec<String>;

#[uniffi::export]
pub fn extract_snippet_placeholders(command: String) -> Vec<String>;
```

---

## 3. Security Considerations

1. **Secret Redaction in Shell History**:
   - `extract_history_commands` filters out any command containing sensitive environment assignments or flags (`API_KEY=`, `TOKEN=`, `SECRET=`, `PASSWORD=`, `Authorization:`, `Bearer `).
2. **Safe History Retrieval**:
   - `fetch_remote_shell_history` uses a fixed, read-only command (`tail -n 200 "$HOME/.zsh_history" "$HOME/.bash_history" 2>/dev/null`) with no user-controlled path interpolation.

---

## 4. Verification & Testing Criteria

- Unit tests in `crates/core/src/snippets.rs` verifying fuzzy subsequence ranking, zsh extended history parsing, secret scrubbing, and placeholder extraction.
- XCFramework build and `swift test` verification.
- iOS `xcodebuild` verification.
