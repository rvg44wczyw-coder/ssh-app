import SwiftUI
import Combine
import SwiftTerm
#if canImport(UIKit)
import UIKit

/// Subclass of SwiftTerm's TerminalView that protects 1-finger vertical scroll
/// from being hijacked by SwiftTerm's internal panSelectionGesture / panMouseGesture.
open class AppTerminalView: TerminalView {
    private var isSetupComplete = false
    private var allowSecondaryPan = false
    private var wheelRemainder: CGFloat = 0
    /// Called with (scrollUp, number of wheel ticks, column, row) for a one-finger vertical drag.
    public var onWheel: ((Bool, Int, Int, Int) -> Void)?

    public override init(frame: CGRect, font: UIFont? = nil, options: TerminalOptions = TerminalOptions.default) {
        super.init(frame: frame, font: font, options: options)
        isSetupComplete = true
        setupScrollConfig()
    }

    public required init?(coder: NSCoder) {
        super.init(coder: coder)
        isSetupComplete = true
        setupScrollConfig()
    }

    private func setupScrollConfig() {
        self.isScrollEnabled = true
        self.alwaysBounceVertical = true
        self.bounces = true
        self.showsVerticalScrollIndicator = true
        self.showsHorizontalScrollIndicator = false
        self.indicatorStyle = .white
        self.keyboardDismissMode = .onDrag

        self.panGestureRecognizer.minimumNumberOfTouches = 1
        self.panGestureRecognizer.maximumNumberOfTouches = 1
        self.panGestureRecognizer.cancelsTouchesInView = false
        self.panGestureRecognizer.delaysTouchesBegan = false

        for gr in gestureRecognizers ?? [] {
            if gr is UIPanGestureRecognizer && gr !== self.panGestureRecognizer {
                gr.isEnabled = false
                removeGestureRecognizer(gr)
            }
        }
    }

