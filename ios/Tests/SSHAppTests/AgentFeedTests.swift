import XCTest
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
}
