import XCTest
import WebKit
import ImageIO
import UniformTypeIdentifiers
import UIKit
import SshCoreBridge
@testable import SSHApp

final class AgentFeedTests: XCTestCase {
    func testSingleLineIsTextAndEnter() {
        XCTAssertEqual(FeedSubmit.bytes(for: "  hello \n"), Array("hello".utf8) + [0x0d])
    }

    func testMultiLineIsOneBracketedPaste() {
        let b = FeedSubmit.bytes(for: "a\nb")
        XCTAssertEqual(b, Array("\u{1b}[200~a\nb\u{1b}[201~".utf8) + [0x0d])
    }

    func testPasteMarkersInsideTheTextAreRemoved() {
        let b = FeedSubmit.bytes(for: "x\u{1b}[201~\nrm -rf ~")
        let text = String(decoding: b, as: UTF8.self)
        XCTAssertEqual(text.components(separatedBy: "\u{1b}[201~").count, 2)
    }

    func testPresentationStyleRoundTripAndFallback() {
        for style in FeedPresentationStyle.allCases {
            XCTAssertEqual(FeedPresentationStyle.resolved(style.rawValue), style)
        }
        XCTAssertEqual(FeedPresentationStyle.resolved("nonsense"), FeedPresentationStyle.fallback)
        XCTAssertEqual(FeedPresentationStyle.resolved(""), .assistant)
    }

    func testGitContextLabel() {
        let ctx = GitSessionContext(isGit: true, repoName: "ssh-app", branch: "main", worktreeName: "ios-tests",
                                    isWorktree: true, detached: false)
        XCTAssertEqual(ctx.displayLabel, "ssh-app · main · wt ios-tests")
        XCTAssertNil(GitSessionContext.none.displayLabel)
    }

    func testLongGitNamesAreShortenedInTheMiddle() {
        XCTAssertEqual(GitSessionContext.short("main", limit: 28), "main")
        let long = "feature/very-long-branch-name-for-ticket-ABC-12345"
        let short = GitSessionContext.short(long, limit: GitSessionContext.branchLimit)
        XCTAssertEqual(short.count, GitSessionContext.branchLimit)
        XCTAssertTrue(short.hasPrefix("feature/"))
        XCTAssertTrue(short.hasSuffix("12345"))
        XCTAssertTrue(short.contains("…"))
        let ctx = GitSessionContext(isGit: true, repoName: "r", branch: long, worktreeName: nil, isWorktree: false, detached: false)
        XCTAssertEqual(ctx.label(full: true), "r · " + long)
        XCTAssertEqual(ctx.label(full: false), "r · " + short)
        // emoji / multi-scalar names are cut on characters, not bytes
        XCTAssertEqual(GitSessionContext.short(String(repeating: "é", count: 40), limit: 10).count, 10)
    }

    func testDraftAppendTypesOnlyTheNewEnd() {
        XCTAssertEqual(DraftSync.bytes(from: "hel", to: "hello"), Array("lo".utf8))
        XCTAssertEqual(DraftSync.bytes(from: "hello", to: "hello"), [])
    }

    func testDraftDeletionIsBackspaces() {
        XCTAssertEqual(DraftSync.bytes(from: "hello", to: "he"), [0x7f, 0x7f, 0x7f])
        XCTAssertEqual(DraftSync.bytes(from: "hello", to: ""), Array(repeating: 0x7f, count: 5))
    }

    func testDraftEditInTheMiddleRetypesFromThere() {
        XCTAssertEqual(DraftSync.bytes(from: "hello", to: "heXlo"), [0x7f, 0x7f, 0x7f] + Array("Xlo".utf8))
    }

    func testDraftWithNewlineIsAPasteAndControlCharactersAreDropped() {
        let b = DraftSync.bytes(from: "a", to: "a\nb\u{3}\u{1b}[A")
        XCTAssertEqual(String(decoding: b ?? [], as: UTF8.self), "\u{1b}[200~\nb[A\u{1b}[201~")
    }

