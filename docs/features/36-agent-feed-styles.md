# Feature 36: Agent feed presentation styles & settings switcher

- **Slug**: `agent-feed-styles`
- **Target Release**: Milestone 25 (Feed Evolution)
- **Roles**: `doc-dev`, `ios-ui-dev`, `rev`
- **Hacker Gate**: No (presentation-only UI change; remote execution contracts and Keychain remain untouched)
- **Status**: Implemented in `ios/` (2026-10-20; builds, not yet checked on a device or reviewed by `rev`). Follows `31-agent-feed.md` (Agent feed engine) and `07-structured-transcript-dual-mode.md`.

---

## 1. Problem Statement & Scope

### 1.1 The Problem
The agent feed (`StructuredTranscriptView` + `FeedTextView`) was originally built as a direct transcription of terminal text in a single hardcoded style:
- Monospace font everywhere (`UIFont.monospacedSystemFont`), making reading multi-paragraph explanations and prose exhausting.
- Rigid wireframe-like block borders (1pt stroke with low alpha) drawn by `NSLayoutManager.drawBackground`.
- Cluttered technical header displaying truncated remote paths (`~/.claude/projects/.../transcript.jsonl`) instead of actionable context (active agent, git branch, model).
- Static question prompts (`FeedPromptCard`) wedged between the scrollable feed and input bar.
- Developers have diverging needs:
  1. *CLI Purists* want dense, terminal-grade, command-centric views with shell prompts.
  2. *Mobile Readers* want clean Apple HIG typography (SF Pro), smooth bubbles, and comfortable touch targets for reading on the go.
  3. *Engineering Leads* want to monitor file diffs, git status, and pipeline step checklists.

### 1.2 Decision & Constraints
- **Strict No-Emoji Policy**: No emojis anywhere in the app UI, headers, cards, tool items, or code. All iconography relies exclusively on Apple's **SF Symbols** or clean ASCII/typographic glyphs (`●`, `○`, `❯`, `⎿`, `├─`, `└─`, `[✓]`, `[✕]`, `[+]`, `[DIFF]`).
- **User Preference Switcher**: Provide an `@AppStorage("feed_presentation_style")` option in **Settings → Appearance** allowing seamless switching between:
  1. `Pro Terminal`
  2. `Native Assistant`
  3. `Dev Studio`
- **Zero Battery Drain Integrity**: Style switching only affects the local rendering layer (`FeedAttributedBuilder`, `FeedLayoutManager`, and SwiftUI container views). Polling intervals, background socket teardown, and SSH transport remain unchanged.

---

## 2. Domain Models & Storage

### 2.1 Presentation Style Enum (`FeedPresentationStyle`)

Stored in `UserDefaults` via `@AppStorage(FeedPresentationStyle.storageKey)`:

```swift
public enum FeedPresentationStyle: String, CaseIterable, Identifiable {
    case terminal = "terminal"     // Pro Terminal: SF Mono, ❯ prompt, compact command trees
    case assistant = "assistant"   // Native Assistant: SF Pro text, iMessage-style bubbles, soft elevation
    case studio = "studio"         // Dev Studio: pipeline checklist, git indicators, inline diff highlights

    public static let storageKey = "feed_presentation_style"
    public static let fallback: FeedPresentationStyle = .assistant

    public var id: String { rawValue }

    public var title: String {
        switch self {
        case .terminal: return "Pro Terminal"
        case .assistant: return "Native Assistant"
        case .studio: return "Dev Studio"
        }
    }

    public var subtitle: String {
        switch self {
        case .terminal: return "Monospace CLI aesthetics, ❯ prompt, dense execution trees"
        case .assistant: return "Human typography (SF Pro), rounded bubbles, comfortable reading"
        case .studio: return "Code workstation, pipeline checklist, git branch and diff summaries"
        }
    }

    public var iconName: String {
        switch self {
        case .terminal: return "terminal.fill"
        case .assistant: return "bubble.left.and.bubble.right.fill"
        case .studio: return "macwindow.badge.plus"
        }
    }
}

### 2.2 Git Context Model (`GitSessionContext`)

Captured per active session tab to drive the branch and worktree indicators in the feed header:

```swift
public struct GitSessionContext: Equatable {
    public let isGit: Bool
    public let repoName: String?
    public let branch: String?
    /// Name of the worktree folder if the current tab is in a linked worktree (nil if main worktree or not git).
    public let worktreeName: String?
    /// True when the current directory is a secondary linked worktree (not the repo root).
    public let isWorktree: Bool
    public let isMain: Bool
    public let detached: Bool

