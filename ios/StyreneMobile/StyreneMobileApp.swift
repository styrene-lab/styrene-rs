import SwiftUI
import UIKit

@main
struct StyreneMobileApp: App {
    @StateObject private var node = StyreneNodeModel()
    @StateObject private var rnode = RNodeBluetoothController()
    private let integrationProfile: MobileIntegrationLaunchProfile?

    init() {
        do {
            integrationProfile = try MobileIntegrationLaunchProfile.parse(arguments: ProcessInfo.processInfo.arguments)
        } catch {
            fatalError("Invalid Styrene integration launch profile: \(error.localizedDescription)")
        }
    }

    var body: some Scene {
        WindowGroup {
            MobileShell()
                .environmentObject(node)
                .environmentObject(rnode)
                .preferredColorScheme(.dark)
                .task {
                    if let integrationProfile {
                        node.boot(profile: integrationProfile)
                    } else {
                        let defaults = UserDefaults.standard
                        node.boot(
                            hubAddress: defaults.string(forKey: "hubAddress") ?? "",
                            displayName: defaults.string(forKey: "displayName") ?? "Styrene iPhone"
                        )
                    }
                }
                .onChange(of: node.identityHash, initial: true) { _, identityHash in
                    rnode.attach(channel: identityHash.isEmpty ? nil : node.rnodePacketChannel())
                }
        }
    }
}

private enum MobileTab: Hashable {
    case messages
    case people
    case network
    case more
}

private struct MobileShell: View {
    @State private var tab = MobileTab.messages

    var body: some View {
        TabView(selection: $tab) {
            MessagesScreen(onNewMessage: { tab = .people })
                .tag(MobileTab.messages)
                .tabItem {
                    Label("Messages", systemImage: "bubble.left.and.bubble.right")
                        .accessibilityIdentifier("tab.messages")
                }

            PeopleScreen()
                .tag(MobileTab.people)
                .tabItem {
                    Label("People", systemImage: "person.2")
                        .accessibilityIdentifier("tab.people")
                }

            NetworkScreen()
                .tag(MobileTab.network)
                .tabItem {
                    Label("Network", systemImage: "point.3.connected.trianglepath.dotted")
                        .accessibilityIdentifier("tab.network")
                }

            MoreScreen()
                .tag(MobileTab.more)
                .tabItem {
                    Label("More", systemImage: "ellipsis")
                        .accessibilityIdentifier("tab.more")
                }
        }
        .tint(.signal)
    }
}

private struct ConversationRoute: Identifiable, Hashable {
    let id: String
    let name: String
    let isPreview: Bool
}

private struct ConversationCard: Identifiable {
    let id: String
    let name: String
    let preview: String
    let timestamp: String
    let unread: Int
    let isPreview: Bool
}

private struct PersonCard: Identifiable {
    let id: String
    let name: String
    let detail: String
    let saved: Bool
    let isPreview: Bool
}

private struct PreviewMessage: Identifiable {
    let id: String
    let content: String
    let outgoing: Bool
    let timestamp: String
    let state: String
    let route: String?
}

private struct MessagesScreen: View {
    @EnvironmentObject private var node: StyreneNodeModel
    let onNewMessage: () -> Void
    @State private var route: ConversationRoute?
    @State private var showIdentity = false

    private var conversations: [ConversationCard] {
        if node.conversations.isEmpty {
            return PreviewData.conversations
        }
        return node.conversations.map {
            ConversationCard(
                id: $0.peerHash,
                name: node.displayName(for: $0.peerHash),
                preview: "\($0.messageCount) messages",
                timestamp: activityTimestamp($0.lastActivity),
                unread: Int($0.unreadCount),
                isPreview: false
            )
        }
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(spacing: 12) {
                    MessagingHeader(onIdentity: { showIdentity = true }, onCompose: onNewMessage)
                    InboxStatusRow(
                        peerCount: Int(node.status?.peerCount ?? 0),
                        unreadCount: conversations.reduce(0) { $0 + $1.unread },
                        active: node.status?.transportActive == true,
                        status: node.phase.label,
                        canRefresh: node.isRunning,
                        onRefresh: node.refresh
                    )

                    HStack {
                        SectionLabel("CONVERSATIONS")
                        Spacer()
                        if node.conversations.isEmpty { PreviewBadge() }
                    }

                    ForEach(conversations) { conversation in
                        Button {
                            route = ConversationRoute(
                                id: conversation.id,
                                name: conversation.name,
                                isPreview: conversation.isPreview
                            )
                        } label: {
                            ConversationRow(conversation: conversation)
                        }
                        .buttonStyle(.plain)
                        .accessibilityIdentifier("messages.conversation.\(conversation.id)")
                    }

                    if let notice = node.notice {
                        InlineNotice(text: notice)
                    }
                    if let error = node.errorMessage {
                        InlineNotice(text: error, isError: true)
                    }
                }
                .padding(16)
            }
            .background(Color.ink)
            .toolbar(.hidden, for: .navigationBar)
            .navigationDestination(item: $route) { conversation in
                ConversationScreen(conversation: conversation)
            }
            .refreshable { node.refresh() }
            .sheet(isPresented: $showIdentity) {
                NavigationStack { MoreDetail(destination: .identity) }
            }
        }
    }
}

private struct ConversationRow: View {
    let conversation: ConversationCard

    var body: some View {
        HStack(spacing: 14) {
            IdentityGlyph(name: conversation.name)
            VStack(alignment: .leading, spacing: 5) {
                HStack(spacing: 8) {
                    Text(conversation.name)
                        .font(.body.weight(.semibold))
                        .foregroundStyle(Color.paper)
                    if conversation.isPreview { PreviewBadge() }
                }
                Text(conversation.preview)
                    .font(.subheadline)
                    .foregroundStyle(Color.mist)
                    .lineLimit(1)
            }
            Spacer()
            VStack(alignment: .trailing, spacing: 8) {
                Text(conversation.timestamp)
                    .font(.caption2)
                    .foregroundStyle(Color.mist)
                if conversation.unread > 0 {
                    Text(String(conversation.unread))
                        .font(.caption2.bold())
                        .foregroundStyle(Color.ink)
                        .padding(.horizontal, 8)
                        .padding(.vertical, 4)
                        .background(Color.signal, in: Capsule())
                }
            }
        }
        .padding(14)
        .panel(cornerRadius: 16)
    }
}

