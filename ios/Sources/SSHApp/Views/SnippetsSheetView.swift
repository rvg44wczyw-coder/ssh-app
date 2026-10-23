import SwiftUI
import SshCoreBridge

public struct SnippetsSheetView: View {
    @ObservedObject var viewModel: TerminalSessionViewModel
    public var onClose: (() -> Void)?
    @Environment(\.dismiss) private var dismiss

    @State private var searchQuery: String = ""
    @State private var selectedCategory: String = "All"
    @State private var customSnippets: [SnippetItem] = []
    @State private var showAddSnippet: Bool = false

    // New snippet form state
    @State private var newTitle: String = ""
    @State private var newCommand: String = ""
    @State private var newCategory: String = "Custom"

    // Placeholder filling state
    @State private var activePlaceholderSnippet: SnippetItem? = nil
    @State private var activePlaceholders: [String] = []
    @State private var placeholderValues: [String: String] = [:]
    @State private var executeImmediatelyAfterFill: Bool = false

    private let categories = ["All", "AI Agents", "Git", "Cargo", "System", "History", "Custom"]
    private let customSnippetsStorageKey = "ssh_app.custom_snippets.v1"

    public init(viewModel: TerminalSessionViewModel, onClose: (() -> Void)? = nil) {
        self.viewModel = viewModel
        self.onClose = onClose
    }

    private func closeSheet() {
        onClose?()
        dismiss()
    }

    private var allItems: [SnippetItem] {
        var combined = defaultDeveloperSnippets() + customSnippets
        for (idx, cmd) in viewModel.remoteHistoryCommands.enumerated() {
            combined.append(
                SnippetItem(
                    id: "history-\(idx)-\(cmd.hashValue)",
                    title: "Recent Shell Command",
                    command: cmd,
                    category: "History",
                    tags: ["history", "shell"],
                    useCount: UInt64(max(1, 20 - idx))
                )
            )
        }
        if selectedCategory != "All" {
            combined = combined.filter { $0.category == selectedCategory }
        }
        return combined
    }

    private var filteredResults: [FuzzyMatchResult] {
        fuzzySearchSnippets(query: searchQuery, items: allItems, limit: 60)
    }

    public var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                // Fuzzy Search Bar
                HStack(spacing: 8) {
                    Image(systemName: "magnifyingglass")
                        .foregroundColor(.secondary)
                    TextField("Fuzzy search commands & history (e.g. 'ctest', 'gstat')...", text: $searchQuery)
                        .textFieldStyle(PlainTextFieldStyle())
                        .font(.system(size: 14, design: .monospaced))
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        #endif

                    if !searchQuery.isEmpty {
                        Button(action: { searchQuery = "" }) {
                            Image(systemName: "xmark.circle.fill")
                                .foregroundColor(.secondary)
                        }
                    }
                }
                .padding(10)
                .background(Color.appSecondaryBackground)
                .cornerRadius(10)
                .padding(.horizontal, 12)
                .padding(.top, 10)