    public static let none = GitSessionContext(
        isGit: false, repoName: nil, branch: nil, worktreeName: nil,
        isWorktree: false, isMain: true, detached: false
    )

    public var displayLabel: String? {
        guard isGit, let repo = repoName else { return nil }
        var parts = [repo]
        if let branch {
            parts.append(detached ? "detached:\(branch)" : branch)
        }
        if isWorktree, let wt = worktreeName {
            parts.append("wt:\(wt)")
        }
        return parts.joined(separator: " · ")
    }
}
```
```

---

## 3. Visual Specifications of the Three Styles

All styles strictly obey the **No-Emoji** mandate.

### 3.1 Style 1: «Pro Terminal Chic»

Dense, dark, monospace-first aesthetic inspired by Warp and Neovim.

#### Typography & Geometry
- **Body Font**: `SF Mono Regular` (13.5pt, line spacing 1.45).
- **Headings & Names**: `SF Mono Bold` (13pt, tracking -0.2).
- **Background**: OLED Black (`#000000`) or Deep Charcoal (`#0D1117`).
- **Borders & Dividers**: 1pt hairline `Separator.opacity(0.18)`.
- **Corner Radii**: 6pt compact corners.

#### Visual Layout (ASCII Wireframe)
```text
┌───────────────────────────────────────────────────────────┐
│ [>_ Terminal]  ● Claude · ssh-app:main (wt:ios-tests) [ ⌕ ]│
├───────────────────────────────────────────────────────────┤
│                                                           │
│ 14:22:05 ❯ cargo check --manifest-path crates/core/Cargo… │
│ ───────────────────────────────────────────────────────── │
│                                                           │
│  ○ Thought for 4.2s (analyzing AST and uniffi exports) ▾  │
│                                                           │
│  ● Claude                                                 │
│  I inspected the UniFFI records in screen.rs. All types   │
│  match the Swift bridge definition. Here is the plan:     │
│                                                           │
│  ┌─ [>_] Bash: 3 steps ────────────────────────── [✓ 0.4s] ┐│
│  │ ├─ git status -s                                       ││
│  │ ├─ && cargo clippy -p ssh-core -- -D warnings          ││
│  │ └─ && git diff --stat                                  ││
│  └────────────────────────────────────────────────────────┘│
│                                                           │
│  ┌─ [DIFF] crates/core/src/screen.rs ──────────── [+12 -3] ┐│
│  │ 42   pub struct ScreenPrompt {                         ││
│  │ 43 +     pub icon: Option<String>,                     ││
│  │ 44       pub detail: String,                           ││
│  └────────────────────────────────────────────────────────┘│
│                                                           │
│ ───────────────────────────────────────────────────────── │
│ ✶ Building target ssh_coreFFI... (8s)         [ ↓ Latest] │
├───────────────────────────────────────────────────────────┤
│ [Esc] [Tab] [^C] [^D] [│] [↑] [↓] [⏎]                     │
├───────────────────────────────────────────────────────────┤
│ [paperclip] ❯ cargo test --lib                        [⏎] │
└───────────────────────────────────────────────────────────┘
```

---

### 3.2 Style 2: «Native AI Assistant»

Human-centered, accessible iOS chat interface in full harmony with Apple HIG.

#### Typography & Geometry
- **Body Font**: `SF Pro Text` (15.5pt, line spacing 1.4, leading 22pt) for prose, explanations, and thoughts.
- **Code & Shell Font**: `SF Mono` (13pt) with syntax token coloring inside isolated cards.
- **Background**: Adaptive `UIColor.systemBackground` (soft dark gray `#1C1C1E` in dark mode).
- **User Bubble**: Asymmetric rounded capsule (18pt corner radius, top-trailing corner 4pt), `Color.appAccent`.
- **Assistant Card**: Borderless flat card or subtle `secondarySystemBackground` tint (16pt radius).

