import Combine
import Foundation
import SwiftUI

final class StyreneNodeModel: ObservableObject {
    enum Phase {
        case idle
        case starting
        case running(hubConfigured: Bool)
        case stopping

        var label: String {
            switch self {
            case .idle: "Node stopped"
            case .starting: "Starting node"
            case .running(true): "TCP transport running"
            case .running(false): "Node running offline"
            case .stopping: "Stopping node"
            }
        }

        var symbol: String {
            switch self {
            case .idle: "circle.dashed"
            case .starting, .stopping: "hourglass"
            case .running(true): "antenna.radiowaves.left.and.right"
            case .running(false): "antenna.radiowaves.left.and.right.slash"
            }
        }

        var tint: Color {
            switch self {
            case .running(true): .green
            case .running(false): .orange
            case .starting, .stopping: .yellow
            case .idle: .white.opacity(0.65)
            }
        }
    }

    @Published private(set) var phase = Phase.idle
    @Published private(set) var identityHash = ""
    @Published private(set) var status: NodeStatus?
    @Published private(set) var peers: [PeerInfo] = []
    @Published private(set) var conversations: [ConversationInfo] = []
    @Published private(set) var contacts: [ContactEntry] = []
    @Published private(set) var messages: [MessageEntry] = []
    @Published private(set) var selectedPeerHash: String?
    @Published private(set) var isRefreshing = false
    @Published private(set) var notice: String?
    @Published private(set) var errorMessage: String?

    private var node: MobileNode?

    var isRunning: Bool {
        if case .running = phase { return true }
        return false
    }

    var isBusy: Bool {
        switch phase {
        case .starting, .stopping: true
        case .idle, .running: false
        }
    }

    func displayName(for peerHash: String) -> String {
        if let alias = contacts.first(where: { $0.peerHash == peerHash })?.alias {
            return alias
        }
        if let name = peers.first(where: { $0.destinationHash == peerHash })?.name {
            return name
        }
        return String(peerHash.prefix(12))
    }

    func boot(hubAddress: String, displayName: String) {
        guard !isBusy, !isRunning else { return }
        phase = .starting
        errorMessage = nil
        notice = nil

        let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Styrene", isDirectory: true)
        let hub = hubAddress.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = displayName.trimmingCharacters(in: .whitespacesAndNewlines)

#if targetEnvironment(simulator)
        let identityBackend = "plaintext_file"
#else
        let identityBackend = "keychain"
#endif

        let config = MobileConfig(
            configDir: base.appendingPathComponent("Config", isDirectory: true).path,
            dataDir: base.appendingPathComponent("Data", isDirectory: true).path,
            hubAddress: hub.isEmpty ? nil : hub,
            hubDeliveryHash: nil,
            displayName: name.isEmpty ? nil : name,
            identityBackend: identityBackend,
            interfaces: [],
            enableRnodeChannel: false
        )

        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            do {
                let node = try MobileNode.boot(config: config)
                let status = try node.status()
                let peers = try node.listPeers()
                let conversations = try node.listConversations()
                let contacts = try node.listContacts()
                let identityHash = node.identityHash()
                DispatchQueue.main.async {
                    self?.node = node
                    self?.status = status
                    self?.peers = peers
                    self?.conversations = conversations
                    self?.contacts = contacts
                    self?.identityHash = identityHash
                    self?.phase = .running(hubConfigured: !hub.isEmpty)
                }
            } catch {
                DispatchQueue.main.async {
                    self?.phase = .idle
                    self?.errorMessage = error.localizedDescription
                }
            }
        }
    }

    func announce() {
        guard let node else { return }
        errorMessage = nil
        notice = nil
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            do {
                try node.announce()
                DispatchQueue.main.async {
                    self?.notice = "Announce sent"
                }
                DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                    self?.refresh()
                }
            } catch {
                self?.publish(error: error)
            }
        }
    }

    func refresh() {
        guard let node, !isRefreshing else { return }
        isRefreshing = true
        errorMessage = nil
        let selectedPeerHash = selectedPeerHash
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            do {
                let status = try node.status()
                let peers = try node.listPeers()
                let conversations = try node.listConversations()
                let contacts = try node.listContacts()
                let messages = try selectedPeerHash.map {
                    try node.getMessages(peerHash: $0, limit: 100)
                }
                DispatchQueue.main.async {
                    self?.status = status
                    self?.peers = peers
                    self?.conversations = conversations
                    self?.contacts = contacts
                    if let messages {
                        self?.messages = messages
                    }
                    self?.isRefreshing = false
                }
            } catch {
                self?.publish(error: error, finishRefresh: true)
            }
        }
    }

    func pollHub() {
        guard let node, !isRefreshing else { return }
        isRefreshing = true
        errorMessage = nil
        notice = nil
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            do {
                let result = try node.pollHub()
                DispatchQueue.main.async {
                    self?.notice = result.messageCount == 1
                        ? "Received 1 message"
                        : "Received \(result.messageCount) messages"
                    self?.isRefreshing = false
                    self?.refresh()
                }
            } catch {
                self?.publish(error: error, finishRefresh: true)
            }
        }
    }

    func openConversation(peerHash: String) {
        guard let node else { return }
        selectedPeerHash = peerHash
        messages = []
        errorMessage = nil
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            do {
                try node.markRead(peerHash: peerHash)
                let messages = try node.getMessages(peerHash: peerHash, limit: 100)
                let conversations = try node.listConversations()
                DispatchQueue.main.async {
                    guard self?.selectedPeerHash == peerHash else { return }
                    self?.messages = messages
                    self?.conversations = conversations
                }
            } catch {
                self?.publish(error: error)
            }
        }
    }

    func closeConversation() {
        selectedPeerHash = nil
        messages = []
    }

    func send(_ content: String) {
        guard let node, let peerHash = selectedPeerHash else { return }
        let text = content.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        errorMessage = nil
        notice = nil
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            do {
                _ = try node.sendChat(peerHash: peerHash, content: text)
                let messages = try node.getMessages(peerHash: peerHash, limit: 100)
                let conversations = try node.listConversations()
                DispatchQueue.main.async {
                    guard self?.selectedPeerHash == peerHash else { return }
                    self?.messages = messages
                    self?.conversations = conversations
                    self?.notice = "Message queued"
                }
            } catch {
                self?.publish(error: error)
            }
        }
    }

    func shutdown() {
        guard let node else { return }
        self.node = nil
        phase = .stopping
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            node.shutdown()
            DispatchQueue.main.async {
                self?.status = nil
                self?.peers = []
                self?.conversations = []
                self?.contacts = []
                self?.messages = []
                self?.selectedPeerHash = nil
                self?.notice = nil
                self?.phase = .idle
            }
        }
    }

    private func publish(error: Error, finishRefresh: Bool = false) {
        DispatchQueue.main.async { [weak self] in
            self?.errorMessage = error.localizedDescription
            if finishRefresh {
                self?.isRefreshing = false
            }
        }
    }
}
