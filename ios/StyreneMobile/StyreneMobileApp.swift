import SwiftUI

@main
struct StyreneMobileApp: App {
    @StateObject private var node = StyreneNodeModel()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(node)
        }
    }
}

private struct ContentView: View {
    private enum Field: Hashable {
        case displayName
        case hubAddress
        case directHash
    }

    @EnvironmentObject private var node: StyreneNodeModel
    @AppStorage("hubAddress") private var hubAddress = ""
    @AppStorage("displayName") private var displayName = "Styrene iPhone"
    @State private var directHash = ""
    @FocusState private var focusedField: Field?

    var body: some View {
        NavigationStack {
            ZStack {
                meshBackground

                if let peerHash = node.selectedPeerHash {
                    ConversationView(peerHash: peerHash)
                } else {
                    dashboard
                }
            }
            .toolbarColorScheme(.dark, for: .navigationBar)
            .toolbar {
                ToolbarItemGroup(placement: .keyboard) {
                    Spacer()
                    Button("Done") {
                        focusedField = nil
                    }
                }
            }
        }
    }

    private var dashboard: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                header
                connectionCard
                if node.isRunning {
                    meshActions
                    conversationCard
                    peerCard
                }
                identityCard
            }
            .padding(20)
        }
        .scrollDismissesKeyboard(.interactively)
        .refreshable {
            node.refresh()
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("STYRENE MOBILE")
                .font(.caption.weight(.bold))
                .tracking(2.4)
                .foregroundStyle(.teal)
            Text("Mesh link")
                .font(.system(size: 38, weight: .semibold, design: .rounded))
                .foregroundStyle(.white)
            Text("A private Reticulum node running directly on this phone.")
                .foregroundStyle(.white.opacity(0.65))
        }
    }

    private var connectionCard: some View {
        VStack(alignment: .leading, spacing: 16) {
            Label(node.phase.label, systemImage: node.phase.symbol)
                .font(.headline)
                .foregroundStyle(node.phase.tint)

            TextField(
                "",
                text: $displayName,
                prompt: Text("Display name").foregroundStyle(.white.opacity(0.4))
            )
            .focused($focusedField, equals: .displayName)
            .submitLabel(.next)
            .onSubmit { focusedField = .hubAddress }
            .textInputAutocapitalization(.words)
            .disabled(node.isRunning)
            .fieldStyle()

            TextField(
                "",
                text: $hubAddress,
                prompt: Text("Hub address, for example 192.168.0.202:4242")
                    .foregroundStyle(.white.opacity(0.4))
            )
            .focused($focusedField, equals: .hubAddress)
            .submitLabel(.done)
            .onSubmit { focusedField = nil }
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .keyboardType(.numbersAndPunctuation)
            .disabled(node.isRunning)
            .fieldStyle()

            if hubAddress.isEmpty && !node.isRunning {
                Button("Use LAN hub 192.168.0.202:4242") {
                    hubAddress = "192.168.0.202:4242"
                }
                .font(.footnote.weight(.medium))
                .foregroundStyle(.teal)
            }

            Button(node.isRunning ? "Stop node" : "Start node") {
                focusedField = nil
                if node.isRunning {
                    node.shutdown()
                } else {
                    node.boot(hubAddress: hubAddress, displayName: displayName)
                }
            }
            .buttonStyle(.borderedProminent)
            .tint(node.isRunning ? .red : .teal)
            .disabled(node.isBusy)

            feedback
        }
        .cardStyle()
    }

    private var meshActions: some View {
        HStack(spacing: 12) {
            actionButton("Announce", symbol: "dot.radiowaves.left.and.right") {
                node.announce()
            }
            actionButton("Refresh", symbol: "arrow.clockwise") {
                node.refresh()
            }
            actionButton("Poll hub", symbol: "tray.and.arrow.down") {
                node.pollHub()
            }
        }
        .disabled(node.isRefreshing)
    }

    private var conversationCard: some View {
        VStack(alignment: .leading, spacing: 14) {
            sectionHeader("CONVERSATIONS", count: node.conversations.count)

            if node.conversations.isEmpty {
                emptyState("No messages yet", detail: "Announce to the mesh or open a peer directly.")
            } else {
                ForEach(node.conversations, id: \.peerHash) { conversation in
                    Button {
                        node.openConversation(peerHash: conversation.peerHash)
                    } label: {
                        HStack(spacing: 12) {
                            peerGlyph(conversation.peerHash)
                            VStack(alignment: .leading, spacing: 4) {
                                Text(node.displayName(for: conversation.peerHash))
                                    .font(.body.weight(.semibold))
                                    .foregroundStyle(.white)
                                Text("\(conversation.messageCount) messages")
                                    .font(.caption)
                                    .foregroundStyle(.white.opacity(0.5))
                            }
                            Spacer()
                            if conversation.unreadCount > 0 {
                                Text(String(conversation.unreadCount))
                                    .font(.caption.bold())
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 4)
                                    .background(.teal, in: Capsule())
                                    .foregroundStyle(.black)
                            }
                            Image(systemName: "chevron.right")
                                .font(.caption.bold())
                                .foregroundStyle(.white.opacity(0.35))
                        }
                    }
                    .buttonStyle(.plain)
                }
            }

            Divider().overlay(.white.opacity(0.12))
            TextField(
                "",
                text: $directHash,
                prompt: Text("Peer delivery hash").foregroundStyle(.white.opacity(0.4))
            )
            .focused($focusedField, equals: .directHash)
            .submitLabel(.go)
            .onSubmit { openDirectConversation() }
            .textInputAutocapitalization(.never)
            .autocorrectionDisabled()
            .font(.system(.body, design: .monospaced))
            .fieldStyle()

            Button("Open direct conversation") {
                openDirectConversation()
            }
            .buttonStyle(.bordered)
            .tint(.teal)
            .disabled(directHash.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .cardStyle()
    }

    private var peerCard: some View {
        VStack(alignment: .leading, spacing: 14) {
            sectionHeader("PEERS", count: node.peers.count)
            if node.peers.isEmpty {
                emptyState("No peers discovered", detail: "Send an announce, then refresh after nearby nodes respond.")
            } else {
                ForEach(node.peers, id: \.destinationHash) { peer in
                    Button {
                        node.openConversation(peerHash: peer.destinationHash)
                    } label: {
                        HStack(spacing: 12) {
                            peerGlyph(peer.destinationHash)
                            VStack(alignment: .leading, spacing: 3) {
                                Text(node.displayName(for: peer.destinationHash))
                                    .font(.body.weight(.semibold))
                                    .foregroundStyle(.white)
                                Text(peer.destinationHash)
                                    .font(.caption.monospaced())
                                    .lineLimit(1)
                                    .foregroundStyle(.white.opacity(0.45))
                            }
                            Spacer()
                            Text(peer.status.uppercased())
                                .font(.caption2.weight(.bold))
                                .foregroundStyle(.teal)
                        }
                    }
                    .buttonStyle(.plain)
                }
            }
        }
        .cardStyle()
    }

    private var identityCard: some View {
        VStack(alignment: .leading, spacing: 12) {
            sectionHeader("NODE IDENTITY")
            Text(node.identityHash.isEmpty ? "Start the node to create an identity." : node.identityHash)
                .font(.system(.body, design: .monospaced))
                .foregroundStyle(.white)
                .textSelection(.enabled)

            if let status = node.status {
                Divider().overlay(.white.opacity(0.15))
                HStack {
                    metric("Peers", value: String(status.peerCount))
                    metric("Links", value: String(status.linkCount))
                    metric("Uptime", value: "\(status.uptimeSecs)s")
                }
            }
        }
        .cardStyle()
    }

    @ViewBuilder
    private var feedback: some View {
        if let error = node.errorMessage {
            Text(error)
                .font(.footnote)
                .foregroundStyle(.red.opacity(0.9))
                .textSelection(.enabled)
        } else if let notice = node.notice {
            Text(notice)
                .font(.footnote)
                .foregroundStyle(.teal)
        }
    }

    private var meshBackground: some View {
        LinearGradient(
            colors: [Color(red: 0.03, green: 0.09, blue: 0.11), .black],
            startPoint: .topLeading,
            endPoint: .bottomTrailing
        )
        .ignoresSafeArea()
    }

    private func openDirectConversation() {
        let hash = directHash.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !hash.isEmpty else { return }
        focusedField = nil
        node.openConversation(peerHash: hash)
    }

    private func actionButton(_ title: String, symbol: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            VStack(spacing: 8) {
                Image(systemName: symbol).font(.title3)
                Text(title).font(.caption.weight(.semibold))
            }
            .foregroundStyle(.white)
            .frame(maxWidth: .infinity)
            .padding(.vertical, 13)
            .background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 14))
        }
        .buttonStyle(.plain)
    }

    private func sectionHeader(_ title: String, count: Int? = nil) -> some View {
        HStack {
            Text(title)
                .font(.caption.weight(.bold))
                .tracking(1.8)
                .foregroundStyle(.white.opacity(0.5))
            Spacer()
            if let count {
                Text(String(count))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.teal)
            }
        }
    }

    private func emptyState(_ title: String, detail: String) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title).font(.body.weight(.medium)).foregroundStyle(.white)
            Text(detail).font(.footnote).foregroundStyle(.white.opacity(0.5))
        }
    }

    private func peerGlyph(_ hash: String) -> some View {
        ZStack {
            RoundedRectangle(cornerRadius: 11)
                .fill(.teal.opacity(0.16))
            Text(String(hash.prefix(2)).uppercased())
                .font(.caption.monospaced().bold())
                .foregroundStyle(.teal)
        }
        .frame(width: 42, height: 42)
    }

    private func metric(_ title: String, value: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(value).font(.title3.monospacedDigit().weight(.semibold))
            Text(title).font(.caption).foregroundStyle(.white.opacity(0.5))
        }
        .foregroundStyle(.white)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct ConversationView: View {
    @EnvironmentObject private var node: StyreneNodeModel
    let peerHash: String
    @State private var draft = ""
    @FocusState private var composerFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                LazyVStack(spacing: 10) {
                    if node.messages.isEmpty {
                        VStack(spacing: 10) {
                            Image(systemName: "bubble.left.and.bubble.right")
                                .font(.largeTitle)
                                .foregroundStyle(.teal)
                            Text("No messages with this peer")
                                .foregroundStyle(.white.opacity(0.65))
                        }
                        .padding(.top, 80)
                    }

                    ForEach(Array(node.messages.reversed()), id: \.id) { message in
                        messageBubble(message)
                    }
                }
                .padding(16)
            }
            .scrollDismissesKeyboard(.interactively)

            HStack(alignment: .bottom, spacing: 10) {
                TextField("Message", text: $draft, axis: .vertical)
                    .focused($composerFocused)
                    .lineLimit(1...5)
                    .submitLabel(.send)
                    .onSubmit { send() }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 11)
                    .background(.white.opacity(0.1), in: RoundedRectangle(cornerRadius: 18))
                    .foregroundStyle(.white)

                Button(action: send) {
                    Image(systemName: "arrow.up")
                        .font(.headline.bold())
                        .frame(width: 42, height: 42)
                        .background(.teal, in: Circle())
                        .foregroundStyle(.black)
                }
                .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .padding(12)
            .background(.ultraThinMaterial)
        }
        .navigationTitle(node.displayName(for: peerHash))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarLeading) {
                Button {
                    node.closeConversation()
                } label: {
                    Label("Mesh", systemImage: "chevron.left")
                }
                .tint(.teal)
            }
            ToolbarItem(placement: .topBarTrailing) {
                Button {
                    node.openConversation(peerHash: peerHash)
                } label: {
                    Image(systemName: "arrow.clockwise")
                }
                .tint(.teal)
            }
            ToolbarItemGroup(placement: .keyboard) {
                Spacer()
                Button("Done") { composerFocused = false }
            }
        }
    }

    private func messageBubble(_ message: MessageEntry) -> some View {
        HStack {
            if message.isOutgoing { Spacer(minLength: 52) }
            VStack(alignment: message.isOutgoing ? .trailing : .leading, spacing: 5) {
                Text(message.content)
                    .foregroundStyle(message.isOutgoing ? .black : .white)
                Text(timestamp(message.timestamp))
                    .font(.caption2.monospacedDigit())
                    .foregroundStyle(message.isOutgoing ? .black.opacity(0.55) : .white.opacity(0.45))
            }
            .padding(.horizontal, 13)
            .padding(.vertical, 10)
            .background(
                message.isOutgoing ? AnyShapeStyle(Color.teal) : AnyShapeStyle(Color.white.opacity(0.1)),
                in: RoundedRectangle(cornerRadius: 16)
            )
            if !message.isOutgoing { Spacer(minLength: 52) }
        }
    }

    private func send() {
        let content = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !content.isEmpty else { return }
        draft = ""
        node.send(content)
    }

    private func timestamp(_ value: Int64) -> String {
        Date(timeIntervalSince1970: TimeInterval(value)).formatted(date: .omitted, time: .shortened)
    }
}

private extension View {
    func fieldStyle() -> some View {
        padding(12)
            .background(.white.opacity(0.08), in: RoundedRectangle(cornerRadius: 10))
            .foregroundStyle(.white)
    }

    func cardStyle() -> some View {
        frame(maxWidth: .infinity, alignment: .leading)
            .padding(18)
            .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 18))
            .overlay {
                RoundedRectangle(cornerRadius: 18)
                    .stroke(.white.opacity(0.1), lineWidth: 1)
            }
    }
}