#### Visual Layout (ASCII Wireframe)
```text
┌───────────────────────────────────────────────────────────┐
│ [ chevron.left Terminal ]     Claude 3.7 Sonnet     [ ⌕ ] │
│   ● ssh-app · arrow.triangle.branch main                  │
│     square.stack.3d.down.right wt:ios-tests               │
├───────────────────────────────────────────────────────────┤
│                                                           │
│                                ┌────────────────────────┐ │
│                                │ Добавь анимацию баннера│ │
│                                │ ожидания подтверждения │ │
│                                │                  14:32 │ │
│                                └────────────────────────┘ │
│                                                           │
│ ┌─ chevron.right Thinking (6s) ─────────────────────────┐ │
│ └───────────────────────────────────────────────────────┘ │
│                                                           │
│ Claude                                                    │
│ Я обновил компонент баннера. Теперь при смене вопроса     │
│ плашка плавно скользит снизу с легким отскоком (spring).  │
│                                                           │
│ ┌─ gearshape Tool Execution ────────────────── checkmark ┐│
│ │ Bash: cargo fmt && cargo test                   0.8s   ││
│ └────────────────────────────────────────────────────────┘│
│                                                           │
│ Вот обновленный фрагмент кода:                            │
│                                                           │
│ ┌─ Swift ────────────────────────────── doc.on.doc (Copy) ┐│
│ │ withAnimation(.spring(response: 0.35)) {               ││
│ │     self.isPresented = true                            ││
│ │ }                                                      ││
│ └────────────────────────────────────────────────────────┘│
│                                                           │
├───────────────────────────────────────────────────────────┤
│ [?] Confirmation Required                             [x] │
│ Allow running: git push origin main?                      │
│ [ xmark Deny ]                        [ checkmark Allow ] │
├───────────────────────────────────────────────────────────┤
│  [Esc]   [Tab]   [^C]   [ ⌃ ]   [ ⌥ ]   [ ⌘ ]             │
├───────────────────────────────────────────────────────────┤
│ ┌───────────────────────────────────────────────────────┐ │
│ │ [paperclip] Ask Claude a question...       [arrow.up] │ │
│ └───────────────────────────────────────────────────────┘ │
```

---

### 3.3 Style 3: «Mobile Dev Studio»

IDE-inspired command center with git context, task pipeline progress, and inline diff snippets.

#### Typography & Geometry
- **Body Font**: `SF Pro Text` (14.5pt).
- **Metrics & Indicators**: `SF Mono Medium` (11.5pt, uppercase tracking 0.5).
- **Diff Font**: `SF Mono Regular` (12.5pt) with colored line gutter backgrounds (`+` green tint, `-` red tint).
- **Background**: GitHub Dark / VS Code Navy (`#0D1117`, surface `#161B22`, stroke `#30363D`).
- **Corner Radii**: 10pt structured rounded rectangles.

#### Visual Layout (ASCII Wireframe)
```text
┌───────────────────────────────────────────────────────────┐
│ [>_] ssh-app · branch:main · wt:ios-tests · ● 3 ch.   [⌕] │
├───────────────────────────────────────────────────────────┤
│ [ BRANCH: main ]  [ WT: ios-tests ]  [ UNCOMMITTED: 3 ]   │
├───────────────────────────────────────────────────────────┤
│                                                           │
│ ❯ User: Отрефактори хедер ленты и добавь статус git       │
│                                                           │
│ ┌─ PIPELINE (4 steps) ──────────────────────────────────┐ │
│ │ [✓] Read: StructuredTranscriptView.swift         0.2s │ │
│ │ [✓] Search: activeTranscriptPath in ios/         0.1s │ │
│ │ [●] Edit: StructuredTranscriptView.swift         0.4s │ │
│ │ [ ] Exec: make iphone-sim                        wait │ │
│ └───────────────────────────────────────────────────────┘ │
│                                                           │
│ ┌─ DIFF: StructuredTranscriptView.swift ────────────────┐ │
│ │ @@ -40,8 +40,11 @@                                     │ │
│ │ -   Text(path).font(.monospaced)                       │ │
│ │ +   HeaderAgentBadge(agent: viewModel.dialect)         │ │
│ │ +   GitBranchIndicator(branch: "feature-31")           │ │
│ └─────────────────────────────────────────── [VIEW FILE] ┘ │
│                                                           │
│ Claude: Хедер обновлен. Теперь в нем выводится текущая    │
│ ветка git и строгий статус агента вместо системного пути. │
│                                                           │
├───────────────────────────────────────────────────────────┤
│ ✶ Running make iphone-sim (12s)...          [ >_ Output ] │
├───────────────────────────────────────────────────────────┤
│ [ git diff ]  [ git status ]  [ cargo test ]  [ git log ] │
├───────────────────────────────────────────────────────────┤
│ [Esc] [Tab] [^C] [↑] [↓]                                  │
│ [paperclip] Command or prompt...               [arrow.up] │
└───────────────────────────────────────────────────────────┘
```

