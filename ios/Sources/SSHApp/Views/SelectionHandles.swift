import SwiftTerm
import UIKit

/// Cell size of the terminal grid in the view's content coordinates.
@MainActor
func terminalCellSize(_ tv: TerminalView) -> CGSize {
    let term = tv.getTerminal()
    return CGSize(
        width: tv.bounds.width / CGFloat(max(term.cols, 1)),
        height: tv.bounds.height / CGFloat(max(term.rows, 1)))
}

/// A draggable end of the terminal selection (a dot on a short stem). The hit area is larger than
/// the dot so a finger can grab it.
final class SelectionKnob: UIView, UIGestureRecognizerDelegate {
    let isStart: Bool

    init(isStart: Bool, color: UIColor) {
        self.isStart = isStart
        super.init(frame: CGRect(x: 0, y: 0, width: 44, height: 44))
        backgroundColor = .clear
        tintColor = color
    }

    required init?(coder: NSCoder) { nil }

    override func draw(_ rect: CGRect) {
        tintColor.setFill()
        tintColor.setStroke()
        let c = CGPoint(x: bounds.midX, y: bounds.midY)
        UIBezierPath(ovalIn: CGRect(x: c.x - 7, y: c.y - 7, width: 14, height: 14)).fill()
        // Stem towards the text: down from a start dot, up from an end dot.
        let stem = UIBezierPath()
        stem.move(to: CGPoint(x: c.x, y: c.y + (isStart ? 7 : -7)))
        stem.addLine(to: CGPoint(x: c.x, y: c.y + (isStart ? 15 : -15)))
        stem.lineWidth = 2
        stem.stroke()
    }

    /// Other pans (the terminal's scroll pan) wait for a knob drag to fail first.
    func gestureRecognizer(_ g: UIGestureRecognizer, shouldBeRequiredToFailBy other: UIGestureRecognizer) -> Bool {
        other is UIPanGestureRecognizer
    }
}

/// The two handles of the current selection. They live in the terminal's content space, so they
/// scroll with the text; `refresh()` places them (or hides them when nothing is selected).
@MainActor
final class SelectionHandles: NSObject {
    private weak var terminalView: TerminalView?
    private let startKnob: SelectionKnob
    private let endKnob: SelectionKnob
    private let onFinished: (CGPoint) -> Void
    /// Distance between a dot's centre and the text edge it points at.
    private let reach: CGFloat = 15

    init(terminalView: TerminalView, color: UIColor, onFinished: @escaping (CGPoint) -> Void) {
        self.terminalView = terminalView
        self.startKnob = SelectionKnob(isStart: true, color: color)
        self.endKnob = SelectionKnob(isStart: false, color: color)
        self.onFinished = onFinished
        super.init()
        for knob in [startKnob, endKnob] {
            knob.isHidden = true
            let pan = UIPanGestureRecognizer(target: self, action: #selector(drag(_:)))
            pan.delegate = knob
            knob.addGestureRecognizer(pan)
            terminalView.addSubview(knob)
        }
    }

    func refresh() {
        guard let tv = terminalView, let sel = tv.selection, tv.hasActiveSelection else {
            startKnob.isHidden = true
            endKnob.isHidden = true
            return
        }
        let cell = terminalCellSize(tv)
        startKnob.center = CGPoint(
            x: CGFloat(sel.start.col) * cell.width, y: CGFloat(sel.start.row) * cell.height - reach)
        endKnob.center = CGPoint(
            x: CGFloat(sel.end.col) * cell.width, y: CGFloat(sel.end.row + 1) * cell.height + reach)
        startKnob.isHidden = false
        endKnob.isHidden = false
        tv.bringSubviewToFront(startKnob)
        tv.bringSubviewToFront(endKnob)
        startKnob.setNeedsDisplay()
        endKnob.setNeedsDisplay()
    }

    @objc private func drag(_ gesture: UIPanGestureRecognizer) {
        guard let tv = terminalView, let sel = tv.selection,
            let knob = gesture.view as? SelectionKnob
        else { return }
        let finger = gesture.location(in: tv)
        switch gesture.state {
        case .began:
            // The end that is not dragged stays where it is.
            sel.pivot = knob.isStart ? sel.end : sel.start
        case .changed:
            let cell = terminalCellSize(tv)
            let cols = tv.getTerminal().cols
            // The dot sits outside the text; aim at the cell its stem points to.
            let aimY = finger.y + (knob.isStart ? reach : -reach) + (knob.isStart ? cell.height : -cell.height) / 2
            let col = min(max(Int((finger.x / cell.width).rounded()), 0), cols)
            let row = max(Int(aimY / cell.height), 0)
            sel.pivotExtend(bufferPosition: Position(col: col, row: row))
            tv.setNeedsDisplay()
            refresh()
        case .ended, .cancelled:
            onFinished(CGPoint(x: knob.center.x, y: knob.center.y))
        default:
            break
        }
    }
}
