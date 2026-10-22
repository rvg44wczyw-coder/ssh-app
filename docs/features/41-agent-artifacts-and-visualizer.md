# Feature 41: Agent Artifacts & Interactive Architecture Visualizer (Mermaid & Markdown Engine)

- **Slug**: `agent-artifacts-and-visualizer`
- **Target Release**: Milestone 30 (Visual Artifact Inspection & Rich Rendering)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (sandboxed WebView rendering, XSS prevention, Content Security Policy)
- **Status**: Built 2026-10-22, not yet seen on a device: extraction (`crates/core/src/artifacts.rs`), hardened viewer (`ArtifactViews.swift`), Mermaid 11.17.2 pinned by SHA-256 (`ios/Resources/Mermaid/`; checked in Chrome and, as a test (`testMermaidDrawsInWebKitUnderThePolicy`), in the iOS simulator's WebKit under the CSP: no `eval`; flowchart, sequence, state, ER, `xychart-beta` and a hostile label draw, a broken diagram reports its error; not yet on a real device). Markdown reports are drawn natively by block (headings, lists, quotes, code, tables as monospaced text; no web view, no HTML, no images, links never opened; at most 2000 blocks); "Photos export" is the system share sheet's own *Save Image* for the PNG, so the app asks for no photo permission (the add-only usage string of the first draft is not needed); a theme-name match is not done. Reviewed and corrected (`rev`, 2026-10-20). Source is Claude's session file only; agy is excluded until it has a structured source. Mermaid version is chosen after the checks of section 2.3.

---

## 1. Problem Statement & Scope

### 1.1 The User Problem
Autonomous AI agents frequently generate rich artifacts beyond code snippets:
- Architecture diagrams and state flowcharts in `mermaid` syntax (`flowchart TD`, `sequenceDiagram`, `stateDiagram-v2`).
- Structured analysis reports, database entity schemas (`erDiagram`), and benchmark XY charts (`xychart-beta`).
- Unified and side-by-side Git diffs spanning multiple files.

Viewing raw Mermaid code or long unformatted markdown on an iPhone terminal is unreadable. Developers need a native, fast, pinch-to-zoom interactive viewer for diagrams and artifacts.

### 1.2 In Scope
- Artifact extraction from **`transcript.jsonl` (Claude Code) only**: the screen feed has no code fences, so the screen parser cannot produce artifacts, and agy is excluded. Kinds: ````mermaid ... ```` blocks, Markdown reports, HTML snippets (rendered without JavaScript, 2.2).
- Sandboxed local renderer via hardened `WKWebView` with zero external network access:
  - Local Mermaid.js, one pinned version checked into the repo with its SHA-256, inlined into the page (2.2).
  - SVG generation with native pinch-to-zoom and pan gestures.
  - Dark mode adaptive styling matching the app's selected theme (Oled Black, Solarized Dark, Nord, Cyberpunk).
- One-tap actions:
  - `[ arrow.down.doc ]` Export as PNG / SVG to Files, or to Photos (needs `NSPhotoLibraryAddUsageDescription`, add-only).
  - `[ doc.on.doc ]` Copy Mermaid source.
  - `[ arrow.up.left.and.arrow.down.right ]` Full-screen interactive viewer.
- Diffs reuse the **existing Git hunk viewer**; this feature adds no second diff viewer.

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
    // Input is text taken from transcript.jsonl assistant messages.
    // Parses ```mermaid code fences and extracts content
    // Generates deterministic UUID based on content SHA-256
    artifacts
}
```

### 2.2 Sandboxed WKWebView

```swift
let config = WKWebViewConfiguration()
config.websiteDataStore = .nonPersistent()          // nothing is kept
let prefs = WKWebpagePreferences()
prefs.allowsContentJavaScript = true                // Mermaid page only; false for HtmlSnippet
config.defaultWebpagePreferences = prefs
// CSP in a <meta> of the generated page:
//   default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:
```

- **Mermaid is inlined into the page** (one local copy, about 2.5 MB each load; or a custom `WKURLSchemeHandler` serving the bundled file). With `loadHTMLString(baseURL: nil)` a `'self'` script source cannot load `mermaid.min.js`, so the CSP above allows inline script and nothing else, and the page has no network access.
- **Navigation is blocked** in `WKNavigationDelegate` (only the initial load is allowed; link taps open nothing). `allowFileAccessFromFileURLs` is a private key and is not used.
- **Mermaid runs with `securityLevel: 'strict'`.**
- **`HtmlSnippet` renders with `allowsContentJavaScript = false`**; scripts from agent output never run.
- **`ArtifactType` has no `files_affected` for diffs**: diffs are not artifacts of this feature.

### 2.3 Checks before the version is chosen

The pinned Mermaid build must (a) run under the CSP without `unsafe-eval`, (b) support `xychart-beta`, (c) be reproducible from the pinned hash. If a candidate needs `unsafe-eval`, it is rejected, not allowed.

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
| **XSS via Malicious Mermaid/Markdown** | CSP `default-src 'none'`, no `eval`, no network; Mermaid in `strict` mode; HTML snippets without JavaScript; non-persistent data store; navigation blocked. |
| **Local File Access / Path Traversal** | The page is loaded from memory with `baseURL: nil`; no file URL is ever loaded; no private WebKit keys are used. |
| **Memory DOS / Huge Diagram Freeze** | Mermaid diagrams exceeding 500 lines or 50 KB raw text are truncated or require user confirmation before rendering, preventing main thread freezes. |

---

## 5. Verification & Test Suite

1. **Rust Core Tests**:
   - `test_extract_mermaid_blocks`: validates detection of `flowchart`, `sequenceDiagram`, `stateDiagram-v2`.
   - `test_ignore_unclosed_code_blocks`: robust handling of partial streaming transcripts.
   - `test_html_snippet_is_marked_script_free`; `test_text_without_fences_gives_no_artifacts` (the screen feed case).
2. **Swift UI Tests**:
   - WebView rendering verification with dark mode theme injection.
   - SVG export to UIImage and clipboard copy functionality.
   - Navigation delegate refuses a link tap; the page loads with no network.
   - Pinned Mermaid hash matches the bundled file.

---

## Review (`rev`, 2026-10-20) - applied

Folded in above: transcript-only extraction and agy excluded; Mermaid inlined or served by a scheme handler; no private WebKit keys, navigation blocked, non-persistent store; HTML without JavaScript; `strict` level and version checks; Photos usage string; the existing hunk viewer is reused.
