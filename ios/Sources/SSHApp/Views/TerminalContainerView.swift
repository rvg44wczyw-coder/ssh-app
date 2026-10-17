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

    private var momentumLink: CADisplayLink?
    private var wheelDismissedKeyboard = false
    private var momentumVelocity: CGFloat = 0

    private func stopMomentum() {
        momentumLink?.invalidate()
        momentumLink = nil
    }

    /// Finger distance (points) per tmux wheel tick. One tick moves about five lines, so one row
    /// of finger travel per tick makes the text move several times faster than the finger.
    private var wheelStep: CGFloat {
        max(bounds.height / CGFloat(max(getTerminal().rows, 1)), 8)
    }

    private func emitWheel(_ distance: CGFloat) {
        wheelRemainder += distance
        let ticks = Int(wheelRemainder / wheelStep)
        guard ticks != 0 else { return }
        wheelRemainder -= CGFloat(ticks) * wheelStep
        let terminal = getTerminal()
        // Dragging down shows older output, which is a wheel-up event.
        onWheel?(ticks > 0, min(abs(ticks), 40), max(terminal.cols / 2, 1), max(terminal.rows / 2, 1))
    }

    @objc private func momentumTick(_ link: CADisplayLink) {
        let dt = CGFloat(link.targetTimestamp - link.timestamp)
        emitWheel(momentumVelocity * dt)
        momentumVelocity *= 0.95
        if abs(momentumVelocity) < 60 { stopMomentum() }
    }

    @objc private func handleWheelPan(_ pan: UIPanGestureRecognizer) {
        switch pan.state {
        case .began:
            stopMomentum()
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
            let link = CADisplayLink(target: self, selector: #selector(momentumTick(_:)))
            link.add(to: .main, forMode: .common)
            momentumLink = link
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
        terminalView.selectionHandleColor = UIColor(AppAccent.current.color)

        // Setup modern iOS Edit Menu Interaction for text selection actions
        let editInteraction = UIEditMenuInteraction(delegate: context.coordinator)
        terminalView.addInteraction(editInteraction)
        context.coordinator.editMenuInteraction = editInteraction

        // Remove SwiftTerm's default single tap, double tap, and long press recognizers:
        for gr in terminalView.gestureRecognizers ?? [] {
            if gr is UILongPressGestureRecognizer {
                terminalView.removeGestureRecognizer(gr)
            } else if let tap = gr as? UITapGestureRecognizer {
                if tap.numberOfTapsRequired == 1 || tap.numberOfTapsRequired == 2 {
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
        if let tripleTap = terminalView.gestureRecognizers?.first(where: { ($0 as? UITapGestureRecognizer)?.numberOfTapsRequired == 3 }) {
            doubleTap.require(toFail: tripleTap)
        }
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

        @objc func handleDoubleTap(_ gesture: UITapGestureRecognizer) {
            guard let tv = terminalView else { return }
            let point = gesture.location(in: tv)

            // Ensure keyboard is summoned on double tap
            if !tv.isFirstResponder {
                _ = tv.becomeFirstResponder()
            }

            // Also select word if tapping on text
            tv.showStandardContextMenu(at: point)
            tv.select(nil)

            if tv.hasActiveSelection {
                let generator = UIImpactFeedbackGenerator(style: .medium)
                generator.impactOccurred()
                let config = UIEditMenuConfiguration(identifier: nil, sourcePoint: point)
                editMenuInteraction?.presentEditMenu(with: config)
            }
        }

        @objc func handleLongPress(_ gesture: UILongPressGestureRecognizer) {
            guard gesture.state == .began, let tv = terminalView else { return }
            let point = gesture.location(in: tv)

            tv.showStandardContextMenu(at: point)
            tv.select(nil)

            let generator = UIImpactFeedbackGenerator(style: .medium)
            generator.impactOccurred()

            let config = UIEditMenuConfiguration(identifier: nil, sourcePoint: point)
            editMenuInteraction?.presentEditMenu(with: config)
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
                let copyAction = UIAction(title: "Copy", image: UIImage(systemName: "doc.on.doc")) { [weak tv] _ in
                    UIPasteboard.general.string = selectedText
                    let feedback = UINotificationFeedbackGenerator()
                    feedback.notificationOccurred(.success)
                    tv?.clearSelection()
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
                let feedback = UIImpactFeedbackGenerator(style: .light)
                feedback.impactOccurred()
                let config = UIEditMenuConfiguration(identifier: nil, sourcePoint: CGPoint(x: tv.bounds.midX, y: tv.bounds.midY))
                self.editMenuInteraction?.presentEditMenu(with: config)
            }
            actions.append(selectAllAction)

            if UIPasteboard.general.hasStrings {
                let pasteAction = UIAction(title: "Paste", image: UIImage(systemName: "doc.on.clipboard")) { [weak self] _ in
                    if let str = UIPasteboard.general.string {
                        self?.viewModel.sendInput(Data(str.utf8))
                    }
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