private struct ConversationScreen: View {
    @EnvironmentObject private var node: StyreneNodeModel
    @Environment(\.dismiss) private var dismiss
    let conversation: ConversationRoute
    @State private var draft = ""
    @State private var previewMessages = [PreviewMessage]()
    @State private var showAttachments = false
    @State private var showDelivery = false

    var body: some View {
        VStack(spacing: 0) {
            ConversationHeader(name: conversation.name) {
                dismiss()
            }
            if conversation.isPreview {
                Text("PREVIEW THREAD  •  NO PACKETS WILL BE SENT")
                    .font(.caption2.monospaced().weight(.semibold))
                    .tracking(1)
                    .foregroundStyle(Color.signal)
                    .frame(maxWidth: .infinity)
                    .padding(10)
                    .background(Color.signal.opacity(0.1))
                    .accessibilityIdentifier("preview.label")
            }
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 10) {
                        if conversation.isPreview {
                            ForEach(previewMessages) { message in
                                MessageBubble(
                                    content: message.content,
                                    outgoing: message.outgoing,
                                    timestamp: message.timestamp,
                                    state: message.state,
                                    route: message.route,
                                    isPreview: true
                                )
                                .id(message.id)
                            }
                        } else if node.messages.isEmpty {
                            EmptyState(
                                title: "No messages yet",
                                detail: "Write the first message when a route is available."
                            )
                        } else {
                            ForEach(node.messages.reversed(), id: \.id) { message in
                                MessageBubble(
                                    content: message.content,
                                    outgoing: message.isOutgoing,
                                    timestamp: Self.time(message.timestamp),
                                    state: message.isOutgoing ? "Outgoing" : "Received",
                                    route: message.isOutgoing ? "Route evidence unavailable" : nil,
                                    isPreview: false
                                )
                                .id(message.id)
                            }
                        }
                    }
                    .padding(16)
                }
                .onChange(of: previewMessages.count) { _, _ in
                    if let id = previewMessages.last?.id { proxy.scrollTo(id, anchor: .bottom) }
                }
                .onChange(of: node.messages.count) { _, _ in
                    if let id = node.messages.first?.id { proxy.scrollTo(id, anchor: .bottom) }
                }
            }
            composer
        }
        .background(Color.ink)
        .toolbar(.hidden, for: .navigationBar)
        .toolbar(.hidden, for: .tabBar)
        .onAppear {
            if conversation.isPreview {
                previewMessages = PreviewData.messages(seed: conversation.id)
            } else {
                node.openConversation(peerHash: conversation.id)
            }
        }
        .onDisappear {
            if !conversation.isPreview { node.closeConversation() }
        }
        .alert("Attachments are not available yet", isPresented: $showAttachments) {
            Button("OK", role: .cancel) { }
        } message: {
            Text("LXMF attachments are supported by the daemon, but attachment transfer is not exported through the mobile API.")
        }
        .sheet(isPresented: $showDelivery) { DeliveryOptionsSheet() }
    }

    private var composer: some View {
        HStack(alignment: .bottom, spacing: 10) {
            Button { showAttachments = true } label: {
                Image(systemName: "plus")
                    .foregroundStyle(Color.paper)
                    .frame(width: 40, height: 40)
                    .background(Color.panelRaised, in: RoundedRectangle(cornerRadius: 12))
            }
            TextField("Message \(conversation.name)", text: $draft, axis: .vertical)
                .lineLimit(1...4)
                .padding(.horizontal, 14)
                .padding(.vertical, 11)
                .background(Color.panelRaised, in: RoundedRectangle(cornerRadius: 12))
                .foregroundStyle(Color.paper)
                .accessibilityIdentifier("messages.composer")
            Button {
                send()
            } label: {
                Image(systemName: "arrow.up")
                    .font(.body.bold())
                    .foregroundStyle(Color.ink)
                    .frame(width: 44, height: 44)
                    .background(Color.signal, in: Circle())
            }
            .disabled(draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || node.isSending)
            .accessibilityLabel("Send message")
            .accessibilityIdentifier("messages.send")
        }
        .padding(.horizontal, 12)
        .padding(.top, 8)
        .padding(.bottom, 10)
        .background(Color.panel)
        .overlay(alignment: .topLeading) {
            Button { showDelivery = true } label: {
                Label("Direct", systemImage: "arrow.right")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(Color.mist)
            }
            .offset(x: 58, y: -18)
        }
    }

    private func send() {
        let content = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !content.isEmpty else { return }
        if conversation.isPreview {
            draft = ""
            previewMessages.append(
                PreviewMessage(
                    id: "preview-\(previewMessages.count)",
                    content: content,
                    outgoing: true,
                    timestamp: "Now",
                    state: "Preview",
                    route: "Direct · Preview composer"
                )
            )
        } else {
            node.send(content) { draft = "" }
        }
    }

    private static func time(_ timestamp: Int64) -> String {
        Date(timeIntervalSince1970: TimeInterval(timestamp)).formatted(date: .omitted, time: .shortened)
    }
}

private struct ConversationHeader: View {
    let name: String
    let onBack: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Button(action: onBack) {
                Image(systemName: "chevron.left")
                    .font(.body.bold())
                    .foregroundStyle(Color.paper)
            }
            IdentityGlyph(name: name, size: 36)
            VStack(alignment: .leading, spacing: 2) {
                Text(name).font(.headline).foregroundStyle(Color.paper)
                Text("CONVERSATION")
                    .font(.caption2.monospaced())
                    .tracking(1.3)
                    .foregroundStyle(Color.mist)
            }
            Spacer()
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(Color.ink)
    }
}

