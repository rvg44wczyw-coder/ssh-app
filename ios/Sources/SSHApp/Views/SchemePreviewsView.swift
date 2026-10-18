import SwiftUI

/// All terminal color schemes with a sample line each, on their own page so Appearance stays short.
struct SchemePreviewsView: View {
    var body: some View {
        List {
            ForEach(TerminalScheme.all) { scheme in
                VStack(alignment: .leading, spacing: 6) {
                    Text(scheme.name).font(.caption).foregroundColor(.secondary)
                    VStack(alignment: .leading, spacing: 4) {
                        Text("$ claude --resume").font(.system(size: 12, design: .monospaced))
                            .foregroundColor(Color(hex: scheme.foreground))
                        HStack(spacing: 0) {
                            ForEach(0..<16, id: \.self) { Color(hex: scheme.ansi[$0]).frame(height: 10) }
                        }
                    }
                    .padding(8)
                    .background(Color(hex: scheme.background))
                    .cornerRadius(8)
                }
            }
        }
        .navigationTitle("Theme previews")
    }
}