    /// The host's tmux owns the history (the client only ever receives the visible screen), so a
    /// one-finger drag is sent to tmux as mouse-wheel events instead of moving the local buffer.
    /// tmux then walks its scrollback in copy mode (`mouse on` is set for the session on attach).
    func installWheelScroll() {
        panGestureRecognizer.isEnabled = false
        let pan = UIPanGestureRecognizer(target: self, action: #selector(handleWheelPan(_:)))
        pan.minimumNumberOfTouches = 1
        pan.maximumNumberOfTouches = 1
        pan.cancelsTouchesInView = false
        allowSecondaryPan = true
        super.addGestureRecognizer(pan)
        allowSecondaryPan = false
    }

    /// 1-based terminal cell under a point of this view (accounts for the scroll offset).
    func cell(at point: CGPoint) -> (col: Int, row: Int) {
        let terminal = getTerminal()
        let cols = max(terminal.cols, 1), rows = max(terminal.rows, 1)
        let x = Int(point.x / (bounds.width / CGFloat(cols))) + 1
        let y = Int((point.y - contentOffset.y) / (bounds.height / CGFloat(rows))) + 1
        return (min(max(x, 1), cols), min(max(y, 1), rows))
    }

    private var scrollLink: CADisplayLink?
    private var wheelDismissedKeyboard = false
    private var momentumVelocity: CGFloat = 0
    /// Wheel ticks computed but not yet sent (positive = wheel up). They are paced out one frame
    /// at a time so tmux gets an even stream instead of bursts, which is what looks jerky.
    private var pendingTicks = 0

    private func stopMomentum() {
        momentumVelocity = 0
    }

    private func startScrollLink() {
        guard scrollLink == nil else { return }
        let link = CADisplayLink(target: self, selector: #selector(scrollTick(_:)))
        link.add(to: .main, forMode: .common)
        scrollLink = link
    }

    private func stopScrollLink() {
        scrollLink?.invalidate()
        scrollLink = nil
    }

    /// Finger distance (points) per tmux wheel tick (tmux moves about five lines per tick).
    private var wheelStep: CGFloat {
        max(bounds.height / CGFloat(max(getTerminal().rows, 1)) * 1.5, 12)
    }

    private func emitWheel(_ distance: CGFloat) {
        wheelRemainder += distance
        let ticks = Int(wheelRemainder / wheelStep)
        guard ticks != 0 else { return }
        wheelRemainder -= CGFloat(ticks) * wheelStep
        // A change of direction drops what is still queued the other way.
        if (pendingTicks > 0) != (ticks > 0) { pendingTicks = 0 }
        pendingTicks = max(-60, min(60, pendingTicks + ticks))
        startScrollLink()
    }

    @objc private func scrollTick(_ link: CADisplayLink) {
        let dt = CGFloat(link.targetTimestamp - link.timestamp)
        if momentumVelocity != 0 {
            emitWheel(momentumVelocity * dt)
            // Frame-rate independent decay (about 5% per frame at 60 Hz).
            momentumVelocity *= exp(-3.0 * dt)
            if abs(momentumVelocity) < 60 { momentumVelocity = 0 }
        }
        if pendingTicks != 0 {
            // One tick per frame, a few more when a backlog builds up.
            let n = min(abs(pendingTicks), max(1, abs(pendingTicks) / 4))
            let up = pendingTicks > 0
            pendingTicks += up ? -n : n
            let terminal = getTerminal()
            onWheel?(up, n, max(terminal.cols / 2, 1), max(terminal.rows / 2, 1))
        } else if momentumVelocity == 0 {
            stopScrollLink()
        }
    }

    @objc private func handleWheelPan(_ pan: UIPanGestureRecognizer) {
        switch pan.state {
        case .began:
            stopMomentum()
            pendingTicks = 0
            wheelRemainder = 0
            wheelDismissedKeyboard = false
        case .changed:
            // Same as the scroll view's `.onDrag`, which the wheel pan replaced: a drag hides the keyboard.
            if !wheelDismissedKeyboard, abs(pan.translation(in: self).y) > 8 {
                wheelDismissedKeyboard = true
                if isFirstResponder { _ = resignFirstResponder() }
            }
            let dy = pan.translation(in: self).y
            pan.setTranslation(.zero, in: self)
            emitWheel(dy)
        case .ended:
            let v = pan.velocity(in: self).y
            guard abs(v) > 300 else { return }
            momentumVelocity = v
            startScrollLink()
        default:
            break
        }
    }

    open override func addGestureRecognizer(_ gestureRecognizer: UIGestureRecognizer) {
        if isSetupComplete {
            // SwiftTerm's enableSelectionPanGesture() and enableMousePanGesture() add secondary UIPanGestureRecognizers
            // that hijack 1-finger vertical swipes to send arrow keys or mouse reporting instead of scrolling!
            // We block and disable any such secondary pan gesture so 1-finger drag always scrolls the terminal.
            if gestureRecognizer is UIPanGestureRecognizer && gestureRecognizer !== self.panGestureRecognizer && !allowSecondaryPan {
                print("[AppTerminalView] Blocked secondary pan gesture from hijacking scroll: \(gestureRecognizer)")
                gestureRecognizer.isEnabled = false
                return
            }
        }
        super.addGestureRecognizer(gestureRecognizer)
    }
}

public struct TerminalContainerView: UIViewRepresentable {
    @ObservedObject public var viewModel: TerminalSessionViewModel
    @Binding public var isCtrlLocked: Bool
    public var scheme: TerminalScheme = .fallback
    @AppStorage("terminal_font_size") private var terminalFontSize: Double = 13.0

    public init(viewModel: TerminalSessionViewModel, isCtrlLocked: Binding<Bool>, scheme: TerminalScheme = .fallback) {
        self.viewModel = viewModel
        self._isCtrlLocked = isCtrlLocked
        self.scheme = scheme
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(viewModel: viewModel, isCtrlLocked: $isCtrlLocked)
    }

    public func makeUIView(context: Context) -> TerminalView {
        var options = TerminalOptions.default
        options.scrollback = 10_000
        let fontSize = CGFloat(terminalFontSize)
        let terminalView = AppTerminalView(
            frame: .zero,
            font: UIFont.monospacedSystemFont(ofSize: fontSize, weight: .regular),
            options: options
        )
        terminalView.changeScrollback(10_000)
        terminalView.terminalDelegate = context.coordinator
        terminalView.delegate = context.coordinator
        context.coordinator.applyScheme(scheme, to: terminalView)
        terminalView.isUserInteractionEnabled = true
        terminalView.allowMouseReporting = false

        // Setup text selection appearance
        terminalView.selectionHandleColor = UIColor(AppAccent.currentColor)

        // Setup modern iOS Edit Menu Interaction for text selection actions
        let editInteraction = UIEditMenuInteraction(delegate: context.coordinator)
        terminalView.addInteraction(editInteraction)
        context.coordinator.editMenuInteraction = editInteraction

        // Remove SwiftTerm's default single tap, double tap, and long press recognizers:
        for gr in terminalView.gestureRecognizers ?? [] {
            if gr is UILongPressGestureRecognizer {
                terminalView.removeGestureRecognizer(gr)
            } else if let tap = gr as? UITapGestureRecognizer {
                if (1...3).contains(tap.numberOfTapsRequired) {
                    terminalView.removeGestureRecognizer(tap)
                }
            } else if gr is UIPanGestureRecognizer && gr !== terminalView.panGestureRecognizer {
                terminalView.removeGestureRecognizer(gr)
            }
        }

        // Add single-tap gesture: 1 tap summons the keyboard immediately
        let singleTap = UITapGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.handleSingleTap(_:)))
        singleTap.numberOfTapsRequired = 1
        singleTap.cancelsTouchesInView = false
        terminalView.addGestureRecognizer(singleTap)

        // Add custom double-tap gesture to select word
        let doubleTap = UITapGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.handleDoubleTap(_:)))
        doubleTap.numberOfTapsRequired = 2
        doubleTap.cancelsTouchesInView = false
        // Triple tap selects the whole paragraph (the run of non-blank lines around the tap).
        let tripleTap = UITapGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.handleTripleTap(_:)))
        tripleTap.numberOfTapsRequired = 3
        tripleTap.cancelsTouchesInView = false
        terminalView.addGestureRecognizer(tripleTap)
        doubleTap.require(toFail: tripleTap)
        // The first tap of a double or triple tap must not raise the keyboard: it resized the view
        // and made the selection jump.
        singleTap.require(toFail: doubleTap)
        terminalView.addGestureRecognizer(doubleTap)

        // Add custom long-press gesture for text selection
        let longPress = UILongPressGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.handleLongPress(_:)))
        longPress.minimumPressDuration = 0.4
        longPress.cancelsTouchesInView = false
        terminalView.addGestureRecognizer(longPress)

        // Add pinch gesture for smooth zoom in/out
        let pinch = UIPinchGestureRecognizer(target: context.coordinator, action: #selector(Coordinator.handlePinch(_:)))
        terminalView.addGestureRecognizer(pinch)

        terminalView.installWheelScroll()
        terminalView.onWheel = { [weak coordinator = context.coordinator] up, count, col, row in
            // SGR mouse report: button 64 = wheel up, 65 = wheel down.
            let report = "\u{1b}[<\(up ? 64 : 65);\(col);\(row)M"
            let data = Data(String(repeating: report, count: count).utf8)
            coordinator?.viewModel.sendInput(data)
        }

        context.coordinator.terminalView = terminalView
        context.coordinator.handles = SelectionHandles(
            terminalView: terminalView, color: UIColor(AppAccent.currentColor)
        ) { [weak coordinator = context.coordinator] point in
            coordinator?.presentSelectionMenu(at: point)
        }

        viewModel.pasteHandler = { [weak coordinator = context.coordinator] in coordinator?.pasteClipboard() }

        print("[TerminalView \(viewModel.instanceId)] makeUIView created for session '\(viewModel.session.title)'")

        // Feed existing buffered history immediately so screen is never blank,
        // while strictly suppressing automated terminal responses (e.g. DA1, DA2, XTVERSION, size queries)
        let history = viewModel.getHistoryBuffer()
        context.coordinator.feedHistoryBuffer(history, to: terminalView)

        // Subscribe to throttled output stream from Rust Core
        context.coordinator.cancellable = viewModel.outputPublisher
            .receive(on: DispatchQueue.main)
            .sink { [weak terminalView] data in
                let bytes = ArraySlice([UInt8](data))
                terminalView?.feed(byteArray: bytes)
            }

        return terminalView
    }

    public func updateUIView(_ uiView: TerminalView, context: Context) {
        context.coordinator.applyScheme(scheme, to: uiView)
        let currentSize = CGFloat(terminalFontSize)
        if abs(uiView.font.pointSize - currentSize) >= 0.5 {
            print("[TerminalView] Updating font size to \(currentSize) pt")
            uiView.font = UIFont.monospacedSystemFont(ofSize: currentSize, weight: .regular)
        }
        if context.coordinator.viewModel !== viewModel {
            print("[TerminalView] updateUIView switching to vm \(viewModel.instanceId)")
            context.coordinator.updateViewModel(viewModel, for: uiView)
        }
    }

    @MainActor
    public final class Coordinator: NSObject, @preconcurrency TerminalViewDelegate, UIScrollViewDelegate, @preconcurrency UIEditMenuInteractionDelegate {
        var viewModel: TerminalSessionViewModel
        @Binding private var isCtrlLocked: Bool
        var cancellable: AnyCancellable?
        weak var terminalView: TerminalView?
        weak var editMenuInteraction: UIEditMenuInteraction?
        var handles: SelectionHandles?
        var isReplayingHistory: Bool = false

        init(viewModel: TerminalSessionViewModel, isCtrlLocked: Binding<Bool>) {
            self.viewModel = viewModel
            self._isCtrlLocked = isCtrlLocked
        }

        private var appliedSchemeId: String?

        /// Colours of the terminal; applied only when the scheme changes.
        func applyScheme(_ scheme: TerminalScheme, to tv: TerminalView) {
            guard appliedSchemeId != scheme.id else { return }
            appliedSchemeId = scheme.id
            let palette = scheme.ansi.map { hex -> SwiftTerm.Color in
                let rgb = UInt32(hex, radix: 16) ?? 0
                return SwiftTerm.Color(
                    red8: UInt16((rgb >> 16) & 0xff), green8: UInt16((rgb >> 8) & 0xff), blue8: UInt16(rgb & 0xff))
            }
            tv.installColors(palette)
            tv.nativeBackgroundColor = UIColor(hex: scheme.background)
            tv.nativeForegroundColor = UIColor(hex: scheme.foreground)
            tv.caretColor = UIColor(hex: scheme.cursor)
            tv.backgroundColor = UIColor(hex: scheme.background)
            tv.setNeedsDisplay()
        }

        func feedHistoryBuffer(_ history: Data, to terminalView: TerminalView) {
            guard !history.isEmpty else { return }
            print("[TerminalView \(viewModel.instanceId)] Feeding history buffer: \(history.count) bytes (suppressing auto-responses)")
            isReplayingHistory = true
            defer { isReplayingHistory = false }
            terminalView.feed(byteArray: ArraySlice([UInt8](history)))
        }

        func updateViewModel(_ newVM: TerminalSessionViewModel, for terminalView: TerminalView) {
            self.viewModel = newVM
            self.cancellable?.cancel()
            let history = newVM.getHistoryBuffer()
            feedHistoryBuffer(history, to: terminalView)
            self.cancellable = newVM.outputPublisher
                .receive(on: DispatchQueue.main)
                .sink { [weak terminalView] data in
                    let bytes = ArraySlice([UInt8](data))
                    terminalView?.feed(byteArray: bytes)
                }
        }

        // MARK: Selection
        // Selection works on the terminal's own grid and never makes the terminal first responder,
        // so selecting text does not raise the keyboard (which resized the view and made the
        // selection jump).

        private func cell(at point: CGPoint, in tv: TerminalView) -> (col: Int, row: Int) {
            let term = tv.getTerminal()
            let size = terminalCellSize(tv)
            // `point` is in content space; the grid rows the API takes are screen rows.
            let screenY = point.y - tv.contentOffset.y
            return (
                min(max(Int(point.x / size.width), 0), term.cols - 1),
                min(max(Int(screenY / size.height), 0), term.rows - 1)
            )
        }

        private func rowText(_ row: Int, in tv: TerminalView) -> [Character] {
            let term = tv.getTerminal()
            guard let line = term.getLine(row: row) else { return [] }
            return (0..<term.cols).map { line[$0].getCharacter() }
        }

        private func isWordChar(_ c: Character) -> Bool {
            c.isLetter || c.isNumber || "._-/~:@%+=".contains(c)
        }

        /// Selects the word (path, URL, identifier) under the cell; false when it is blank.
        @discardableResult
        private func selectWord(at pos: (col: Int, row: Int), in tv: TerminalView) -> Bool {
            let chars = rowText(pos.row, in: tv)
            guard pos.col < chars.count, isWordChar(chars[pos.col]) else { return false }
            var lo = pos.col, hi = pos.col
            while lo > 0, isWordChar(chars[lo - 1]) { lo -= 1 }
            while hi < chars.count - 1, isWordChar(chars[hi + 1]) { hi += 1 }
            tv.selection.startSelection(row: pos.row, col: lo)
            tv.selection.dragExtend(row: pos.row, col: hi + 1)
            tv.setNeedsDisplay()
            handles?.refresh()
            return true
        }

        /// Pastes the clipboard the way a terminal does: wrapped in a bracketed paste when the
        /// program asked for it (Claude Code does), so several lines arrive as one input and not
        /// as one Enter per line; plain text with Enter for line breaks otherwise.
        func pasteClipboard() {
            guard let raw = UIPasteboard.general.string, !raw.isEmpty else { return }
            let bracketed = terminalView?.getTerminal().bracketedPasteMode == true
            viewModel.sendInput(Data(TerminalPaste.bytes(raw, bracketed: bracketed)))
        }

        func presentSelectionMenu(at point: CGPoint) {
            guard terminalView?.hasActiveSelection == true else { return }
            UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            editMenuInteraction?.presentEditMenu(with: UIEditMenuConfiguration(identifier: nil, sourcePoint: point))
        }

        @objc func handleDoubleTap(_ gesture: UITapGestureRecognizer) {
            guard gesture.state == .ended, let tv = terminalView else { return }
            let point = gesture.location(in: tv)
            if selectWord(at: cell(at: point, in: tv), in: tv) { presentSelectionMenu(at: point) }
        }

        /// Triple tap selects the whole paragraph: the run of non-blank lines around the tap.
        @objc func handleTripleTap(_ gesture: UITapGestureRecognizer) {
            guard gesture.state == .ended, let tv = terminalView else { return }
            let point = gesture.location(in: tv)
            let term = tv.getTerminal()
            let tapped = cell(at: point, in: tv).row
            let blank: (Int) -> Bool = { row in
                self.rowText(row, in: tv).allSatisfy { $0 == " " || $0 == "\0" }
            }
            // Triple tap on an empty line pastes the clipboard (as the keyboard's Paste button does).
            guard !blank(tapped) else {
                UIImpactFeedbackGenerator(style: .light).impactOccurred()
                pasteClipboard()
                if !tv.isFirstResponder { _ = tv.becomeFirstResponder() }
                return
            }
            var first = tapped, last = tapped
            while first > 0, !blank(first - 1) { first -= 1 }
            while last < term.rows - 1, !blank(last + 1) { last += 1 }
            // To the last used column of the final line, so the copy has no trailing padding.
            let tail = rowText(last, in: tv)
            let end = (tail.lastIndex { $0 != " " && $0 != "\0" } ?? 0) + 1
            tv.selection.startSelection(row: first, col: 0)
            tv.selection.dragExtend(row: last, col: end)
            tv.setNeedsDisplay()
            handles?.refresh()
            presentSelectionMenu(at: point)
        }

        /// Hold to select the word under the finger, keep it down and drag to extend the
        /// selection (the first recognised gesture wins, so the scroll pan stays out of it).
        @objc func handleLongPress(_ gesture: UILongPressGestureRecognizer) {
            guard let tv = terminalView else { return }
            let point = gesture.location(in: tv)
            let pos = cell(at: point, in: tv)
            switch gesture.state {
            case .began:
                editMenuInteraction?.dismissMenu()
                if !selectWord(at: pos, in: tv) {
                    tv.selection.startSelection(row: pos.row, col: pos.col)
                    tv.selection.dragExtend(row: pos.row, col: pos.col + 1)
                    tv.setNeedsDisplay()
                    handles?.refresh()
                }
                UIImpactFeedbackGenerator(style: .medium).impactOccurred()
            case .changed:
                guard tv.hasActiveSelection else { return }
                tv.selection.dragExtend(row: pos.row, col: pos.col)
                tv.setNeedsDisplay()
                handles?.refresh()
            case .ended:
                presentSelectionMenu(at: point)
            default:
                break
            }
        }

        @objc func handleSingleTap(_ gesture: UITapGestureRecognizer) {
            guard let tv = terminalView else { return }
            // A tap is also a left click for the program in the pane (e.g. the "jump to bottom"
            // button of an agent UI); tmux forwards it when the program asked for the mouse.
            if let app = tv as? AppTerminalView, !tv.hasActiveSelection {
                let c = app.cell(at: gesture.location(in: tv))
                viewModel.sendInput(Data("\u{1b}[<0;\(c.col);\(c.row)M\u{1b}[<0;\(c.col);\(c.row)m".utf8))
            }
            if tv.hasActiveSelection {
                tv.clearSelection()
                handles?.refresh()
                editMenuInteraction?.dismissMenu()
            }
            if !tv.isFirstResponder {
                _ = tv.becomeFirstResponder()
            }
        }

        private var initialPinchSize: CGFloat = 13

        @objc func handlePinch(_ gesture: UIPinchGestureRecognizer) {
            guard let tv = terminalView else { return }
            if gesture.state == .began {
                initialPinchSize = tv.font.pointSize
            } else if gesture.state == .changed {
                let targetSize = min(max((initialPinchSize * gesture.scale).rounded(), 8), 24)
                if abs(tv.font.pointSize - targetSize) >= 1.0 {
                    tv.font = UIFont.monospacedSystemFont(ofSize: targetSize, weight: .regular)
                    UserDefaults.standard.set(Double(targetSize), forKey: "terminal_font_size")
                }
            }
        }

        // MARK: - UIEditMenuInteractionDelegate

        public func editMenuInteraction(
            _ interaction: UIEditMenuInteraction,
            menuFor configuration: UIEditMenuConfiguration,
            suggestedActions: [UIMenuElement]
        ) -> UIMenu? {
            guard let tv = terminalView else { return nil }

            var actions: [UIMenuElement] = []

            if let selectedText = tv.getSelection(), !selectedText.isEmpty {
                let copyAction = UIAction(title: "Copy", image: UIImage(systemName: "doc.on.doc")) { [weak tv, weak self] _ in
                    UIPasteboard.general.string = selectedText
                    let feedback = UINotificationFeedbackGenerator()
                    feedback.notificationOccurred(.success)
                    tv?.clearSelection()
                    self?.handles?.refresh()
                }
                actions.append(copyAction)

                let shareAction = UIAction(title: "Share...", image: UIImage(systemName: "square.and.arrow.up")) { [weak tv] _ in
                    guard let tv = tv else { return }
                    let activityVC = UIActivityViewController(activityItems: [selectedText], applicationActivities: nil)
                    if let rootVC = tv.window?.rootViewController {
                        var presenter = rootVC
                        while let presented = presenter.presentedViewController {
                            presenter = presented
                        }
                        presenter.present(activityVC, animated: true)
                    }
                }
                actions.append(shareAction)
            }

            let selectAllAction = UIAction(title: "Select All", image: UIImage(systemName: "selection.pin.in.out")) { [weak self, weak tv] _ in
                guard let tv = tv, let self = self else { return }
                tv.selectAll()
                self.handles?.refresh()
                let feedback = UIImpactFeedbackGenerator(style: .light)
                feedback.impactOccurred()
                let config = UIEditMenuConfiguration(identifier: nil, sourcePoint: CGPoint(x: tv.bounds.midX, y: tv.bounds.midY))
                self.editMenuInteraction?.presentEditMenu(with: config)
            }
            actions.append(selectAllAction)

            if UIPasteboard.general.hasStrings {
                let pasteAction = UIAction(title: "Paste", image: UIImage(systemName: "doc.on.clipboard")) { [weak self] _ in
                    self?.pasteClipboard()
                }
                actions.append(pasteAction)
            }

            return UIMenu(children: actions)
        }

        // MARK: - UIScrollViewDelegate

        public func scrollViewWillBeginDragging(_ scrollView: UIScrollView) {
            // Dismiss keyboard when dragging begins (scrolling on top or history)
            if let tv = terminalView, tv.isFirstResponder {
                _ = tv.resignFirstResponder()
            }
            editMenuInteraction?.dismissMenu()
        }

        public func scrollViewDidScroll(_ scrollView: UIScrollView) {
            // Guarantee keyboard is hidden when user scrolls
            if let tv = terminalView, tv.isFirstResponder, (scrollView.isTracking || scrollView.isDragging) {
                _ = tv.resignFirstResponder()
            }
        }

        // Forward keystrokes to Rust SSH channel
        public func send(source: TerminalView, data: ArraySlice<UInt8>) {
            if isReplayingHistory {
                print("[TerminalView \(viewModel.instanceId)] Dropped auto-response during history replay (\(data.count) bytes)")
                return
            }

            var bytesToSend = [UInt8](data)

            // Defense-in-depth: Never send synthetic terminal capability / window size responses to the remote shell
            if bytesToSend.starts(with: [0x1B, 0x50, 0x3E, 0x7C]) // \033P>| (XTVERSION)
                || bytesToSend.starts(with: [0x1B, 0x5B, 0x3F, 0x36, 0x35]) // \033[?65 (Primary DA)
                || bytesToSend.starts(with: [0x1B, 0x5B, 0x3E, 0x36, 0x35]) // \033[>65 (Secondary DA)
                || (bytesToSend.starts(with: [0x1B, 0x5B, 0x34, 0x3B]) && bytesToSend.last == 0x74) // \033[4;...t (Window pixel size)
                || (bytesToSend.starts(with: [0x1B, 0x5B, 0x38, 0x3B]) && bytesToSend.last == 0x74) // \033[8;...t (Window char size)
            {
                print("[TerminalView \(viewModel.instanceId)] Blocked synthetic terminal report: \(bytesToSend)")
                return
            }

            print("[TerminalView \(viewModel.instanceId)] send() bytes: len=\(bytesToSend.count)")

            // If CTRL lock is active, transform alphabetical keystrokes to control codes
            if isCtrlLocked && bytesToSend.count == 1 {
                let char = bytesToSend[0]
                if (char >= 0x61 && char <= 0x7A) { // 'a'...'z'
                    bytesToSend = [char - 0x60] // 0x01...0x1A
                    isCtrlLocked = false
                } else if (char >= 0x41 && char <= 0x5A) { // 'A'...'Z'
                    bytesToSend = [char - 0x40]
                    isCtrlLocked = false
                }
            }

            viewModel.sendInput(Data(bytesToSend))
        }

        public func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
            print("[TerminalView \(viewModel.instanceId)] sizeChanged cols=\(newCols), rows=\(newRows)")
            viewModel.resize(cols: UInt16(newCols), rows: UInt16(newRows))
        }

        public func setTerminalTitle(source: TerminalView, title: String) {}
        public func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
        public func scrolled(source: TerminalView, position: Double) {}
        public func requestOpenLink(source: TerminalView, link: String, params: [String : String]) {}
        public func bell(source: TerminalView) {}
        public func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
    }
}
#endif


/// Bytes for a terminal paste. An end-of-paste marker inside the text is removed, so pasted
/// content cannot leave the paste early and run as keystrokes.
enum TerminalPaste {
    static func bytes(_ text: String, bracketed: Bool) -> [UInt8] {
        let clean = text
            .replacingOccurrences(of: "\u{1b}[201~", with: "")
            .replacingOccurrences(of: "\u{1b}[200~", with: "")
            .replacingOccurrences(of: "\r\n", with: "\n")
            .replacingOccurrences(of: "\r", with: "\n")
        if bracketed {
            return Array("\u{1b}[200~".utf8) + Array(clean.utf8) + Array("\u{1b}[201~".utf8)
        }
        return Array(clean.replacingOccurrences(of: "\n", with: "\r").utf8)
    }
}