private struct MessageBubble: View {
    @AppStorage("showRouteEvidence") private var showRouteEvidence = true
    let content: String
    let outgoing: Bool
    let timestamp: String
    let state: String
    let route: String?
    let isPreview: Bool

    var body: some View {
        HStack {
            if outgoing { Spacer(minLength: 54) }
            VStack(alignment: .leading, spacing: 7) {
                Text(content)
                    .foregroundStyle(outgoing ? Color.ink : Color.paper)
                Text("\(timestamp)  •  \(state)")
                    .font(.caption2.monospaced())
                    .foregroundStyle(outgoing ? Color.ink.opacity(0.62) : Color.mist)
                if let route, showRouteEvidence {
                    HStack(spacing: 4) {
                        Image(systemName: "point.3.connected.trianglepath.dotted")
                        Text(route)
                        if isPreview { Text("PREVIEW") }
                    }
                    .font(.system(size: 9, weight: .medium, design: .monospaced))
                    .foregroundStyle(outgoing ? Color.ink.opacity(0.62) : Color.cyanSignal)
                }
            }
            .padding(14)
            .background(outgoing ? Color.signal : Color.panelRaised)
            .clipShape(
                UnevenRoundedRectangle(
                    topLeadingRadius: 16,
                    bottomLeadingRadius: outgoing ? 16 : 4,
                    bottomTrailingRadius: outgoing ? 4 : 16,
                    topTrailingRadius: 16
                )
            )
            if !outgoing { Spacer(minLength: 54) }
        }
    }
}

private struct PeopleScreen: View {
    @EnvironmentObject private var node: StyreneNodeModel
    @State private var savedOnly = true
    @State private var selectedPerson: PersonCard?
    @State private var route: ConversationRoute?

    private var source: [PersonCard] {
        if node.peers.isEmpty && node.contacts.isEmpty { return PreviewData.people }
        var entries = node.contacts.map {
            PersonCard(
                id: $0.peerHash,
                name: $0.alias ?? node.displayName(for: $0.peerHash),
                detail: "Saved contact",
                saved: true,
                isPreview: false
            )
        }
        let known = Set(entries.map(\.id))
        entries += node.peers.filter { !known.contains($0.destinationHash) }.map {
            PersonCard(
                id: $0.destinationHash,
                name: $0.name ?? String($0.destinationHash.prefix(12)),
                detail: "Discovered  •  \($0.status)",
                saved: false,
                isPreview: false
            )
        }
        return entries
    }

    private var people: [PersonCard] {
        savedOnly ? source.filter(\.saved) : source
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(spacing: 12) {
                    AppHeader(title: "People")
                    Picker("Directory", selection: $savedOnly) {
                        Text("Contacts").tag(true)
                        Text("Discovered").tag(false)
                    }
                    .pickerStyle(.segmented)

                    if people.isEmpty {
                        EmptyState(
                            title: "No saved contacts",
                            detail: "Discovered identities can be saved after verification."
                        )
                    }

                    ForEach(people) { person in
                        Button { selectedPerson = person } label: {
                            HStack(spacing: 14) {
                                IdentityGlyph(name: person.name)
                                VStack(alignment: .leading, spacing: 4) {
                                    HStack(spacing: 8) {
                                        Text(person.name).font(.body.weight(.semibold))
                                        if person.isPreview { PreviewBadge() }
                                    }
                                    Text(person.detail).font(.subheadline).foregroundStyle(Color.mist)
                                }
                                Spacer()
                                Image(systemName: "chevron.right").foregroundStyle(Color.mist)
                            }
                            .foregroundStyle(Color.paper)
                            .padding(16)
                            .panel(cornerRadius: 16)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(16)
            }
            .background(Color.ink)
            .toolbar(.hidden, for: .navigationBar)
            .sheet(item: $selectedPerson) { person in
                PersonDetail(
                    person: person,
                    onMessage: {
                        selectedPerson = nil
                        DispatchQueue.main.async {
                            route = ConversationRoute(id: person.id, name: person.name, isPreview: person.isPreview)
                        }
                    }
                )
            }
            .navigationDestination(item: $route) { ConversationScreen(conversation: $0) }
        }
    }
}

extension PersonCard: Hashable {}

private struct PersonDetail: View {
    @Environment(\.dismiss) private var dismiss
    let person: PersonCard
    let onMessage: () -> Void

    var body: some View {
        NavigationStack {
            VStack(spacing: 18) {
                IdentityGlyph(name: person.name, size: 72)
                Text(person.name).font(.title2.bold())
                Text(person.detail).foregroundStyle(Color.mist)
                Text(person.id)
                    .font(.caption.monospaced())
                    .foregroundStyle(Color.mist)
                    .textSelection(.enabled)
                Divider().overlay(Color.mist.opacity(0.25))
                Text("Discovery is not connectivity. Route and link evidence will appear here when the mobile API exposes it.")
                    .font(.subheadline)
                    .foregroundStyle(Color.mist)
                HStack {
                    Button("Message") {
                        onMessage()
                        dismiss()
                    }
                    .buttonStyle(.borderedProminent)
                    Button(person.saved ? "Edit unavailable" : "Save unavailable") { }
                        .buttonStyle(.bordered)
                        .disabled(true)
                }
                Spacer()
            }
            .padding(24)
            .background(Color.ink)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) { Button("Done") { dismiss() } }
            }
        }
        .presentationDetents([.medium, .large])
    }
}