    func testUncertainDraftIsClearedGenerouslyAndRetyped() {
        let b = DraftSync.bytes(from: "ab", to: "abc", uncertain: true) ?? []
        XCTAssertEqual(b.prefix(24).filter { $0 == 0x7f }.count, 24)
        XCTAssertEqual(Array(b.suffix(3)), Array("abc".utf8))
    }

    func testDraftTooLongToDeleteIsNotMirrored() {
        XCTAssertNil(DraftSync.bytes(from: String(repeating: "x", count: 5000), to: ""))
    }

    func testMarkdownSplitsFencedCode() {
        let segs = FeedMarkdown.segments("intro\n```swift\nlet a = 1\n```\nafter")
        XCTAssertEqual(segs, [.prose("intro"), .code(language: "swift", code: "let a = 1"), .prose("after")])
    }

    func testUnclosedFenceRunsToTheEnd() {
        let segs = FeedMarkdown.segments("```py\nprint(1)")
        XCTAssertEqual(segs, [.code(language: "py", code: "print(1)")])
    }
}

final class TerminalPasteTests: XCTestCase {
    func testBracketedPasteKeepsLinesTogether() {
        let b = TerminalPaste.bytes("a\r\nb", bracketed: true)
        XCTAssertEqual(String(decoding: b, as: UTF8.self), "\u{1b}[200~a\nb\u{1b}[201~")
    }

    func testPlainPasteTurnsLineBreaksIntoEnter() {
        XCTAssertEqual(TerminalPaste.bytes("a\nb", bracketed: false), Array("a\rb".utf8))
    }

    func testEndMarkerInsideThePasteIsRemoved() {
        let text = String(decoding: TerminalPaste.bytes("x\u{1b}[201~y", bracketed: true), as: UTF8.self)
        XCTAssertEqual(text, "\u{1b}[200~xy\u{1b}[201~")
    }

    // MARK: ImagePreflight (feature 38)

    private func makeImage(width: Int, height: Int, type: UTType, gps: Bool) -> Data {
        let size = CGSize(width: width, height: height)
        let fmt = UIGraphicsImageRendererFormat.default()
        fmt.scale = 1
        let img = UIGraphicsImageRenderer(size: size, format: fmt).image { c in
            UIColor.red.setFill(); c.fill(CGRect(origin: .zero, size: size))
        }.cgImage!
        let out = NSMutableData()
        let dest = CGImageDestinationCreateWithData(out, type.identifier as CFString, 1, nil)!
        var props: [CFString: Any] = [:]
        if gps {
            props[kCGImagePropertyGPSDictionary] = [kCGImagePropertyGPSLatitude: 55.75, kCGImagePropertyGPSLongitude: 37.61]
            props[kCGImagePropertyTIFFDictionary] = [kCGImagePropertyTIFFModel: "iPhone-Secret-Model"]
        }
        CGImageDestinationAddImage(dest, img, props as CFDictionary)
        CGImageDestinationFinalize(dest)
        return out as Data
    }

    func testPreflightDropsGpsAndDeviceMetadata() throws {
        let input = makeImage(width: 64, height: 64, type: .jpeg, gps: true)
        let before = CGImageSourceCopyPropertiesAtIndex(CGImageSourceCreateWithData(input as CFData, nil)!, 0, nil) as? [CFString: Any]
        XCTAssertNotNil(before?[kCGImagePropertyGPSDictionary], "the fixture must carry GPS")
        let r = try ImagePreflight.process(input)
        XCTAssertEqual(r.ext, "jpg")
        let after = CGImageSourceCopyPropertiesAtIndex(CGImageSourceCreateWithData(r.data as CFData, nil)!, 0, nil) as? [CFString: Any]
        XCTAssertNil(after?[kCGImagePropertyGPSDictionary])
        XCTAssertNil((after?[kCGImagePropertyTIFFDictionary] as? [CFString: Any])?[kCGImagePropertyTIFFModel])
        XCTAssertFalse(String(decoding: r.data, as: UTF8.self).contains("iPhone-Secret-Model"))
    }