                // Category Chips
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(categories, id: \.self) { category in
                            Button(action: {
                                selectedCategory = category
                                if category == "History" && viewModel.remoteHistoryCommands.isEmpty {
                                    viewModel.fetchRemoteHistory()
                                }
                            }) {
                                Text(category)
                                    .font(.system(size: 12, weight: selectedCategory == category ? .bold : .medium))
                                    .padding(.horizontal, 12)
                                    .padding(.vertical, 6)
                                    .background(selectedCategory == category ? Color.appAccent : Color.appSecondaryBackground)
                                    .foregroundColor(selectedCategory == category ? .white : .primary)
                                    .cornerRadius(16)
                            }
                        }
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                }

                Divider()

                // New Snippet Form (if toggled)
                if showAddSnippet {
                    VStack(alignment: .leading, spacing: 8) {
                        Text("New Custom Snippet")
                            .font(.system(size: 12, weight: .bold))
                            .foregroundColor(.secondary)

                        TextField("Title (e.g. Deploy Staging)", text: $newTitle)
                            .textFieldStyle(RoundedBorderTextFieldStyle())
                            .font(.system(size: 13))

                        TextField("Command (use {{var}} for placeholders)", text: $newCommand)
                            .textFieldStyle(RoundedBorderTextFieldStyle())
                            .font(.system(size: 13, design: .monospaced))
                            .autocorrectionDisabled()
                            #if os(iOS)
                            .textInputAutocapitalization(.never)
                            #endif

                        HStack {
                            Button("Cancel") {
                                showAddSnippet = false
                            }
                            .font(.system(size: 12))
                            .foregroundColor(.secondary)

                            Spacer()

                            Button("Save Snippet") {
                                saveNewSnippet()
                            }
                            .font(.system(size: 12, weight: .bold))
                            .disabled(newTitle.trimmingCharacters(in: .whitespaces).isEmpty || newCommand.trimmingCharacters(in: .whitespaces).isEmpty)
                        }
                    }
                    .padding(12)
                    .background(Color.appSecondaryBackground)
                    Divider()
                }

                // Placeholder Input Banner (if a parameterized snippet is selected)
                if let snippet = activePlaceholderSnippet {
                    VStack(alignment: .leading, spacing: 8) {
                        HStack {
                            Text("Fill Parameters for '\(snippet.title)'")
                                .font(.system(size: 12, weight: .bold))
                            Spacer()
                            Button("Cancel") {
                                activePlaceholderSnippet = nil
                            }
                            .font(.system(size: 11))
                        }

                        ForEach(activePlaceholders, id: \.self) { param in
                            HStack {
                                Text("{{\(param)}}:")
                                    .font(.system(size: 12, weight: .semibold, design: .monospaced))
                                    .frame(width: 90, alignment: .leading)
                                TextField("Value for \(param)", text: Binding(
                                    get: { placeholderValues[param] ?? "" },
                                    set: { placeholderValues[param] = $0 }
                                ))
                                .textFieldStyle(RoundedBorderTextFieldStyle())
                                .font(.system(size: 12, design: .monospaced))
                                .autocorrectionDisabled()
                                #if os(iOS)
                                .textInputAutocapitalization(.never)
                                #endif
                            }
                        }

                        Button(action: {
                            commitPlaceholderCommand(snippet: snippet)
                        }) {
                            Text(executeImmediatelyAfterFill ? "Run Parameterized Command" : "Insert Parameterized Command")
                                .font(.system(size: 13, weight: .bold))
                                .frame(maxWidth: .infinity)
                                .padding(.vertical, 8)
                                .background(Color.appAccent)
                                .foregroundColor(.white)
                                .cornerRadius(8)
                        }
                    }
                    .padding(12)
                    .background(Color.appAccent.opacity(0.1))
                    Divider()
                }

                // Results List
                if filteredResults.isEmpty {
                    VStack(spacing: 10) {
                        Image(systemName: "sparkle.magnifyingglass")
                            .font(.system(size: 36))
                            .foregroundColor(.secondary)
                        Text("No Matching Snippets")
                            .font(.headline)
                        Text("Try another fuzzy query or sync remote shell history.")
                            .font(.caption)
                            .foregroundColor(.secondary)
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    List {
                        ForEach(filteredResults, id: \.item.id) { match in
                            SnippetRowView(
                                match: match,
                                onInsert: { handleAction(snippet: match.item, execute: false) },
                                onRun: { handleAction(snippet: match.item, execute: true) }
                            )
                        }
                    }
                    .listStyle(.plain)
                }
            }
            .navigationTitle("Smart Snippets & History")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Close") {
                        closeSheet()
                    }
                }
                ToolbarItemGroup(placement: .topBarTrailing) {
                    Button(action: { viewModel.fetchRemoteHistory() }) {
                        if viewModel.isHistoryLoading {
                            ProgressView()
                                .scaleEffect(0.7)
                        } else {
                            Image(systemName: "clock.arrow.circlepath")
                        }
                    }
                    .accessibilityLabel("Sync Remote History")

                    Button(action: {
                        withAnimation { showAddSnippet.toggle() }
                    }) {
                        Image(systemName: "plus")
                    }
                    .accessibilityLabel("Add Custom Snippet")
                }
            }
            .onAppear {
                loadCustomSnippets()
                if viewModel.remoteHistoryCommands.isEmpty {
                    viewModel.fetchRemoteHistory()
                }
            }
        }
        .interactiveDismissDisabled()
    }

    private func handleAction(snippet: SnippetItem, execute: Bool) {
        let placeholders = extractSnippetPlaceholders(command: snippet.command)
        if !placeholders.isEmpty {
            activePlaceholderSnippet = snippet
            activePlaceholders = placeholders
            placeholderValues = [:]
            executeImmediatelyAfterFill = execute
            return
        }

        if execute {
            viewModel.runShellCommand(snippet.command)
        } else {
            viewModel.insertShellCommand(snippet.command)
        }
        closeSheet()
    }

    private func commitPlaceholderCommand(snippet: SnippetItem) {
        var rendered = snippet.command
        for param in activePlaceholders {
            let val = placeholderValues[param] ?? ""
            rendered = rendered.replacingOccurrences(of: "{{\(param)}}", with: val)
        }
        activePlaceholderSnippet = nil
        if executeImmediatelyAfterFill {
            viewModel.runShellCommand(rendered)
        } else {
            viewModel.insertShellCommand(rendered)
        }
        closeSheet()
    }

    private func loadCustomSnippets() {
        guard let data = UserDefaults.standard.data(forKey: customSnippetsStorageKey) else { return }
        if let decoded = try? JSONDecoder().decode([CodableSnippet].self, from: data) {
            customSnippets = decoded.map { $0.toSnippetItem() }
        }
    }

    private func saveNewSnippet() {
        let item = SnippetItem(
            id: "custom-\(UUID().uuidString.prefix(8))",
            title: newTitle.trimmingCharacters(in: .whitespaces),
            command: newCommand.trimmingCharacters(in: .whitespaces),
            category: newCategory,
            tags: ["custom"],
            useCount: 5
        )
        customSnippets.insert(item, at: 0)
        let codable = customSnippets.map { CodableSnippet(from: $0) }
        if let encoded = try? JSONEncoder().encode(codable) {
            UserDefaults.standard.set(encoded, forKey: customSnippetsStorageKey)
            Task { @MainActor in VaultSync.shared.sync() }
        }
        newTitle = ""
        newCommand = ""
        showAddSnippet = false
    }
}