private struct NetworkScreen: View {
    @EnvironmentObject private var node: StyreneNodeModel
    @EnvironmentObject private var rnode: RNodeBluetoothController
    @AppStorage("hubAddress") private var hubAddress = ""
    @AppStorage("displayName") private var displayName = "Styrene iPhone"
    @State private var showSetup = false

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(spacing: 12) {
                    AppHeader(title: "Network")
                    SignalBrief(
                        eyebrow: "NETWORK POSTURE",
                        title: node.phase.label,
                        detail: "\(Int(node.status?.peerCount ?? 0)) peers  •  \(Int(node.status?.linkCount ?? 0)) links",
                        action: node.isRunning ? "Refresh" : "Configure"
                    ) {
                        if node.isRunning { node.refresh() } else { showSetup = true }
                    }
                    MeshPathCard()

                    HStack(spacing: 10) {
                        Button {
                            node.announce()
                        } label: {
                            Label("Announce", systemImage: "bolt.fill").frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.borderedProminent)
                        .tint(.signal)
                        .foregroundStyle(Color.ink)
                        .disabled(!node.isRunning)

                        Button {
                            node.refresh()
                        } label: {
                            Label("Observe", systemImage: "arrow.clockwise").frame(maxWidth: .infinity)
                        }
                        .buttonStyle(.bordered)
                        .disabled(!node.isRunning)
                    }

                    HStack {
                        SectionLabel("INTERFACES")
                        Spacer()
                        Button("Setup") { showSetup = true }
                            .font(.caption)
                            .accessibilityIdentifier("connection.open-setup")
                    }
                    InterfaceCard(
                        icon: "network",
                        title: "Direct TCP",
                        state: hubAddress.isEmpty ? "No active profile" : hubAddress,
                        detail: tcpDetail
                    )
                    InterfaceCard(
                        icon: "antenna.radiowaves.left.and.right",
                        title: "Bluetooth RNode",
                        state: rnode.summary,
                        detail: "Preferred bearer  •  RX \(rnode.rxPackets)  •  TX \(rnode.txPackets)",
                        action: "Scan",
                        onAction: rnode.scan
                    )
                    if rnode.hasApproval {
                        InterfaceCard(
                            icon: "checkmark.shield",
                            title: "Approved RNode",
                            state: "Reconnect is enabled",
                            detail: "This removes app approval, not the iOS Bluetooth bond",
                            action: "Forget approval",
                            onAction: rnode.forgetApproval
                        )
                    }
                    ForEach(rnode.candidates) { candidate in
                        InterfaceCard(
                            icon: "dot.radiowaves.left.and.right",
                            title: candidate.name,
                            state: "Approval and pairing required",
                            detail: "Bluetooth peripheral \(candidate.id.uuidString.suffix(5))",
                            action: "Connect",
                            onAction: { rnode.approve(candidate) }
                        )
                    }
                    FieldMapPreview()

                    if let notice = node.notice { InlineNotice(text: notice) }
                    if let error = node.errorMessage { InlineNotice(text: error, isError: true) }
                }
                .padding(16)
            }
            .background(Color.ink)
            .toolbar(.hidden, for: .navigationBar)
            .sheet(isPresented: $showSetup) {
                ConnectionSetup(
                    displayName: $displayName,
                    hubAddress: $hubAddress,
                    node: node
                )
            }
        }
    }

    private var tcpDetail: String {
        guard !hubAddress.isEmpty else { return "Configure a hub or direct peer in setup" }
        if node.status?.transportActive == true { return "Transport active" }
        return node.isRunning ? "Profile configured; transport inactive" : "Node stopped"
    }
}

private struct MeshPathCard: View {
    @EnvironmentObject private var node: StyreneNodeModel

    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            SectionLabel("ACTIVE PATH")
            HStack {
                PathNode(icon: "iphone", label: "PHONE", active: node.isRunning)
                PathLine(active: node.isRunning)
                PathNode(icon: "network", label: "LAN", active: node.status?.transportActive == true)
                PathLine(active: (node.status?.linkCount ?? 0) > 0)
                PathNode(icon: "point.3.connected.trianglepath.dotted", label: "MESH", active: (node.status?.linkCount ?? 0) > 0)
            }
            Text(node.isRunning ? "The embedded node is running. Route and link evidence remain distinct from discovery." : "Start the node from connection setup to observe the mesh.")
                .font(.caption)
                .foregroundStyle(Color.mist)
        }
        .padding(18)
        .panel(cornerRadius: 22)
    }
}

private struct PathNode: View {
    let icon: String
    let label: String
    let active: Bool

    var body: some View {
        VStack(spacing: 6) {
            Image(systemName: icon)
                .foregroundStyle(active ? Color.ink : Color.mist)
                .frame(width: 42, height: 42)
                .background(active ? Color.signal : Color.panelRaised, in: Circle())
            Text(label)
                .font(.caption2.monospaced())
                .foregroundStyle(active ? Color.paper : Color.mist)
        }
    }
}

private struct PathLine: View {
    let active: Bool
    var body: some View {
        Rectangle()
            .fill(active ? Color.signal : Color.mist.opacity(0.22))
            .frame(height: 2)
    }
}

private struct InterfaceCard: View {
    let icon: String
    let title: String
    let state: String
    let detail: String
    var action: String? = nil
    var onAction: () -> Void = {}

    var body: some View {
        HStack(spacing: 14) {
            Image(systemName: icon)
                .foregroundStyle(Color.cyanSignal)
                .frame(width: 42, height: 42)
                .background(Color.cyanSignal.opacity(0.12), in: Circle())
            VStack(alignment: .leading, spacing: 3) {
                Text(title).font(.body.weight(.semibold))
                Text(state).font(.subheadline).foregroundStyle(Color.paper)
                Text(detail).font(.caption).foregroundStyle(Color.mist)
            }
            Spacer()
            if let action {
                Button(action, action: onAction)
                    .font(.caption.weight(.semibold))
            }
        }
        .padding(16)
        .panel(cornerRadius: 18)
    }
}