    func testPreflightClampsTheLongestSideAndKeepsPng() throws {
        let big = makeImage(width: 3000, height: 1500, type: .png, gps: false)
        let r = try ImagePreflight.process(big)
        XCTAssertEqual(r.ext, "png")
        XCTAssertEqual(max(r.width, r.height), ImagePreflight.maxSide)
        XCTAssertEqual(r.width, 2048)
        XCTAssertEqual(r.height, 1024)
    }

    func testPreflightLeavesSmallImagesAtTheirSize() throws {
        let r = try ImagePreflight.process(makeImage(width: 100, height: 40, type: .jpeg, gps: false))
        XCTAssertEqual(r.width, 100)
        XCTAssertEqual(r.height, 40)
    }

    func testPreflightRejectsGarbage() {
        XCTAssertThrowsError(try ImagePreflight.process(Data("not an image".utf8)))
        XCTAssertThrowsError(try ImagePreflight.process(Data()))
    }

    func testMultiplexerPreferenceIsKeptPerHostAndForgetsTheChosenProgram() {
        let key = "test@zellij-host-\(UUID().uuidString):22"
        XCTAssertEqual(HostTargetCache.multiplexerPreference(forKey: key), .auto)
        HostTargetCache.setChosenMultiplexer(.tmux, forKey: key)
        HostTargetCache.setMultiplexerPreference(.preferZellij, forKey: key)
        XCTAssertEqual(HostTargetCache.multiplexerPreference(forKey: key), .preferZellij)
        XCTAssertNil(HostTargetCache.chosenMultiplexer(forKey: key), "a changed preference asks the host again")
        HostTargetCache.setMultiplexerPreference(.auto, forKey: key)
        XCTAssertEqual(HostTargetCache.multiplexerPreference(forKey: key), .auto)
    }

    // MARK: App Intents (feature 48)

    func testIntentBudgetReportsNotStartedInsteadOfWaiting() async {
        do {
            _ = try await withIntentBudget(seconds: 0.2) { () -> Int in
                try await Task.sleep(for: .seconds(5))
                return 1
            }
            XCTFail("the slow work must not be waited for")
        } catch let e as IntentFailure {
            guard case .notStarted = e else { return XCTFail("wrong failure \(e)") }
        } catch {
            XCTFail("unexpected \(error)")
        }
    }

    func testIntentBudgetReturnsTheResultOfFastWork() async throws {
        let v = try await withIntentBudget(seconds: 5) { 42 }
        XCTAssertEqual(v, 42)
    }

    @MainActor
    func testAnUnknownServerIsRefused() {
        XCTAssertThrowsError(try IntentTarget.resolve(serverId: "00000000-0000-0000-0000-000000000000")) { error in
            guard case IntentFailure.unknownItem = error else { return XCTFail("wrong failure \(error)") }
        }
    }

    func testAnInstructionIsData_NotACommand() throws {
        let profile = SavedAgentProfile(defaultLaunchProfiles()[0])
        let c = try buildAgentTaskCommand(profile: profile.coreProfile, sessionName: "intent-ab12cd34",
                                          prompt: "x'; rm -rf ~ #`id` $(id)")
        XCTAssertTrue(c.contains("tmux new-session -d -s intent-ab12cd34"))
        XCTAssertThrowsError(try buildAgentTaskCommand(profile: profile.coreProfile, sessionName: "bad name", prompt: "x"))
        XCTAssertThrowsError(try buildAgentTaskCommand(profile: profile.coreProfile, sessionName: "intent-ab12cd34", prompt: "  "))
    }

