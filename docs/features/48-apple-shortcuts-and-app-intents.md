# Feature 48: Apple Shortcuts, Siri Integration & App Intents Automation

- **Slug**: `apple-shortcuts-and-app-intents`
- **Target Release**: Milestone 37 (System-Wide Apple Automation & Siri Control)
- **Roles**: `doc-dev`, `rust-core-dev`, `swift-bridge-dev`, `ios-ui-dev`, `rev`, `hacker`
- **Hacker Gate**: Yes (background command execution, Siri voice authorization, App Intent parameter sanitization)
- **Status**: Reviewed and corrected (`rev` + `hacker`, 2026-10-20); first release implemented 2026-10-22 (see the end of this file). Intents choose saved items; they never run free text.

---

## 1. Problem Statement & Scope

### 1.1 The User Problem
Developers want to trigger routine remote engineering workflows without opening the app:
- *«Hey Siri, run test suite on MacBook»*
- *«Hey Siri, restart Docker database»*
- Automated triggers: When iPhone is connected to power at 23:00 $\rightarrow$ automatically run a repository health check and notify if tests fail.
- Home Screen 1-tap action widgets (e.g. `[ ▷ Deploy Staging ]`, `[ ↺ Reset Dev Server ]`).

### 1.2 In Scope
1. **Swift App Intents (`AppIntents.framework`)** over saved items only:
   - `RunAgentTaskIntent`: starts a **saved agent profile** on a **saved server**. The one free-text field is the prompt handed to the agent (not a shell command); it is delivered as the agent's input through the same path as the composer, with the composer's quoting (`tmux::shell_quote`).
   - `RunSavedCommandIntent`: runs a **saved command template** chosen by identifier (the templates are the user's snippets); nothing typed into Shortcuts is executed.
   - `RestartDockerContainerIntent` (needs feature 45): chooses a container from a list read from the host, with the confirmation of section 3.
   - `FetchHostHealthIntent`: read-only host facts for spoken feedback.
   - There is **no approve intent**. Approving happens on the approval card with the command visible; an intent may open that card (`openAppWhenRun`) and nothing more.
2. **Apple Shortcuts**: servers, profiles and templates are `AppEntity` values with an entity query, so Shortcuts offers a picker, not a text box. The result is structured JSON for chaining.
3. **Control Center and Lock Screen widgets** need a separate widget-extension target and a project-file entry; they are a later step, not part of the first intents release.

### 1.3 Out of Scope
- Unattended headless SSH operations without user pre-configured intent templates (prevents arbitrary remote execution by background apps).

---

## 2. App Intents Architecture (`ios/Sources/SSHApp/Intents/`)

### 2.1 Intent Definition Example (`RunAgentTaskIntent.swift`)

```swift
import AppIntents
import SshCoreBridge

public struct RunAgentTaskIntent: AppIntent {
    public static var title: LocalizedStringResource = "Run Remote Agent Task"
    public static var description = IntentDescription("Executes a task with an AI agent on a remote host.")

    @Parameter(title: "Server Name")
    public var serverName: String

    @Parameter(title: "Agent Profile")
    public var agentProfile: String

    @Parameter(title: "Instruction / Prompt")
    public var instruction: String

    public static var parameterSummary: some ParameterSummary {
        Summary("Run \(\.$agentProfile\) on \(\.$serverName\) with '\(\.$instruction\)'")
    }

    public func perform() async throws -> some IntentResult & ProvidesDialog {
        // 1. Resolve server and launch profile
        // 2. Connect via SshSessionHandle
        // 3. Dispatch prompt to tmux session
        // 4. Return spoken feedback
        return .result(dialog: "Agent \(agentProfile) started on \(serverName).")
    }
}
```

---

## 3. Security & Threat Modeling (Hacker Gate)

| Threat Vector | Mitigation Strategy |
| :--- | :--- |
| **Voice or Lock Screen abuse** | Each intent lists its protection explicitly. `authenticationPolicy = .requiresAuthentication` (device unlock) is set on every intent that changes anything; `.requiresLocalDeviceAuthentication` on destructive ones (Docker restart/stop). `isDestructive = true` is only metadata for the Shortcuts UI and authenticates nothing. |
| **Command injection via an intent string** | An intent cannot carry a command. Profiles, servers, templates and containers are chosen by identifier from entity queries; the free prompt is data for the agent and passes through `tmux::shell_quote`. Container names are validated as in feature 45. |
| **Approval by voice** | Removed: no intent approves a request. |
| **Background time limit** | A background intent has about 30 s. If the SSH connection cannot be made inside the budget the intent returns "not started" with the reason; it never retries in the background and never reports success it did not see. |
| **Cold start on a poor network** | Same rule: a clear "not started", no silent failure. |

---

## 4. Verification & Test Suite

1. **App Intent Unit Tests**:
   - `test_prompt_goes_through_shell_quote`: a prompt with `; rm -rf ~`, backticks and `$(id)` reaches the agent as inert text.
   - `test_unknown_entity_id_is_refused`: a profile/template/container id not in the store is refused.
   - `test_budget_exceeded_reports_not_started`.
   - `test_server_resolution_failure`: returns graceful dialog if server is offline.
2. **Shortcuts Workflow Integration**:
   - Triggering intent via `Shortcuts.app` and verifying tmux session launch.

---

## Review (`rev`, 2026-10-20) - applied

Folded in above: saved items only, no free command, no approval intent, the two authentication mechanisms named separately, the 30 s budget reported honestly, widgets deferred to an extension target, `quote_argument_for_shell` replaced by the existing `tmux::shell_quote`.

## Built 2026-10-22 (first release)

`ios/Sources/SSHApp/Intents/HostIntents.swift`: entities `ServerEntity`, `AgentProfileEntity`, `SavedCommandEntity` (read from the app's saved servers, profiles and the user's own saved commands, offered as pickers); intents `FetchHostHealthIntent` (read-only, spoken answer), `RunSavedCommandIntent` (a saved command chosen by id, refused if it has `{{placeholders}}`, output cut to 400 characters) and `RunAgentTaskIntent` (a saved **Claude Code** profile started in a new detached tmux session `intent-<8 hex>` with the instruction as its first message); one Siri phrase set for host health. The two intents that change anything carry `authenticationPolicy = .requiresAuthentication`; there is no approve intent, and no intent takes a command as text.

- Core: `build_agent_task_command(profile, session, prompt)` builds the detached start. The instruction is cleaned (control characters other than newline and tab removed, 4000 characters at most, a leading `-` covered with "Task: " so the agent's option parser cannot read it as a flag) and passed as one single-quoted argument; a test runs the generated launch line with a hostile instruction and reads the argument back byte for byte. Other agents are refused with `Unsupported` rather than started without the instruction (no other agent takes a first message on its command line).
- A server without a pinned host key (never connected in the app) or without its key available is "not started" with the reason; every call is bounded by a 25 s budget (`withIntentBudget`) and never retries or reports success it did not see.
- Tests: 5 Swift tests (budget, unknown server, instruction as data, placeholders) and 3 Rust tests. Siri and Shortcuts themselves are not tried (needs the device).
- Added 2026-10-22: `RestartDockerContainerIntent` (exact container name, system confirmation, `.requiresLocalDeviceAuthentication`; details in feature 45). The container is typed or dictated by name rather than picked from a list: a list would need a host connection at suggestion time.
- Siri phrases (2026-10-22): all four intents have a spoken phrase (Apple allows one parameter per phrase, so the server is in the phrase and Siri asks for the rest): "Run a saved command on <server>", "Start an agent on <server>", "Restart a Docker container".
- **Decision: no widgets.** A Control Center or Lock Screen widget would run in its own process, so the SSH key and the server list would have to be shared with it (keychain access group, App Group), the control API needs iOS 18 (the app targets 17), a widget that refreshes over the network breaks the rule of no radio use in the background, and none of it can be checked without a device. That is more secret-handling surface for no gain in reliability. Revisit only if the owner asks, with a tap-only control that opens the app (`openAppWhenRun`) and shares no key.
- Later: a container picker.

Security (hacker gate): the free prompt is the only free text and is data; entity ids are looked up in the store and an unknown id is refused; background budget enforced; destructive intents do not exist yet. One residual: the saved command runs as typed by the user on their own host, like in the snippets sheet; a locked phone needs the device unlock first.