private struct FieldMapPreview: View {
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Label("Field map", systemImage: "map").font(.body.weight(.semibold))
                Spacer()
                PreviewBadge()
            }
            Canvas { context, size in
                let grid = Color.mist.opacity(0.12)
                for index in 0...4 {
                    let x = size.width * CGFloat(index) / 4
                    context.stroke(Path { $0.move(to: CGPoint(x: x, y: 0)); $0.addLine(to: CGPoint(x: x, y: size.height)) }, with: .color(grid))
                }
                for index in 0...3 {
                    let y = size.height * CGFloat(index) / 3
                    context.stroke(Path { $0.move(to: CGPoint(x: 0, y: y)); $0.addLine(to: CGPoint(x: size.width, y: y)) }, with: .color(grid))
                }
                let points = [CGPoint(x: size.width * 0.18, y: size.height * 0.68), CGPoint(x: size.width * 0.52, y: size.height * 0.38), CGPoint(x: size.width * 0.81, y: size.height * 0.58)]
                context.stroke(Path { $0.move(to: points[0]); $0.addLine(to: points[1]); $0.addLine(to: points[2]) }, with: .color(Color.signal.opacity(0.55)), lineWidth: 2)
                for (index, point) in points.enumerated() {
                    let rect = CGRect(x: point.x - 5, y: point.y - 5, width: 10, height: 10)
                    context.fill(Path(ellipseIn: rect), with: .color(index == 1 ? .signal : .cyanSignal))
                }
            }
            .frame(height: 150)
            .background(Color.ink, in: RoundedRectangle(cornerRadius: 16))
            Text("Location and route are separate observations. Live map telemetry is not yet exported to mobile.")
                .font(.caption)
                .foregroundStyle(Color.mist)
        }
        .padding(18)
        .panel(cornerRadius: 22)
    }
}

private struct ConnectionSetup: View {
    @Environment(\.dismiss) private var dismiss
    @Binding var displayName: String
    @Binding var hubAddress: String
    @ObservedObject var node: StyreneNodeModel

    var body: some View {
        NavigationStack {
            Form {
                Section("Identity") {
                    TextField("Display name", text: $displayName)
                        .disabled(node.isRunning)
                        .accessibilityIdentifier("connection.display-name")
                    Text("Stored in the iOS application container and protected by Keychain on device.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Section("Direct TCP") {
                    TextField("Hub or peer address", text: $hubAddress)
                        .textInputAutocapitalization(.never)
                        .autocorrectionDisabled()
                        .disabled(node.isRunning)
                        .accessibilityIdentifier("connection.hub-address")
                    Button("Use LAN hub 192.168.0.202:4242") {
                        hubAddress = "192.168.0.202:4242"
                    }
                    .disabled(node.isRunning)
                    .accessibilityIdentifier("connection.use-lan-hub")
                    if node.isRunning {
                        Text("Stop the node before changing the active connection profile.")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                }
                Section {
                    Button(node.isRunning ? "Stop node" : "Start node") {
                        if node.isRunning {
                            node.shutdown()
                        } else {
                            node.boot(hubAddress: hubAddress, displayName: displayName)
                        }
                        dismiss()
                    }
                    .disabled(node.isBusy)
                    .accessibilityIdentifier("connection.node-action")
                } footer: {
                    Text("The node starts automatically when Styrene opens. Stop it only for configuration changes or recovery.")
                }
            }
            .navigationTitle("Connection setup")
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Done") { dismiss() }
                        .accessibilityIdentifier("connection.done")
                }
            }
        }
    }
}

private enum MoreDestination: String, CaseIterable, Identifiable {
    case identity = "Identity"
    case propagation = "Propagation"
    case pages = "Pages"
    case settings = "Settings"
    case diagnostics = "Diagnostics"
    case about = "About"

    var id: String { rawValue }
    var icon: String {
        switch self {
        case .identity: "checkmark.shield"
        case .propagation: "arrow.triangle.2.circlepath"
        case .pages: "doc.text"
        case .settings: "gearshape"
        case .diagnostics: "waveform.path.ecg"
        case .about: "info.circle"
        }
    }
    var subtitle: String {
        switch self {
        case .identity: "Public hashes and secure custody"
        case .propagation: "Background delivery and sync"
        case .pages: "Micron information access"
        case .settings: "Connections, notifications, appearance"
        case .diagnostics: "Redacted runtime evidence"
        case .about: "Capabilities and build information"
        }
    }
}

private struct MoreScreen: View {
    @EnvironmentObject private var node: StyreneNodeModel

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(spacing: 12) {
                    AppHeader(title: "More")
                    IdentitySummary()
                    ForEach(MoreDestination.allCases) { destination in
                        NavigationLink(value: destination) {
                            HStack(spacing: 14) {
                                Image(systemName: destination.icon)
                                    .foregroundStyle(Color.signal)
                                    .frame(width: 28)
                                VStack(alignment: .leading, spacing: 3) {
                                    Text(destination.rawValue).font(.body.weight(.semibold))
                                    Text(destination.subtitle).font(.caption).foregroundStyle(Color.mist)
                                }
                                Spacer()
                                Image(systemName: "chevron.right").foregroundStyle(Color.mist)
                            }
                            .foregroundStyle(Color.paper)
                            .padding(16)
                            .panel(cornerRadius: 18)
                        }
                        .buttonStyle(.plain)
                    }
                }
                .padding(16)
            }
            .background(Color.ink)
            .toolbar(.hidden, for: .navigationBar)
            .navigationDestination(for: MoreDestination.self) { MoreDetail(destination: $0) }
        }
    }
}

private struct IdentitySummary: View {
    @EnvironmentObject private var node: StyreneNodeModel