    func testPlaceholdersInASavedCommandAreFound() {
        XCTAssertFalse(extractSnippetPlaceholders(command: "ls {{dir}}").isEmpty)
        XCTAssertTrue(extractSnippetPlaceholders(command: "ls -la").isEmpty)
    }

    // MARK: Docker (feature 45)

    func testDockerTabsHaveTheirOwnTypeAndSurviveSaving() throws {
        let command = try buildDockerExecCommand(containerId: "postgres-db")
        let name = try dockerSessionName(containerName: "a.b", containerId: "abcdef123456")
        let tab = TabSession(title: "[docker] a.b", agentType: .dockerContainer, sessionName: name, command: command)
        let back = try JSONDecoder().decode(TabSession.self, from: JSONEncoder().encode(tab))
        XCTAssertEqual(back.agentType, .dockerContainer)
        XCTAssertEqual(back.agentType.iconName, "shippingbox.fill")
        XCTAssertNil(back.agentType.defaultCommand)
        XCTAssertEqual(back.sessionName, "docker-a-b-abcdef")
        XCTAssertThrowsError(try buildDockerExecCommand(containerId: "a; reboot"))
    }

    // MARK: Module settings (feature 46)

    @MainActor
    func testSettingsAreStoredAndAFailedImportChangesNothing() throws {
        let key = AppConfigStore.storageKey
        let before = UserDefaults.standard.string(forKey: key)
        defer { UserDefaults.standard.set(before, forKey: key) }
        let store = AppConfigStore.shared
        store.replace(defaultAppConfig())
        store.update { $0.docker.logTailLines = 400 }
        XCTAssertEqual(AppConfigStore.current().docker.logTailLines, 400)
        XCTAssertThrowsError(try importAppConfig(current: store.config, fileJson: "{\"config\":{\"version\":1,\"docker\":{\"log_tail_lines\":9}}}"))
        XCTAssertEqual(store.config.docker.logTailLines, 400)
        let file = try exportAppConfig(config: store.config)
        let back = try importAppConfig(current: defaultAppConfig(), fileJson: file)
        XCTAssertEqual(back.config.docker.logTailLines, 400)
        store.replace(defaultAppConfig())
        XCTAssertEqual(AppConfigStore.current(), defaultAppConfig())
    }

    @MainActor
    func testPictureSettingsReachThePreflight() throws {
        let key = AppConfigStore.storageKey
        let before = UserDefaults.standard.string(forKey: key)
        defer { UserDefaults.standard.set(before, forKey: key) }
        AppConfigStore.shared.replace(defaultAppConfig())
        AppConfigStore.shared.update { $0.uploads.maxDimension = 256 }
        let r = try ImagePreflight.process(makeImage(width: 1000, height: 500, type: .png, gps: false))
        XCTAssertEqual(max(r.width, r.height), 256)
        AppConfigStore.shared.replace(defaultAppConfig())
    }

    // MARK: Diagrams (feature 41)

    func testTheDiagramPageCarriesTheSourceOnlyAsBase64AndAllowsNoNetwork() {
        let hostile = "flowchart TD\n A[\"</script><script>alert(1)</script>\"]"
        let page = ArtifactPage.mermaid(source: hostile, mermaidJS: "/* lib */", dark: true)
        XCTAssertFalse(page.contains("alert(1)"))
        XCTAssertTrue(page.contains(Data(hostile.utf8).base64EncodedString()))
        XCTAssertTrue(page.contains("default-src 'none'"))
        XCTAssertFalse(page.contains("connect-src"))
        XCTAssertFalse(page.contains("unsafe-eval"))
        XCTAssertTrue(page.contains("securityLevel: 'strict'"))
    }

    func testAnHtmlSnippetPageHasNoScriptSource() {
        let page = ArtifactPage.html(snippet: "<b>x</b><script>alert(1)</script>", dark: false)
        XCTAssertFalse(ArtifactPage.htmlCSP.contains("script-src"))
        XCTAssertTrue(page.contains("default-src 'none'"))
    }