---

## 4. Component Refactoring & Implementation Plan

### 4.1 Header Bar (`FeedHeaderView`) & Git Context
Refactor the top bar in `StructuredTranscriptView.swift` to eliminate technical paths and dynamically display Git repository, branch, and worktree status:

1. **Information Extraction**:
   - Query `viewModel.sessionHandle?.fetchGitWorktrees(sessionName: viewModel.session.name)` upon entering the feed or on pane directory change (cached in `TerminalSessionViewModel.gitContext`).
   - If `status == .ok`: extract `repoRoot` (or folder basename), current `branch` (or `detached:head`), and linked worktree folder (`worktreeName`).
   - If not in git (`status != .ok`): gracefully hide git indicators and display the current working folder name.

2. **Visual Hierarchy across Styles**:
   - **`Pro Terminal`**: Compact single-line subtitle:
     `● Claude · <repo>:<branch> [wt:<worktree>]` in `SF Mono` (11pt).
   - **`Native Assistant`**: Stacked center layout:
     - Top: Agent Name (`Claude 3.7 Sonnet`) in `SF Pro Medium` (14pt).
     - Bottom: Capsule chip displaying SF Symbols `arrow.triangle.branch <branch>` and `square.stack.3d.down.right <worktree>` in `SF Pro` (11.5pt).
   - **`Dev Studio`**: Dedicated context strip directly beneath the header:
     - Interactive chips: `[ BRANCH: <branch> ]`, `[ WORKTREE: <worktree> ]`, `[ UNCOMMITTED: <count> ]`.

3. **Interactive Navigation**:
   - Tapping the branch chip opens `GitBranchesSheetView`.
   - Tapping the worktree chip opens `GitWorktreesSheetView` for instant worktree switching.
   - Tapping the `[>_ Terminal]` button switches instantly back to the raw SwiftTerm session.

### 4.2 Attributed String Engine (`FeedAttributedBuilder`)
Pass `style: FeedPresentationStyle` into `build()`:
- **Font Selection**:
  - `terminal`: `UIFont.monospacedSystemFont(ofSize: 13.5 * scale, weight: .regular)` for all segments.
  - `assistant` & `studio`: `UIFont.systemFont(ofSize: 15.5 * scale, weight: .regular)` for prose/markdown text, and `UIFont.monospacedSystemFont(ofSize: 13 * scale, weight: .regular)` for code blocks, inline code, and shell commands.
- **Paragraph Spacing & Indentation**:
  - `terminal`: tight paragraph spacing (2pt), 0pt first line indent, 8pt padding.
  - `assistant`: relaxed paragraph spacing (6pt), generous line height (1.4x), 12pt card margins.
  - `studio`: structured tabular spacing (4pt), gutter indent for diff blocks.

### 4.3 Background Renderer (`FeedLayoutManager`)
Update `drawBackground(forGlyphRange:at:)`:
- In `assistant` mode: draw user messages as rounded bubbles with corner radius 16pt, accent fill, no border. Draw assistant replies with flat padding and rounded card containers.
- In `terminal` mode: draw 1pt hairline borders with 6pt corner radius and dark fills.
- In `studio` mode: draw header tabs for code blocks and diff containers with status borders.