    var body: some View {
        HStack(spacing: 12) {
            IdentityGlyph(name: "You")
            VStack(alignment: .leading, spacing: 4) {
                Text("Your Styrene identity").font(.body.bold())
                Text(node.deliveryHash.isEmpty ? "Not routable" : shortHash(node.deliveryHash))
                    .font(.caption.monospaced())
                    .foregroundStyle(Color.mist)
            }
            Spacer()
            Button {
                guard !node.deliveryHash.isEmpty else { return }
                UIPasteboard.general.string = node.deliveryHash
            } label: {
                Image(systemName: "doc.on.doc").foregroundStyle(Color.signal)
            }
            .disabled(node.deliveryHash.isEmpty)
            .accessibilityLabel("Copy delivery destination")
        }
        .padding(16)
        .background(Color.panelRaised, in: RoundedRectangle(cornerRadius: 16))
    }
}

private struct MoreDetail: View {
    @EnvironmentObject private var node: StyreneNodeModel
    let destination: MoreDestination

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                if destination != .pages && destination != .settings {
                    Label(destination.rawValue, systemImage: destination.icon)
                        .font(.title2.bold())
                        .foregroundStyle(Color.signal)
                }
                if destination != .pages && destination != .settings {
                    Text(bodyText)
                        .foregroundStyle(Color.mist)
                        .textSelection(.enabled)
                }
                if destination == .identity {
                    HashBlock(label: "IDENTITY", value: node.identityHash)
                    HashBlock(label: "LXMF DELIVERY", value: node.deliveryHash)
                }
                if destination == .propagation {
                    CapabilityNotice(title: "No propagation peer configured", detail: "The production view will show last sync, queued transfers, checkpoints, and failures without conflating the legacy local queue.")
                }
                if destination == .diagnostics {
                    HashBlock(label: "RUNTIME", value: "\(node.phase.label)\nPeers \(Int(node.status?.peerCount ?? 0)) / Links \(Int(node.status?.linkCount ?? 0))")
                }
                if destination == .pages { PagesBrowser() }
                if destination == .settings { CapabilitySettings() }
                Spacer()
            }
            .padding(20)
        }
        .background(Color.ink)
        .navigationTitle(destination.rawValue)
        .navigationBarTitleDisplayMode(.inline)
    }

    private var bodyText: String {
        switch destination {
        case .identity: "Your public identity and delivery destination can be shared. Private key material never appears here."
        case .propagation: "Propagation extends delivery when a direct path is unavailable. Sync and queue truth must come from the daemon."
        case .pages: "Micron browsing is supported by the runtime but excluded from the compact communicator baseline until a mobile capability profile enables it."
        case .settings: "Everyday settings remain separate from advanced interface configuration and platform permissions."
        case .diagnostics: "Evidence is bounded and redacted. Exports must omit keys, credentials, and message payloads."
        case .about: "Native Styrene compact communicator mockup. Capability states come from the daemon; preview data is always labeled."
        }
    }
}

private struct MessagingHeader: View {
    @EnvironmentObject private var node: StyreneNodeModel
    let onIdentity: () -> Void
    let onCompose: () -> Void

    var body: some View {
        HStack(spacing: 12) {
            Button(action: onIdentity) {
                HStack(spacing: 10) {
                    IdentityGlyph(name: "You", size: 38)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Messages").font(.title3.bold()).foregroundStyle(Color.paper)
                        Text("YOU · \(node.deliveryHash.isEmpty ? "NOT ROUTABLE" : shortHash(node.deliveryHash))")
                            .font(.system(size: 9, weight: .medium, design: .monospaced))
                            .foregroundStyle(Color.mist)
                            .lineLimit(1)
                    }
                }
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("messages.identity-anchor")
            Spacer()
            Button(action: onCompose) {
                Image(systemName: "square.and.pencil")
                    .font(.body.weight(.semibold))
                    .foregroundStyle(Color.ink)
                    .frame(width: 40, height: 40)
                    .background(Color.signal, in: RoundedRectangle(cornerRadius: 12))
            }
            .accessibilityLabel("New message")
        }
    }
}

private struct InboxStatusRow: View {
    let peerCount: Int
    let unreadCount: Int
    let active: Bool
    let status: String
    let canRefresh: Bool
    let onRefresh: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Circle().fill(active ? Color.cyanSignal : Color.signal).frame(width: 8, height: 8)
            Text(status)
                .font(.caption.weight(.semibold))
            Text("· \(peerCount) peers · \(unreadCount) unread")
                .font(.caption)
                .foregroundStyle(Color.mist)
            Spacer()
            Button(action: onRefresh) {
                Image(systemName: "arrow.clockwise").foregroundStyle(Color.signal)
            }
            .disabled(!canRefresh)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 10)
        .panel(cornerRadius: 12)
    }
}

private struct DeliveryOptionsSheet: View {
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            List {
                Section("Current mobile behavior") {
                    Label("Direct", systemImage: "checkmark.circle.fill")
                    Text("The mobile API currently queues plain text with the default direct method.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
                Section("Full LXMF methods") {
                    UnavailableSetting(title: "Opportunistic", detail: "Method selection is not exported to mobile")
                    UnavailableSetting(title: "Propagated", detail: "Propagation state and fallback evidence are required")
                    UnavailableSetting(title: "Paper", detail: "Paper URI outcomes are not exported to mobile")
                }
                Section("Route evidence") {
                    Text("Delivery method, bearer, and receipt state are separate. Future evidence can identify RNode/LoRa, public TCP, or a WireGuard peer tunnel without guessing from the method.")
                        .font(.caption)
                }
            }
            .navigationTitle("Delivery options")
            .toolbar { ToolbarItem(placement: .topBarTrailing) { Button("Done") { dismiss() } } }
        }
        .presentationDetents([.medium, .large])
    }
}