    func testThePinnedMermaidFileMatchesItsHash() throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Resources/Mermaid/mermaid.min.js")
        let data = try Data(contentsOf: url)
        XCTAssertEqual(ArtifactPage.sha256(data), ArtifactPage.pinnedMermaidSHA256)
    }

    func testArtifactsAreCollectedOncePerBlockFromAssistantTurnsOnly() {
        let block = "```mermaid\nflowchart TD\n A-->B\n```"
        func turn(_ type: TranscriptTurnType, _ text: String, _ id: String) -> TranscriptTurn {
            TranscriptTurn(id: id, stepIndex: 0, turnType: type, timestamp: nil, content: text, thinking: nil,
                           toolCalls: [], status: .done, label: nil)
        }
        let turns = [turn(.assistant, block, "1"), turn(.assistant, block, "2"), turn(.user, block, "3")]
        XCTAssertEqual(StructuredTranscriptView.collectArtifacts(turns).count, 1)
    }

    /// Real WebKit (the simulator's), the page and settings of the viewer: every diagram family draws
    /// under the policy, and a hostile label does not run.
    @MainActor
    func testMermaidDrawsInWebKitUnderThePolicy() async throws {
        let url = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .appendingPathComponent("Resources/Mermaid/mermaid.min.js")
        let js = try String(contentsOf: url, encoding: .utf8)
        let sources = [
            "flowchart TD\n A-->B",
            "sequenceDiagram\n A->>B: hi",
            "stateDiagram-v2\n [*]-->S",
            "erDiagram\n A ||--o{ B : has",
            "xychart-beta\n x-axis [a,b]\n bar [1,2]",
            "flowchart TD\n A[\"<img src=x onerror=document.title='pwned'>\"]",
        ]
        for (n, source) in sources.enumerated() {
            let probe = WebProbe()
            let result = await probe.run(ArtifactPage.mermaid(source: source, mermaidJS: js, dark: n % 2 == 0))
            XCTAssertEqual(result.ok, true, "source \(n): \(result.text)")
            XCTAssertNotEqual(result.title, "pwned")
        }
    }

    @MainActor
    func testTheSystemPasteTargetTakesTextAndNothingElse() async {
        let target = SystemPasteButton.PasteTarget()
        var got: [String] = []
        target.onText = { got.append($0) }
        let text = NSItemProvider(object: "echo hi\nsecond" as NSString)
        XCTAssertTrue(target.canPaste([text]))
        target.paste(itemProviders: [text])
        try? await Task.sleep(nanoseconds: 300_000_000)
        XCTAssertEqual(got, ["echo hi\nsecond"])
        XCTAssertFalse(target.canPaste([NSItemProvider(object: UIImage())]))
    }

    func testAReportIsSplitIntoBlocks() {
        let md = "# Title\n\nSome *text*\ncontinues.\n\n- one\n  - nested\n1. first\n\n> quoted\n\n```swift\nlet x = 1\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n---\n"
        let blocks = ReportBlock.parse(md)
        XCTAssertEqual(blocks, [
            .heading(level: 1, text: "Title"),
            .paragraph("Some *text* continues."),
            .bullet(text: "one", depth: 0),
            .bullet(text: "nested", depth: 1),
            .numbered(text: "first", number: "1", depth: 0),
            .quote("quoted"),
            .code("let x = 1"),
            .table(["| a | b |", "|---|---|", "| 1 | 2 |"]),
            .rule,
        ])
    }

    func testAnUnclosedFenceAndHugeReportsAreBounded() {
        XCTAssertEqual(ReportBlock.parse("```\nlet y = 2"), [.code("let y = 2")])
        let huge = (0..<5000).map { "- item \($0)" }.joined(separator: "\n")
        XCTAssertEqual(ReportBlock.parse(huge).count, ReportBlock.maxBlocks)
        XCTAssertEqual(ReportBlock.parse(""), [])
        // raw HTML and links are only text: nothing here can become markup
        XCTAssertEqual(ReportBlock.parse("<script>alert(1)</script>"), [.paragraph("<script>alert(1)</script>")])
    }

    func testADockerIntentNamesOneContainerExactly() {
        func c(_ id: String, _ name: String) -> DockerContainerInfo {
            DockerContainerInfo(id: id, name: name, image: "img", state: .running, statusText: "Up", createdAt: "", ports: "", unhealthy: false, project: nil, service: nil, scope: "", restartable: true)
        }
        let list = [c("a1", "db"), c("b2", "db-replica"), c("c3", "Web")]
        XCTAssertEqual(DockerIntentMatch.container(named: "db", in: list)?.id, "a1")
        XCTAssertEqual(DockerIntentMatch.container(named: "  db \n", in: list)?.id, "a1")
        XCTAssertNil(DockerIntentMatch.container(named: "d", in: list))          // a prefix is not a name
        XCTAssertNil(DockerIntentMatch.container(named: "web", in: list))        // case matters
        XCTAssertNil(DockerIntentMatch.container(named: "", in: list))
        XCTAssertNil(DockerIntentMatch.container(named: "db", in: list + [c("d4", "db")]))   // twice: refuse
    }
}


