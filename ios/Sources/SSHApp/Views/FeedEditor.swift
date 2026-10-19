import SwiftUI

#if canImport(UIKit)
import UIKit

/// The feed's message field: a native multi-line text view (cursor by finger, selection handles,
/// magnifier, system Cut / Copy / Paste, undo by shake or three-finger swipe, autocorrect, Look
/// Up). Grows with its text up to `maxHeight`, then scrolls.
struct FeedEditor: UIViewRepresentable {
    @Binding var text: String
    let placeholder: String
    let maxHeight: CGFloat
    var focus: FocusState<Bool>.Binding

    func makeCoordinator() -> Coordinator { Coordinator(self) }

    func makeUIView(context: Context) -> UITextView {
        let tv = UITextView()
        tv.delegate = context.coordinator
        tv.font = .preferredFont(forTextStyle: .body)
        tv.adjustsFontForContentSizeCategory = true
        tv.backgroundColor = .clear
        tv.textContainerInset = UIEdgeInsets(top: 8, left: 6, bottom: 8, right: 6)
        tv.isScrollEnabled = true
        tv.keyboardDismissMode = .interactive
        tv.allowsEditingTextAttributes = false
        tv.setContentHuggingPriority(.defaultLow, for: .horizontal)
        tv.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let label = UILabel()
        label.text = placeholder
        label.font = tv.font
        label.textColor = .placeholderText
        label.translatesAutoresizingMaskIntoConstraints = false
        label.tag = 7
        tv.addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: tv.leadingAnchor, constant: 11),
            label.topAnchor.constraint(equalTo: tv.topAnchor, constant: 8),
        ])
        return tv
    }

    func updateUIView(_ tv: UITextView, context: Context) {
        context.coordinator.parent = self
        if tv.text != text { tv.text = text }
        tv.viewWithTag(7)?.isHidden = !text.isEmpty
        if focus.wrappedValue != tv.isFirstResponder {
            if focus.wrappedValue { tv.becomeFirstResponder() } else { tv.resignFirstResponder() }
        }
    }

    func sizeThatFits(_ proposal: ProposedViewSize, uiView: UITextView, context: Context) -> CGSize? {
        let width = proposal.width ?? 300
        let fit = uiView.sizeThatFits(CGSize(width: width, height: .greatestFiniteMagnitude))
        return CGSize(width: width, height: min(max(fit.height, 40), maxHeight))
    }

    final class Coordinator: NSObject, UITextViewDelegate {
        var parent: FeedEditor
        init(_ parent: FeedEditor) { self.parent = parent }

        func textViewDidChange(_ textView: UITextView) {
            parent.text = textView.text
            textView.viewWithTag(7)?.isHidden = !textView.text.isEmpty
            // Keep the caret in view while the field is at its maximum height.
            if let range = textView.selectedTextRange {
                textView.scrollRectToVisible(textView.caretRect(for: range.end), animated: false)
            }
        }

        func textViewDidBeginEditing(_ textView: UITextView) { parent.focus.wrappedValue = true }
        func textViewDidEndEditing(_ textView: UITextView) { parent.focus.wrappedValue = false }
    }
}
#endif