private struct CapabilitySettings: View {
    @AppStorage("showRouteEvidence") private var showRouteEvidence = true
    @AppStorage("enableExperimentalPages") private var experimentalPages = true

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SettingsGroup(title: "MESSAGING") {
                Toggle("Show route evidence", isOn: $showRouteEvidence)
                UnavailableSetting(title: "Attachments", detail: "Requires mobile attachment transfer API")
                UnavailableSetting(title: "Delivery receipts", detail: "Requires typed lifecycle and receipt evidence")
                UnavailableSetting(title: "Advanced delivery methods", detail: "Requires requested and actual method projection")
            }
            SettingsGroup(title: "INFORMATION ACCESS") {
                Toggle("Experimental Micron source browser", isOn: $experimentalPages)
                UnavailableSetting(title: "Rendered pages and forms", detail: "Requires typed page sessions")
                UnavailableSetting(title: "Page file downloads", detail: "Requires mobile transfer API")
            }
            SettingsGroup(title: "DELIVERY AND NETWORK") {
                UnavailableSetting(title: "Automatic propagation", detail: "Requires propagation policy and queue state")
                UnavailableSetting(title: "Background receive", detail: "Requires iOS background task integration")
                UnavailableSetting(title: "Interface policy", detail: "Configured only when the node starts")
            }
            SettingsGroup(title: "NOTIFICATIONS") {
                UnavailableSetting(title: "Conversation notifications", detail: "Requires host notification delivery and mute state")
            }
        }
    }
}

private struct SettingsGroup<Content: View>: View {
    let title: String
    let content: Content

    init(title: String, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            SectionLabel(title)
            content
        }
        .padding(16)
        .panel(cornerRadius: 16)
    }
}

private struct UnavailableSetting: View {
    let title: String
    let detail: String

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "lock.fill").foregroundStyle(Color.mist).frame(width: 20)
            VStack(alignment: .leading, spacing: 2) {
                Text(title).foregroundStyle(Color.paper)
                Text(detail).font(.caption).foregroundStyle(Color.mist)
            }
            Spacer()
        }
    }
}

private struct PagesBrowser: View {
    @EnvironmentObject private var node: StyreneNodeModel
    @AppStorage("enableExperimentalPages") private var enabled = true
    @AppStorage("pageBrowserHost") private var host = ""
    @AppStorage("pageBrowserPath") private var path = "/page/index.mu"

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("EXPERIMENTAL SOURCE BROWSER")
                    .font(.caption2.monospaced().weight(.semibold))
                    .foregroundStyle(Color.signal)
                Spacer()
                Text("BASIC API").font(.caption2.monospaced()).foregroundStyle(Color.cyanSignal)
            }
            Text("Fetches raw Micron source. Structured rendering, links, forms, files, and page-host discovery require typed mobile page sessions.")
                .font(.caption)
                .foregroundStyle(Color.mist)
            TextField("32-character destination hash", text: $host)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
            TextField("/page/index.mu", text: $path)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textFieldStyle(.roundedBorder)
            Button {
                node.browsePage(host: host, path: path)
            } label: {
                Label(node.isBrowsingPage ? "Fetching" : "Fetch page", systemImage: "arrow.down.doc")
                    .frame(maxWidth: .infinity)
            }
            .buttonStyle(.borderedProminent)
            .disabled(
                !enabled || !node.isRunning ||
                    host.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ||
                    path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || node.isBrowsingPage
            )
            if let error = node.pageError { InlineNotice(text: error, isError: true) }
            if !node.pageSource.isEmpty {
                Text(node.pageAddress).font(.caption2.monospaced()).foregroundStyle(Color.cyanSignal)
                ScrollView([.horizontal, .vertical]) {
                    Text(node.pageSource)
                        .font(.caption.monospaced())
                        .foregroundStyle(Color.paper)
                        .textSelection(.enabled)
                }
                .frame(maxHeight: 320)
                .padding(12)
                .background(Color.ink, in: RoundedRectangle(cornerRadius: 12))
            }
        }
        .padding(16)
        .panel(cornerRadius: 16)
    }
}

private struct HashBlock: View {
    let label: String
    let value: String

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            SectionLabel(label)
            Text(value.isEmpty ? "Unavailable" : value)
                .font(.caption.monospaced())
                .foregroundStyle(Color.paper)
                .textSelection(.enabled)
        }
        .padding(16)
        .panel(cornerRadius: 16)
    }
}

private struct CapabilityNotice: View {
    let title: String
    let detail: String
    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title).font(.body.weight(.semibold))
            Text(detail).font(.caption).foregroundStyle(Color.mist)
        }
        .padding(16)
        .panel(cornerRadius: 16)
    }
}

private struct AppHeader: View {
    @EnvironmentObject private var node: StyreneNodeModel
    let title: String

    var body: some View {
        HStack(spacing: 12) {
            Text("S")
                .font(.body.weight(.black))
                .foregroundStyle(Color.ink)
                .frame(width: 30, height: 30)
                .background(Color.signal, in: Circle())
            VStack(alignment: .leading, spacing: 1) {
                Text(title).font(.title3.bold()).foregroundStyle(Color.paper)
                Text(node.phase.label.uppercased())
                    .font(.caption2.monospaced())
                    .tracking(1.4)
                    .foregroundStyle(connectionColor)
            }
            Spacer()
            Circle().fill(connectionColor).frame(width: 10, height: 10)
        }
        .padding(.bottom, 4)
    }

    private var connectionColor: Color {
        switch node.phase {
        case .running(true): .cyanSignal
        case .running(false), .starting: .signal
        case .stopping: .danger
        case .idle: .mist
        }
    }
}

private struct SignalBrief: View {
    let eyebrow: String
    let title: String
    let detail: String
    let action: String
    let onAction: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(eyebrow)
                .font(.caption2.monospaced().weight(.semibold))
                .tracking(1.6)
                .foregroundStyle(Color.signal)
            Text(title).font(.title2.bold()).foregroundStyle(Color.paper)
            Text(detail).foregroundStyle(Color.mist)
            Button(action: onAction) {
                HStack(spacing: 4) {
                    Text(action)
                    Image(systemName: "chevron.right")
                }
            }
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(Color.signal)
            .padding(.top, 5)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(20)
        .background(Color.panelRaised, in: RoundedRectangle(cornerRadius: 24))
    }
}