/// Loads a page in a web view set up like the viewer's and reports what the page posted.
@MainActor
private final class WebProbe: NSObject, WKScriptMessageHandler {
    private var cont: CheckedContinuation<(ok: Bool, text: String, title: String), Never>?
    private var web: WKWebView?

    func run(_ html: String) async -> (ok: Bool, text: String, title: String) {
        await withCheckedContinuation { c in
            cont = c
            let config = WKWebViewConfiguration()
            config.websiteDataStore = .nonPersistent()
            config.userContentController.add(self, name: "done")
            let w = WKWebView(frame: CGRect(x: 0, y: 0, width: 390, height: 600), configuration: config)
            web = w
            w.loadHTMLString(html, baseURL: nil)
            Task { @MainActor in
                try? await Task.sleep(nanoseconds: 20_000_000_000)
                self.finish(false, "timeout")
            }
        }
    }

    private func finish(_ ok: Bool, _ text: String) {
        guard let c = cont else { return }
        cont = nil
        let title = web?.title ?? ""
        web?.configuration.userContentController.removeScriptMessageHandler(forName: "done")
        c.resume(returning: (ok, text, title))
    }

    func userContentController(_ c: WKUserContentController, didReceive m: WKScriptMessage) {
        let body = m.body as? [String: Any]
        finish(body?["ok"] as? Bool == true, body?["error"] as? String ?? "")
    }
}

@MainActor
final class FeedCopyTests: XCTestCase {
    private func built(_ text: String) -> NSAttributedString {
        let turn = TranscriptTurn(id: "t1", stepIndex: 0, turnType: .assistant, timestamp: nil, content: text,
                                  thinking: nil, toolCalls: [], status: .done, label: nil)
        return FeedAttributedBuilder.build(turns: [turn], expandedThinking: [], expandedTools: [], dark: false)
    }

    private func index(of needle: String, in s: NSAttributedString) -> Int {
        (s.string as NSString).range(of: needle).location
    }

    func testParagraphListAndCodeAreCopiedAsBlocks() {
        let s = built("First paragraph here.\n\n- one\n- two\n\n```swift\nlet x = 1\nlet y = 2\n```")
        XCTAssertEqual(FeedCopy.block(in: s, at: index(of: "paragraph", in: s)), "First paragraph here.")
        XCTAssertEqual(FeedCopy.block(in: s, at: index(of: "two", in: s)), "- one\n- two")
        XCTAssertEqual(FeedCopy.block(in: s, at: index(of: "let y", in: s)), "let x = 1\nlet y = 2")
    }