private struct CodableSnippet: Codable {
    let id: String
    let title: String
    let command: String
    let category: String
    let tags: [String]
    let useCount: UInt64

    init(from item: SnippetItem) {
        self.id = item.id
        self.title = item.title
        self.command = item.command
        self.category = item.category
        self.tags = item.tags
        self.useCount = item.useCount
    }

    func toSnippetItem() -> SnippetItem {
        SnippetItem(
            id: id,
            title: title,
            command: command,
            category: category,
            tags: tags,
            useCount: useCount
        )
    }
}

private struct SnippetRowView: View {
    let match: FuzzyMatchResult
    let onInsert: () -> Void
    let onRun: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(match.item.title)
                    .font(.system(size: 13, weight: .semibold))
                Spacer()
                Text(match.item.category)
                    .font(.system(size: 10, weight: .bold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(Color.appAccent.opacity(0.15))
                    .foregroundColor(.appAccent)
                    .cornerRadius(4)
            }

            Text(match.item.command)
                .font(.system(size: 12, design: .monospaced))
                .foregroundColor(.primary)
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.black.opacity(0.25))
                .cornerRadius(6)

            HStack(spacing: 10) {
                Spacer()
                Button(action: onInsert) {
                    Label("Insert", systemImage: "text.insert")
                        .font(.system(size: 11, weight: .semibold))
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(Color.secondary.opacity(0.15))
                        .foregroundColor(.primary)
                        .cornerRadius(6)
                }
                .buttonStyle(.plain)

                Button(action: onRun) {
                    Label("Run", systemImage: "play.fill")
                        .font(.system(size: 11, weight: .bold))
                        .padding(.horizontal, 10)
                        .padding(.vertical, 5)
                        .background(Color.appAccent)
                        .foregroundColor(.white)
                        .cornerRadius(6)
                }
                .buttonStyle(.plain)
            }
        }
        .padding(.vertical, 4)
    }
}
