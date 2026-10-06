import SwiftUI
import Combine
import SwiftTerm
#if canImport(UIKit)
import UIKit

/// Subclass of SwiftTerm's TerminalView that protects 1-finger vertical scroll
/// from being hijacked by SwiftTerm's internal panSelectionGesture / panMouseGesture.
open class AppTerminalView: TerminalView {
    private var isSetupComplete = false

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

    open override func addGestureRecognizer(_ gestureRecognizer: UIGestureRecognizer) {
        if isSetupComplete {
            // SwiftTerm's enableSelectionPanGesture() and enableMousePanGesture() add secondary UIPanGestureRecognizers
            // that hijack 1-finger vertical swipes to send arrow keys or mouse reporting instead of scrolling!
            // We block and disable any such secondary pan gesture so 1-finger drag always scrolls the terminal.
            if gestureRecognizer is UIPanGestureRecognizer && gestureRecognizer !== self.panGestureRecognizer {
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
    @AppStorage("terminal_font_size") private var terminalFontSize: Double = 13.0

    public init(viewModel: TerminalSessionViewModel, isCtrlLocked: Binding<Bool>) {
        self.viewModel = viewModel
        self._isCtrlLocked = isCtrlLocked
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(viewModel: viewModel, isCtrlLocked: $isCtrlLocked)
    }

    public func makeUIView(context: Context) -> TerminalView {
        var options = TerminalOptions.default
        options.scrollback = 1000
        let fontSize = CGFloat(terminalFontSize)
        let terminalView = AppTerminalView(
            frame: .zero,
            font: UIFont.monospacedSystemFont(ofSize: fontSize, weight: .regular),
            options: options
        )
        terminalView.changeScrollback(1000)
        terminalView.terminalDelegate = context.coordinator
        terminalView.delegate = context.coordinator
        terminalView.backgroundColor = .black
        terminalView.isUserInteractionEnabled = true
        terminalView.allowMouseReporting = false

        // Setup text selection appearance
        terminalView.selectionHandleColor = UIColor.systemBlue

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

        context.coordinator.terminalView = terminalView

        print("[TerminalView \(viewModel.instanceId)] makeUIView created for session '\(viewModel.session.title)'")

        // Feed existing buffered history immediately so screen is never blank,
        // while strictly suppressing automated terminal responses (e.g. DA1, DA2, XTVERSION, size queries)
        let history = viewModel.getHistoryBuffer()
        context.coordinator.feedHistoryBuffer(history, to: terminalView)

        // Subscribe to throttled output stream from Rust Core
        context.coordinator.cancellable = viewModel.outputPublisher
            .receive(on: DispatchQueue.main)
            .sink { [weak terminalView, weak viewModel] data in
                let vmId = viewModel?.instanceId ?? "unknown"
                print("[TerminalView \(vmId)] feeding live data: \(data.count) bytes")
                let bytes = ArraySlice([UInt8](data))
                terminalView?.feed(byteArray: bytes)
            }

        return terminalView
    }

    public func updateUIView(_ uiView: TerminalView, context: Context) {
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
    public final class Coordinator: NSObject, TerminalViewDelegate, UIScrollViewDelegate, UIEditMenuInteractionDelegate {
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
                .sink { [weak terminalView, weak newVM] data in
                    let vmId = newVM?.instanceId ?? "unknown"
                    print("[TerminalView \(vmId)] feeding live data: \(data.count) bytes")
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
                tv.resignFirstResponder()
            }
            editMenuInteraction?.dismissMenu()
        }

        public func scrollViewDidScroll(_ scrollView: UIScrollView) {
            // Guarantee keyboard is hidden when user scrolls
            if let tv = terminalView, tv.isFirstResponder, (scrollView.isTracking || scrollView.isDragging) {
                tv.resignFirstResponder()
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