    func testInlineCodeIsMarkedAndTheMessageHasACopyAllLink() {
        let s = built("Run `make test` now")
        let i = index(of: "make test", in: s)
        XCTAssertEqual(s.attribute(FeedCopy.inlineKey, at: i, effectiveRange: nil) as? String, "make test")
        let all = index(of: "Copy all", in: s)
        let url = s.attribute(.link, at: all, effectiveRange: nil) as? URL
        XCTAssertEqual(url.flatMap(FeedCopy.messageID), "t1")
    }
}

final class DialogTabKeysTests: XCTestCase {
    func testSwitchingTabsSendsArrowsByTheDistance() {
        XCTAssertEqual(DialogKeys.switchTab(from: 1, to: 1).count, 0)
        XCTAssertEqual(DialogKeys.switchTab(from: 1, to: 4), Array(repeating: DialogKeys.right, count: 3))
        XCTAssertEqual(DialogKeys.switchTab(from: 3, to: 0), Array(repeating: DialogKeys.left, count: 3))
    }
}

final class FeedFileRefTests: XCTestCase {
    func testWholeCodeSpans() {
        XCTAssertEqual(FeedFileRef.parse("src/main.rs"), FeedFileRef(path: "src/main.rs", line: nil))
        XCTAssertEqual(FeedFileRef.parse("Cargo.toml:12"), FeedFileRef(path: "Cargo.toml", line: 12))
        XCTAssertEqual(FeedFileRef.parse("a/b/lib.rs:40:7"), FeedFileRef(path: "a/b/lib.rs", line: 40))
        XCTAssertEqual(FeedFileRef.parse("/etc/app/config.yaml#L9"), FeedFileRef(path: "/etc/app/config.yaml", line: 9))
        XCTAssertNil(FeedFileRef.parse("git push --force"))
        XCTAssertNil(FeedFileRef.parse("e.g."))
        XCTAssertNil(FeedFileRef.parse("v1.2"))
        XCTAssertNil(FeedFileRef.parse("foo.bar"))
        XCTAssertNil(FeedFileRef.parse("https://example.com/a.rs"))
    }

    func testMentionsInPlainText() {
        let text = "See crates/core/src/screen/mod.rs:277 and Package.swift, but not e.g. v1.2 or https://x.io/y.rs."
        let found = FeedFileRef.scan(text)
        XCTAssertEqual(found.map(\.ref), [
            FeedFileRef(path: "crates/core/src/screen/mod.rs", line: 277),
            FeedFileRef(path: "Package.swift", line: nil),
        ])
        let ns = text as NSString
        XCTAssertEqual(ns.substring(with: found[0].range), "crates/core/src/screen/mod.rs:277")
    }

    func testEncodingRoundTrip() {
        let r = FeedFileRef(path: "a/b.rs", line: 3)
        XCTAssertEqual(FeedFileRef(encoded: r.encoded), r)
        XCTAssertEqual(FeedFileRef(encoded: FeedFileRef(path: "x.rs", line: nil).encoded)?.line, nil)
    }

    @MainActor
    func testBuilderMarksMentionsInCodeAndText() {
        let turn = TranscriptTurn(id: "t1", stepIndex: 0, turnType: .assistant, timestamp: nil, content: "Edit `src/lib.rs:5` and notes.md now; `cargo test` stays code.",
                                  thinking: nil, toolCalls: [], status: .done, label: nil)
        let text = FeedAttributedBuilder.build(turns: [turn], expandedThinking: [], expandedTools: [], dark: false)
        var marked: [String] = []
        text.enumerateAttribute(FeedFileRef.key, in: NSRange(location: 0, length: text.length)) { v, _, _ in
            if let v = v as? String { marked.append(v) }
        }
        XCTAssertEqual(marked, ["src/lib.rs\u{0}5", "notes.md\u{0}"])
    }
}
