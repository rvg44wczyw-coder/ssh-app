import SwiftUI
import SshCoreBridge

/// Settings -> Appearance: feed style, icon, menus, colours, themes.
struct AppearanceSettingsPage: View {
    @AppStorage(AppAccent.storageKey) private var accentRaw: String = AppAccent.blue.rawValue
    @AppStorage(FeedPresentationStyle.storageKey) private var feedStyleRaw: String = FeedPresentationStyle.fallback.rawValue
    @AppStorage(CompactChatMode.storageKey) private var compactChatRaw = CompactChatMode.landscape.rawValue
    @AppStorage("debug_feed_timing") private var showFeedTiming = false
    @State private var schemeRefresh = 0

    @State private var iconChoice: AppIconChoice = .standard
    @State private var editingScheme: TerminalScheme?
    @State private var iconError: String?

    private func setIcon(_ choice: AppIconChoice) {
        Task {
            iconError = await choice.apply()
            iconChoice = AppIconChoice.current
        }
    }

    @ViewBuilder
    private func iconTile(design: AppIconChoice.Design, tint: AppIconChoice.Tint) -> some View {
        let isLight = design == .light
        RoundedRectangle(cornerRadius: 12)
            .fill(isLight ? Color.white : Color.black)
            .frame(width: 52, height: 52)
            .overlay(
                Group {
                    if let preview = design.previewImage {
                        Image(preview).resizable().scaledToFill().colorMultiply(tint.color).clipShape(RoundedRectangle(cornerRadius: 12))
                    } else {
                        RoundedRectangle(cornerRadius: 7).fill(isLight ? Color(white: 0.91) : Color(white: 0.09)).padding(9)
                            .overlay(Image(systemName: "chevron.right").font(.system(size: 14, weight: .bold)).foregroundColor(isLight ? .black : tint.color))
                    }
                }
            )
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(Color.primary.opacity(0.25), lineWidth: 1))
    }

    @State private var open: Set<String> = ["feed"]

    private func group<Content: View>(_ id: String, _ title: LocalizedStringKey, _ symbol: String,
                                      @ViewBuilder content: @escaping () -> Content) -> some View {
        Section {
            DisclosureGroup(isExpanded: Binding(
                get: { open.contains(id) },
                set: { if $0 { open.insert(id) } else { open.remove(id) } }
            )) {
                content()
            } label: {
                SettingsLabel(title, symbol)
            }
        }
    }

    private func hint(_ text: String) -> some View {
        Text(text).font(.caption).foregroundColor(.secondary)
    }

    var body: some View {
        Form {
            group("feed", "Feed & layout", "text.bubble") {
                Picker("Agent feed style", selection: $feedStyleRaw) {
                    ForEach(FeedPresentationStyle.allCases) { style in
                        Label(style.title, systemImage: style.iconName).tag(style.rawValue)
                    }
                }
                .pickerStyle(.navigationLink)
                hint(FeedPresentationStyle.resolved(feedStyleRaw).subtitle + ". Applies to Claude Code and Antigravity tabs.")
                Picker(selection: $compactChatRaw) {
                    ForEach(CompactChatMode.allCases) { Text($0.title).tag($0.rawValue) }
                } label: {
                    Label("Compact chat layout", systemImage: "rectangle.compress.vertical")
                }
                Toggle(isOn: $showFeedTiming) {
                    Label("Show feed timing", systemImage: "stopwatch")
                }
                NavigationLink {
                    SchemePreviewsView()
                } label: {
                    Label("Theme previews", systemImage: "paintpalette")
                }
                NavigationLink {
                    SyntaxSettingsView()
                } label: {
                    Label("Syntax highlighting", systemImage: "chevron.left.forwardslash.chevron.right")
                }
            }
            group("icon", "App icon", "app.badge") {
                LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 4), spacing: 12) {
                    ForEach(AppIconChoice.Design.allCases) { design in
                        let selected = iconChoice.design == design
                        Button {
                            var next = iconChoice
                            next.design = design
                            if !design.supportsTint { next.tint = .white }
                            setIcon(next)
                        } label: {
                            VStack(spacing: 4) {
                                iconTile(design: design, tint: selected ? iconChoice.tint : .white)
                                    .overlay(RoundedRectangle(cornerRadius: 14).stroke(Color.appAccent, lineWidth: selected ? 3 : 0).padding(-3))
                                Text(design.title).font(.caption2).foregroundColor(.secondary)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.vertical, 6)
                if iconChoice.design.supportsTint {
                    LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 8), spacing: 10) {
                        ForEach(AppIconChoice.Tint.allCases) { tint in
                            Button {
                                var next = iconChoice
                                next.tint = tint
                                setIcon(next)
                            } label: {
                                Circle().fill(tint.color).frame(width: 26, height: 26)
                                    .overlay(Circle().stroke(Color.primary.opacity(0.3), lineWidth: 1))
                                    .overlay(Circle().stroke(Color.primary, lineWidth: iconChoice.tint == tint ? 2.5 : 0).padding(-3))
                            }
                            .buttonStyle(.plain)
                        }
                    }
                    .padding(.vertical, 4)
                }
                hint(iconError ?? "iOS asks you to confirm a change. Only icons shipped in the app can be used here; your own picture works for tabs and profiles.")
            }
            group("menus", "Menus", "square.grid.2x2") {
                Text("Quick menu").font(.subheadline.weight(.semibold))
                QuickMenuOrderGrid()
                Text("Session menu").font(.subheadline.weight(.semibold))
                QuickMenuOrderGrid(sessionActions: true)
                hint("Tiles per row and size, drag a tile onto another to reorder, tap to show or hide.")
            }
            group("colors", "Colors", "paintpalette") {
                Text("Accent").font(.subheadline.weight(.semibold))
                LazyVGrid(columns: Array(repeating: GridItem(.flexible()), count: 4), spacing: 14) {
                    ForEach(AppAccent.allCases) { accent in
                        Button { accentRaw = accent.rawValue } label: {
                            VStack(spacing: 4) {
                                Circle().fill(accent.color).frame(width: 34, height: 34)
                                    .overlay(Circle().stroke(Color.primary, lineWidth: accentRaw == accent.rawValue ? 3 : 0).padding(-3))
                                Text(accent.title).font(.caption2).foregroundColor(.secondary)
                            }
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(.vertical, 6)
                ColorPicker(selection: Binding(
                    get: { AppAccent.color(for: accentRaw) },
                    set: { accentRaw = "#" + $0.hexString }
                ), supportsOpacity: false) {
                    HStack {
                        Text("Custom color")
                        if accentRaw.hasPrefix("#") {
                            Text(accentRaw).font(.system(size: 12, design: .monospaced)).foregroundColor(.secondary)
                        }
                    }
                }
                if accentRaw.hasPrefix("#") {
                    Button("Back to Blue") { accentRaw = AppAccent.blue.rawValue }.buttonStyle(.glass)
                }
                Text("Terminal colors by agent").font(.subheadline.weight(.semibold))
                ForEach(SchemeSlot.allCases) { slot in
                    Picker(slot.title, selection: Binding(
                        get: { _ = schemeRefresh; return TerminalSchemes.choice(for: slot) },
                        set: { TerminalSchemes.setChoice($0, for: slot); schemeRefresh += 1 }
                    )) {
                        ForEach(TerminalScheme.all) { Text($0.name).tag($0.id) }
                    }
                }
                hint("A profile can override the scheme. Reopen a tab to see a change.")
                Text("My color schemes").font(.subheadline.weight(.semibold))
                ForEach(TerminalScheme.custom) { s in
                    Button { editingScheme = s } label: {
                        HStack {
                            Text(s.name).foregroundColor(.primary)
                            Spacer()
                            HStack(spacing: 0) {
                                ForEach(1..<7, id: \.self) { Color(hex: s.ansi[$0]).frame(width: 10, height: 14) }
                            }
                            .padding(3).background(Color(hex: s.background)).cornerRadius(4)
                        }
                    }
                }
                .onDelete { offsets in
                    let list = TerminalScheme.custom
                    for i in offsets { TerminalScheme.deleteCustom(id: list[i].id) }
                    schemeRefresh += 1
                }
                Button {
                    editingScheme = TerminalScheme.newCustom(from: .fallback, name: "My scheme")
                } label: { Label("New scheme", systemImage: "plus") }
                    .buttonStyle(.glass)
            }
        }
        .navigationTitle("Appearance")
        .onAppear { iconChoice = AppIconChoice.current }
        .sheet(item: $editingScheme) { s in
            TerminalSchemeEditorView(
                scheme: s,
                onSave: { saved in
                    TerminalScheme.upsertCustom(saved)
                    schemeRefresh += 1
                    editingScheme = nil
                },
                onCancel: { editingScheme = nil }
            )
        }
    }
}
