# Feature 41: Agent Artifacts & Interactive Architecture Visualizer (Mermaid & Markdown Engine)

- **Slug**: `agent-artifacts-and-visualizer`
- **Target Release**: Milestone 30 (Visual Artifact Inspection & Rich Rendering)
- **Roles**: `doc-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (sandboxed WebView rendering, XSS prevention, Content Security Policy)
- **Status**: Not ready: extraction source and WebView loading are wrong (`rev`, 2026-10-20; see the last section).

---

## 1. Problem Statement & Scope

### 1.1 The User Problem
Autonomous AI agents frequently generate rich artifacts beyond code snippets:
- Architecture diagrams and state flowcharts in `mermaid` syntax (`flowchart TD`, `sequenceDiagram`, `stateDiagram-v2`).
- Structured analysis reports, database entity schemas (`erDiagram`), and benchmark XY charts (`xychart-beta`).
- Unified and side-by-side Git diffs spanning multiple files.

Viewing raw Mermaid code or long unformatted markdown on an iPhone terminal is unreadable. Developers need a native, fast, pinch-to-zoom interactive viewer for diagrams and artifacts.

### 1.2 In Scope
- Automatic artifact extraction from Agent Feed transcripts (````mermaid ... ```` blocks, standalone Markdown reports, HTML previews).
- Sandboxed local renderer via hardened `WKWebView` with zero external network access:
  - Local bundled Mermaid.js engine (pinned version, integrity checked).
  - SVG generation with native pinch-to-zoom and pan gestures.
  - Dark mode adaptive styling matching the app's selected theme (Oled Black, Solarized Dark, Nord, Cyberpunk).
- One-tap actions:
  - `[ arrow.down.doc ]` Export as PNG / SVG to Photos or Files.
  - `[ doc.on.doc ]` Copy Mermaid source.
  - `[ arrow.up.left.and.arrow.down.right ]` Full-screen interactive viewer.
- Side-by-side or unified split diff viewer for multi-file agent patches.

### 1.3 Out of Scope
- Rendering untrusted remote iframes or external web URLs (handled separately by the Web Preview feature [05-web-preview-and-port-forwarding.md](file:///Users/ivs/rust/ssh-app/docs/features/05-web-preview-and-port-forwarding.md)).

---

## 2. Architecture & Sandboxing

### 2.1 Artifact Extraction & Domain Models (`crates/core/src/artifacts.rs`)

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactType {
    MermaidDiagram { chart_type: String },
    MarkdownReport,
    UnifiedDiff { files_affected: Vec<String> },
    HtmlSnippet,
}

#[derive(Debug, Clone)]
pub struct ExtractedArtifact {
    pub id: String,
    pub title: String,
    pub artifact_type: ArtifactType,
    pub raw_content: String,
    pub line_count: usize,
}

pub fn extract_artifacts_from_markdown(markdown: &str) -> Vec<ExtractedArtifact> {
    let mut artifacts = Vec::new();
    // Parses ```mermaid code fences and extracts content
    // Generates deterministic UUID based on content SHA-256
    artifacts
}
```

### 2.2 Sandboxed WKWebView Security Configuration
The renderer uses a strictly isolated `WKWebViewConfiguration`:
```swift
let config = WKWebViewConfiguration()
let preferences = WKWebpagePreferences()
preferences.allowsContentJavaScript = true
config.defaultWebpagePreferences = preferences

// Strict Content Security Policy injected via HTTP header or meta tag
let csp = """
default-src 'none';
script-src 'self' 'unsafe-inline';
style-src 'self' 'unsafe-inline';
img-src 'self' data:;
"""
```

- **Network Isolation**: All external network requests are blocked (`default-src 'none'`). Mermaid.js is loaded exclusively from the app bundle (`Bundle.main.url(forResource: "mermaid.min", withExtension: "js")`).

---

## 3. UI/UX Wireframes (No Emojis)

```text
┌────────────────────────────────────────────────────────┐
│ [ < Back ]       Architecture Diagram            [ ↗ ] │
│                  flowchart TD · 8 Nodes                │
├────────────────────────────────────────────────────────┤
│                                                        │
│       ┌──────────────┐         ┌──────────────┐        │
│       │  iOS Client  │ ──────► │  Rust Core   │        │
│       └──────────────┘         └──────────────┘        │
│               │                        │               │
│               ▼                        ▼               │
│       ┌──────────────┐         ┌──────────────┐        │
│       │ ActivityKit  │         │  Host Helper │        │
│       └──────────────┘         └──────────────┘        │
│                                                        │
│ [ Pinch to zoom · Pan to inspect ]                     │
├────────────────────────────────────────────────────────┤
│ [ doc.on.doc Copy Code ]          [ square.and.arrow.up Export PNG ] │
└────────────────────────────────────────────────────────┘
```

---

## 4. Security & Threat Modeling (Hacker Gate)

| Threat | Mitigation |
| :--- | :--- |
| **XSS via Malicious Mermaid/Markdown** | Content Security Policy forbids `eval`, external scripts, web sockets, and fetch. Scripting is limited strictly to local Mermaid SVG layout calculation. |
| **Local File Access / Path Traversal** | `WKPreferences` disables file system access (`allowFileAccessFromFileURLs = false`). Resources are loaded in-memory via `loadHTMLString(..., baseURL: nil)`. |
| **Memory DOS / Huge Diagram Freeze** | Mermaid diagrams exceeding 500 lines or 50 KB raw text are truncated or require user confirmation before rendering, preventing main thread freezes. |

---

## 5. Verification & Test Suite

1. **Rust Core Tests**:
   - `test_extract_mermaid_blocks`: validates detection of `flowchart`, `sequenceDiagram`, `stateDiagram-v2`.
   - `test_ignore_unclosed_code_blocks`: robust handling of partial streaming transcripts.
2. **Swift UI Tests**:
   - WebView rendering verification with dark mode theme injection.
   - SVG export to UIImage and clipboard copy functionality.

---

## Review (`rev`, 2026-10-20)

- [High] The feed parser reads the terminal screen, which has no code fences. Extraction works only from `transcript.jsonl` (Claude); say so and exclude agy.
- [High] With `loadHTMLString(baseURL: nil)`, `script-src 'self'` cannot load `mermaid.min.js`. Inline it into the page (about 2.5 MB each time) or register a custom URL scheme handler.
- [High] `allowFileAccessFromFileURLs` is a private key, not public API; do not rely on it. Block navigation in `WKNavigationDelegate` and use a non-persistent data store instead.
- [High] `HtmlSnippet` with scripts contradicts "no XSS"; render without JavaScript or drop it.
- [Medium] Mermaid must run with `securityLevel: 'strict'`; check that the pinned version works without `unsafe-eval` and supports `xychart-beta`.
- [Medium] Saving to Photos needs the add-only usage string. The split diff duplicates the existing Git hunk viewer; reuse it.