private struct IdentityGlyph: View {
    let name: String
    var size: CGFloat = 44

    var body: some View {
        Text(String(name.prefix(2)).uppercased())
            .font(.system(size: size * 0.3, weight: .bold, design: .monospaced))
            .foregroundStyle(Color.cyanSignal)
            .frame(width: size, height: size)
            .background(Color.cyanSignal.opacity(0.14), in: RoundedRectangle(cornerRadius: size * 0.3))
            .overlay(RoundedRectangle(cornerRadius: size * 0.3).stroke(Color.cyanSignal.opacity(0.34)))
    }
}

private struct PreviewBadge: View {
    var body: some View {
        Text("PREVIEW")
            .font(.system(size: 8, weight: .semibold, design: .monospaced))
            .tracking(1)
            .foregroundStyle(Color.cyanSignal)
            .padding(.horizontal, 7)
            .padding(.vertical, 3)
            .background(Color.cyanSignal.opacity(0.12), in: Capsule())
            .accessibilityIdentifier("preview.label")
    }
}

private struct SectionLabel: View {
    let text: String
    init(_ text: String) { self.text = text }
    var body: some View {
        Text(text)
            .font(.caption2.monospaced().weight(.medium))
            .tracking(1.5)
            .foregroundStyle(Color.mist)
    }
}

private struct InlineNotice: View {
    let text: String
    var isError = false
    var body: some View {
        HStack(spacing: 9) {
            Image(systemName: isError ? "exclamationmark.triangle" : "info.circle")
            Text(text).font(.caption)
            Spacer()
        }
        .foregroundStyle(isError ? Color.danger : Color.signal)
        .padding(12)
        .background((isError ? Color.danger : Color.signal).opacity(0.1), in: RoundedRectangle(cornerRadius: 14))
    }
}

private struct EmptyState: View {
    let title: String
    let detail: String
    var body: some View {
        VStack(spacing: 9) {
            Image(systemName: "plus.circle").foregroundStyle(Color.mist)
            Text(title).font(.body.weight(.semibold))
            Text(detail).font(.caption).foregroundStyle(Color.mist).multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity)
        .padding(24)
        .panel(cornerRadius: 20)
    }
}

private struct PanelModifier: ViewModifier {
    let cornerRadius: CGFloat
    func body(content: Content) -> some View {
        content.background(Color.panel, in: RoundedRectangle(cornerRadius: cornerRadius))
    }
}

private extension View {
    func panel(cornerRadius: CGFloat) -> some View {
        modifier(PanelModifier(cornerRadius: cornerRadius))
    }
}

private extension Color {
    static let ink = Color(red: 0.039, green: 0.067, blue: 0.094)
    static let panel = Color(red: 0.071, green: 0.114, blue: 0.153)
    static let panelRaised = Color(red: 0.098, green: 0.153, blue: 0.208)
    static let paper = Color(red: 0.918, green: 0.941, blue: 0.949)
    static let mist = Color(red: 0.569, green: 0.639, blue: 0.682)
    static let signal = Color(red: 1.0, green: 0.706, blue: 0.353)
    static let cyanSignal = Color(red: 0.416, green: 0.847, blue: 0.839)
    static let danger = Color(red: 1.0, green: 0.486, blue: 0.463)
}

private func shortHash(_ hash: String) -> String {
    guard hash.count > 18 else { return hash }
    return "\(hash.prefix(9))…\(hash.suffix(9))"
}

private func activityTimestamp(_ timestamp: Int64) -> String {
    guard timestamp > 0 else { return "No activity" }
    return Date(timeIntervalSince1970: TimeInterval(timestamp)).formatted(date: .abbreviated, time: .shortened)
}

private enum PreviewData {
    static let conversations = [
        ConversationCard(id: "preview-red", name: "Classroom Red", preview: "Meet at the west gate after sunset.", timestamp: "18:42", unread: 2, isPreview: true),
        ConversationCard(id: "preview-relay", name: "Hill Relay", preview: "Propagation window opens in 12 minutes.", timestamp: "17:06", unread: 0, isPreview: true),
        ConversationCard(id: "preview-yellow", name: "Field Team Yellow", preview: "Telemetry bundle received.", timestamp: "Yesterday", unread: 0, isPreview: true),
    ]

    static let people = [
        PersonCard(id: "7ab9b2e4139d7a915f4b813fd98a2611", name: "Classroom Red", detail: "Saved contact  •  seen 2m ago", saved: true, isPreview: true),
        PersonCard(id: "2190f04ad551cee8cd9854ba3d16a977", name: "Hill Relay", detail: "Discovered  •  2 hops", saved: true, isPreview: true),
        PersonCard(id: "2a9d603aec973592515f43d112a6e96f", name: "Unknown 2a9d60", detail: "Announced nearby  •  not verified", saved: false, isPreview: true),
    ]

    static func messages(seed: String) -> [PreviewMessage] {
        [
            PreviewMessage(id: "\(seed)-1", content: "Signal check from the ridge. Can you copy?", outgoing: false, timestamp: "18:36", state: "Received", route: "Direct · LoRa · 2 hops"),
            PreviewMessage(id: "\(seed)-2", content: "Copy. Direct path is marginal; propagation is available.", outgoing: true, timestamp: "18:38", state: "Delivered", route: "Direct · Public TCP"),
            PreviewMessage(id: "\(seed)-3", content: "Meet at the west gate after sunset.", outgoing: false, timestamp: "18:42", state: "Received", route: "Direct · WireGuard peer"),
        ]
    }
}
