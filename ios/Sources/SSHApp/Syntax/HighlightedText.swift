import SwiftUI

extension HighlightedLine {
    /// The line as styled text; colours only, the caller sets font and size.
    func attributed() -> AttributedString {
        var out = AttributedString()
        for run in runs {
            var piece = AttributedString(run.text)
            if let c = run.color {
                piece.foregroundColor = Color(red: c.red, green: c.green, blue: c.blue)
            }
            out.append(piece)
        }
        return out
    }
}