### 4.4 Tool Execution & Thinking Blocks
- All SF Symbols:
  - Bash: `terminal`
  - Read file: `doc.text`
  - Edit/Write file: `pencil`
  - Search: `magnifyingglass`
  - Status Success: `checkmark`
  - Status Failure: `xmark`
  - Expand/Collapse: `chevron.right` / `chevron.down`
- Strict no-emoji policy applied to all fallback titles and status labels.

### 4.5 Settings View Integration (`SettingsView.swift`)
In `SettingsView.swift` under `Page.appearance`:
```swift
Section(
    header: Text("Feed Presentation Style"),
    footer: Text("Controls how AI agent conversations (Claude Code, Antigravity) are formatted and displayed in Feed mode.")
) {
    Picker("Presentation Style", selection: $feedStyle) {
        ForEach(FeedPresentationStyle.allCases) { style in
            Label(style.title, systemImage: style.iconName)
                .tag(style)
        }
    }
    .pickerStyle(.navigationLink)
}
```

---

## 5. Security & Verification

### 5.1 Security Considerations
- **No Remote Commands**: The presentation style switcher is entirely client-side. No remote commands are dispatched.
- **Untrusted Output Sanitization**: Terminal escapes and XML tags continue to be stripped or safely mapped in `transcript.rs` and `FeedXML`. Markdown rendering remains inline-only; links are never opened (they are drawn as plain text; only the feed's own `feed://` fold toggles react to a tap).
- **Sanitized Copying**: Copy actions from code blocks strip bracketed paste markers and control sequences.

### 5.2 Verification & Test Plan
1. **Unit Tests (Swift)**:
   - `FeedPresentationStyle` raw value roundtrip and fallback resolution.
   - `FeedAttributedBuilder` font mapping verification across all three styles.
2. **Visual & Interaction Tests (Simulator & Device)**:
   - Switch between all 3 styles on the fly while connected to an active Claude/Antigravity session. Verify instant re-render without losing scroll position.
   - Verify zero emojis in all rendering paths (search codebase for emoji unicode ranges).
   - Test text selection and copy across turns under all three styles.
3. **Pinch-to-Scale & Dynamic Type**:
   - Verify pinch-to-scale gesture works reliably across all three presentation styles.


---

## 6. Implementation notes (2026-10-20)

- `FeedPresentationStyle` and `GitSessionContext` live in `Models/AppAppearance.swift`; the picker is in Settings → Appearance ("Agent feed style"), default `assistant`.
- `FeedAttributedBuilder.presentation` / `FeedLayoutManager.presentation` are set with the text (like `scale`); the look is part of the text view's inputs, so switching restyles at once and keeps the scroll position rules of any other restyle.
- Differences between the looks today: prose font (mono / SF Pro), line spacing and block gaps, frame shapes (hairline 6 pt / borderless rounded / structured 10 pt), header arrangement, and the studio branch / worktree strip. Not done: the pipeline checklist and inline diff gutters of Dev Studio (§3.3), the `UNCOMMITTED` chip, tapping the branch / worktree chips to open the git sheets (they need the sheet routing of `MainTerminalView`), the elevation of assistant cards.
- No-emoji audit: emoji removed from the keyboard accessory bar (SF Symbols via `sf:` labels), the web preview console and the host monitor; the `📋` constant of the removed stock button stays only as stored data to recognise.
- The `[!]` marker replaces the warning sign on approval requests in the feed.

### 6.x One line instead of two (2026-10-20)
The feed has one thin line at the top instead of the header and the usage strip under the tabs: git (branch, worktree, `+N` changes; tap = full names, long press = Branches / Worktrees / Changes) then the model, context and limits. Tap refreshes. Find and Refresh moved into the session menu (tiles `feedFind`, `feedRefresh`, shown only in the feed).

The switch between the terminal and the feed ("Feed" on the terminal screen, "Terminal" in the feed) is one floating button in the same place on both screens: top right by default, drag it anywhere (centre kept as fractions in `mode_switch_x` / `mode_switch_y`), long press puts it back; Settings → Appearance → "Terminal / Feed switch" sets its opacity (`mode_switch_opacity`). The per-style header arrangements (centered title, studio chips) are gone; the styles now only change the text and the frames of the feed. Not run on a device.
