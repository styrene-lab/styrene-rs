import Combine
import Foundation
import SwiftUI

struct MobileIntegrationLaunchProfile: Equatable {
    enum ParsingError: LocalizedError {
        case duplicateOption(String)
        case invalidProfileID
        case missingHubAddress
        case missingOptionValue(String)
        case optionsRequireProfile
        case unknownOption(String)

        var errorDescription: String? {
            switch self {
            case .duplicateOption(let option): "Duplicate integration option: \(option)"
            case .invalidProfileID: "Invalid integration profile ID"
            case .missingHubAddress: "Integration profile requires --styrene-hub-address"
            case .missingOptionValue(let option): "Integration option requires a value: \(option)"
            case .optionsRequireProfile: "Integration options require --styrene-integration-profile"
            case .unknownOption(let option): "Unknown Styrene launch option: \(option)"
            }
        }
    }

    let id: String
    let hubAddress: String
    let displayName: String
    let resetState: Bool

    func storageRoot(applicationSupport: URL) -> URL {
        applicationSupport
            .appendingPathComponent("Styrene", isDirectory: true)
            .appendingPathComponent("Integration", isDirectory: true)
            .appendingPathComponent(id, isDirectory: true)
    }

    static func parse(arguments: [String]) throws -> MobileIntegrationLaunchProfile? {
        let profileOption = "--styrene-integration-profile"
        let hubOption = "--styrene-hub-address"
        let displayNameOption = "--styrene-display-name"
        let resetOption = "--styrene-reset-state"
        let knownOptions = Set([profileOption, hubOption, displayNameOption, resetOption])
        if let unknown = arguments.first(where: { $0.hasPrefix("--styrene-") && !knownOptions.contains($0) }) {
            throw ParsingError.unknownOption(unknown)
        }

        let profileID = try value(after: profileOption, in: arguments)
        let hubAddress = try value(after: hubOption, in: arguments)
        let displayName = try value(after: displayNameOption, in: arguments)
        let resetCount = arguments.filter { $0 == resetOption }.count
        if resetCount > 1 { throw ParsingError.duplicateOption(resetOption) }

        guard let profileID else {
            guard hubAddress == nil, displayName == nil, resetCount == 0 else {
                throw ParsingError.optionsRequireProfile
            }
            return nil
        }
        guard profileID.range(of: #"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$"#, options: .regularExpression) != nil else {
            throw ParsingError.invalidProfileID
        }
        let hub = hubAddress?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        guard !hub.isEmpty else { throw ParsingError.missingHubAddress }
        let name = displayName?.trimmingCharacters(in: .whitespacesAndNewlines)
        let resolvedName = name.flatMap { $0.isEmpty ? nil : $0 } ?? "iOS \(profileID)"
        return MobileIntegrationLaunchProfile(
            id: profileID,
            hubAddress: hub,
            displayName: resolvedName,
            resetState: resetCount == 1
        )
    }

    private static func value(after option: String, in arguments: [String]) throws -> String? {
        let positions = arguments.indices.filter { arguments[$0] == option }
        if positions.count > 1 { throw ParsingError.duplicateOption(option) }
        guard let position = positions.first else { return nil }
        let valuePosition = arguments.index(after: position)
        guard valuePosition < arguments.endIndex, !arguments[valuePosition].hasPrefix("--") else {
            throw ParsingError.missingOptionValue(option)
        }
        return arguments[valuePosition]
    }
}

final class StyreneNodeModel: ObservableObject {
    enum Phase {
        case idle
        case starting
        case running(transportActive: Bool)
        case stopping

        var label: String {
            switch self {
            case .idle: "Node stopped"
            case .starting: "Starting node"
            case .running(true): "Transport active"
            case .running(false): "Node ready"
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
    @Published private(set) var deliveryHash = ""
    @Published private(set) var status: NodeStatus?
    @Published private(set) var peers: [PeerInfo] = []
    @Published private(set) var conversations: [ConversationInfo] = []
    @Published private(set) var contacts: [ContactEntry] = []
    @Published private(set) var messages: [MessageEntry] = []
    @Published private(set) var selectedPeerHash: String?
    @Published private(set) var isRefreshing = false
    @Published private(set) var notice: String?
    @Published private(set) var errorMessage: String?
    @Published private(set) var pageSource = ""
    @Published private(set) var isBrowsingPage = false
    @Published private(set) var pageError: String?
    @Published private(set) var pageAddress = ""
    @Published private(set) var isSending = false

    private let bootFactory: any MobileNodeBooting
    private let scheduler: any NodeScheduling
    private var node: (any MobileNodeClient)?
    private var nodeGeneration = 0
    private var conversationGeneration = 0
    private var conversationRequest = 0
    private var messageRequest = 0
    private var refreshRequest = 0
    private var sendRequest = 0

    init(
        bootFactory: any MobileNodeBooting = MobileNodeBootFactory(),
        scheduler: any NodeScheduling = DispatchNodeScheduler()
    ) {
        self.bootFactory = bootFactory
        self.scheduler = scheduler
    }

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
        let applicationSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        boot(
            hubAddress: hubAddress,
            displayName: displayName,
            storageRoot: applicationSupport.appendingPathComponent("Styrene", isDirectory: true),
            resetState: false
        )
    }

    func boot(profile: MobileIntegrationLaunchProfile) {
        let applicationSupport = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        boot(
            hubAddress: profile.hubAddress,
            displayName: profile.displayName,
            storageRoot: profile.storageRoot(applicationSupport: applicationSupport),
            resetState: profile.resetState
        )
    }

    private func boot(hubAddress: String, displayName: String, storageRoot: URL, resetState: Bool) {
        guard !isBusy, !isRunning else { return }
        if resetState, FileManager.default.fileExists(atPath: storageRoot.path) {
            do {
                try FileManager.default.removeItem(at: storageRoot)
            } catch {
                errorMessage = "Unable to reset integration profile: \(error.localizedDescription)"
                return
            }
        }
        nodeGeneration += 1
        let generation = nodeGeneration
        phase = .starting
        errorMessage = nil
        notice = nil

        let hub = hubAddress.trimmingCharacters(in: .whitespacesAndNewlines)
        let name = displayName.trimmingCharacters(in: .whitespacesAndNewlines)

#if targetEnvironment(simulator)
        let identityBackend = "plaintext_file"
#else
        let identityBackend = "keychain"
#endif

        let config = MobileConfig(
            configDir: storageRoot.appendingPathComponent("Config", isDirectory: true).path,
            dataDir: storageRoot.appendingPathComponent("Data", isDirectory: true).path,
            hubAddress: hub.isEmpty ? nil : hub,
            hubDeliveryHash: nil,
            displayName: name.isEmpty ? nil : name,
            identityBackend: identityBackend,
            interfaces: [],
            enableRnodeChannel: true
        )

        scheduler.work { [weak self, bootFactory, scheduler] in
            var bootedNode: (any MobileNodeClient)?
            do {
                let node = try bootFactory.boot(config: config)
                bootedNode = node
                let status = try node.status()
                let peers = try node.listPeers()
                let conversations = try node.listConversations()
                let contacts = try node.listContacts()
                let identityHash = node.identityHash()
                let deliveryHash = node.deliveryHash() ?? ""
                scheduler.main { [weak self] in
                    guard let self, self.nodeGeneration == generation, case .starting = self.phase else {
                        let cleanupGeneration = self?.nodeGeneration
                        scheduler.work {
                            node.shutdown()
                            guard let cleanupGeneration else { return }
                            scheduler.main { [weak self] in
                                guard let self,
                                      self.nodeGeneration == cleanupGeneration,
                                      self.node == nil,
                                      case .stopping = self.phase else { return }
                                self.phase = .idle
                            }
                        }
                        return
                    }
                    self.node = node
                    self.status = status
                    self.peers = peers
                    self.conversations = conversations
                    self.contacts = contacts
                    self.identityHash = identityHash
                    self.deliveryHash = deliveryHash
                    self.phase = .running(transportActive: status.transportActive)
                }
            } catch {
                bootedNode?.shutdown()
                scheduler.main { [weak self] in
                    guard let self else { return }
                    if self.nodeGeneration == generation {
                        self.phase = .idle
                        self.errorMessage = error.localizedDescription
                    } else if self.node == nil, case .stopping = self.phase {
                        self.phase = .idle
                    }
                }
            }
        }
    }

    func announce() {
        guard let node else { return }
        errorMessage = nil
        notice = nil
        let generation = nodeGeneration
        scheduler.work { [weak self, scheduler] in
            do {
                try node.announce()
                scheduler.main {
                    guard self?.nodeGeneration == generation else { return }
                    self?.notice = "Announce sent"
                }
                scheduler.main(after: 2) {
                    guard self?.nodeGeneration == generation else { return }
                    self?.refresh()
                }
            } catch {
                self?.publish(error: error, generation: generation)
            }
        }
    }

    func rnodePacketChannel() -> (any RNodePacketChannel)? {
        node as? any RNodePacketChannel
    }

    func refresh() {
        guard let node, !isRefreshing else { return }
        isRefreshing = true
        errorMessage = nil
        let selectedPeerHash = selectedPeerHash
        let generation = nodeGeneration
        refreshRequest += 1
        let refresh = refreshRequest
        conversationRequest += 1
        let conversationsRequest = conversationRequest
        let messagesRequest: Int?
        if selectedPeerHash != nil {
            messageRequest += 1
            messagesRequest = messageRequest
        } else {
            messagesRequest = nil
        }
        scheduler.work { [weak self, scheduler] in
            do {
                let status = try node.status()
                let peers = try node.listPeers()
                let conversations = try node.listConversations()
                let contacts = try node.listContacts()
                let messages = try selectedPeerHash.map {
                    try node.getMessages(peerHash: $0, limit: 100)
                }
                scheduler.main {
                    guard self?.nodeGeneration == generation, self?.refreshRequest == refresh else { return }
                    self?.status = status
                    self?.peers = peers
                    self?.contacts = contacts
                    self?.phase = .running(transportActive: status.transportActive)
                    if self?.conversationRequest == conversationsRequest {
                        self?.conversations = conversations
                    }
                    if let messages, let messagesRequest,
                       self?.messageRequest == messagesRequest,
                       self?.selectedPeerHash == selectedPeerHash {
                        self?.messages = messages
                    }
                    self?.isRefreshing = false
                }
            } catch {
                self?.publish(error: error, generation: generation, refreshRequest: refresh)
            }
        }
    }

    func pollHub() {
        guard let node, !isRefreshing else { return }
        isRefreshing = true
        errorMessage = nil
        notice = nil
        let generation = nodeGeneration
        refreshRequest += 1
        let refresh = refreshRequest
        scheduler.work { [weak self, scheduler] in
            do {
                let result = try node.pollHub()
                scheduler.main {
                    guard self?.nodeGeneration == generation, self?.refreshRequest == refresh else { return }
                    self?.notice = result.messageCount == 1
                        ? "Received 1 message"
                        : "Received \(result.messageCount) messages"
                    self?.isRefreshing = false
                    self?.refresh()
                }
            } catch {
                self?.publish(error: error, generation: generation, refreshRequest: refresh)
            }
        }
    }

    func openConversation(peerHash: String) {
        guard let node else { return }
        selectedPeerHash = peerHash
        messages = []
        errorMessage = nil
        let generation = nodeGeneration
        conversationGeneration += 1
        let session = conversationGeneration
        conversationRequest += 1
        let conversationsRequest = conversationRequest
        messageRequest += 1
        let messagesRequest = messageRequest
        scheduler.work { [weak self, scheduler] in
            do {
                try node.markRead(peerHash: peerHash)
                let messages = try node.getMessages(peerHash: peerHash, limit: 100)
                let conversations = try node.listConversations()
                scheduler.main {
                    guard self?.nodeGeneration == generation,
                          self?.conversationGeneration == session,
                          self?.selectedPeerHash == peerHash else { return }
                    if self?.messageRequest == messagesRequest { self?.messages = messages }
                    if self?.conversationRequest == conversationsRequest { self?.conversations = conversations }
                }
            } catch {
                self?.publish(
                    error: error,
                    generation: generation,
                    conversationGeneration: session,
                    conversationRequest: conversationsRequest,
                    messageRequest: messagesRequest
                )
            }
        }
    }

    func closeConversation() {
        conversationGeneration += 1
        messageRequest += 1
        sendRequest += 1
        selectedPeerHash = nil
        messages = []
        isSending = false
    }

    func send(_ content: String, onQueued: @escaping () -> Void) {
        guard let node, let peerHash = selectedPeerHash, !isSending else { return }
        let text = content.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        errorMessage = nil
        notice = nil
        isSending = true
        let generation = nodeGeneration
        let session = conversationGeneration
        sendRequest += 1
        let send = sendRequest
        conversationRequest += 1
        let conversationsRequest = conversationRequest
        messageRequest += 1
        let messagesRequest = messageRequest
        scheduler.work { [weak self, scheduler] in
            do {
                _ = try node.sendChat(peerHash: peerHash, content: text)
                let messages = try node.getMessages(peerHash: peerHash, limit: 100)
                let conversations = try node.listConversations()
                scheduler.main {
                    guard self?.nodeGeneration == generation, self?.sendRequest == send else { return }
                    self?.isSending = false
                    guard self?.conversationGeneration == session, self?.selectedPeerHash == peerHash else { return }
                    if self?.messageRequest == messagesRequest { self?.messages = messages }
                    if self?.conversationRequest == conversationsRequest { self?.conversations = conversations }
                    self?.notice = "Message queued"
                    onQueued()
                }
            } catch {
                scheduler.main {
                    guard self?.nodeGeneration == generation, self?.sendRequest == send else { return }
                    self?.isSending = false
                    guard self?.conversationGeneration == session, self?.selectedPeerHash == peerHash else { return }
                    self?.errorMessage = error.localizedDescription
                }
            }
        }
    }

    func browsePage(host: String, path: String) {
        guard let node, !isBrowsingPage else { return }
        let destination = host.trimmingCharacters(in: .whitespacesAndNewlines)
        let nativePath = path.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !destination.isEmpty, !nativePath.isEmpty else { return }
        isBrowsingPage = true
        pageError = nil
        pageSource = ""
        pageAddress = ""
        let generation = nodeGeneration
        scheduler.work { [weak self, scheduler] in
            do {
                let source = try node.browsePage(host: destination, path: nativePath)
                scheduler.main {
                    guard self?.nodeGeneration == generation else { return }
                    self?.pageSource = source
                    self?.pageAddress = "\(destination):\(nativePath)"
                    self?.isBrowsingPage = false
                }
            } catch {
                scheduler.main {
                    guard self?.nodeGeneration == generation else { return }
                    self?.pageError = error.localizedDescription
                    self?.isBrowsingPage = false
                }
            }
        }
    }

    func shutdown() {
        if case .stopping = phase { return }
        shutdownCurrentNode()
    }

    private func shutdownCurrentNode() {
        guard node != nil || isBusy else { return }
        let node = node
        self.node = nil
        nodeGeneration += 1
        let generation = nodeGeneration
        conversationGeneration += 1
        conversationRequest += 1
        messageRequest += 1
        refreshRequest += 1
        sendRequest += 1
        isRefreshing = false
        isSending = false
        isBrowsingPage = false
        clearNodeState()
        guard let node else {
            phase = .stopping
            return
        }
        phase = .stopping
        scheduler.work { [weak self, scheduler] in
            node.shutdown()
            scheduler.main {
                guard self?.nodeGeneration == generation, self?.node == nil else { return }
                self?.phase = .idle
            }
        }
    }

    private func clearNodeState() {
        status = nil
        identityHash = ""
        deliveryHash = ""
        peers = []
        conversations = []
        contacts = []
        messages = []
        selectedPeerHash = nil
        notice = nil
        pageSource = ""
        pageError = nil
        pageAddress = ""
    }

    private func publish(
        error: Error,
        generation: Int? = nil,
        conversationGeneration: Int? = nil,
        refreshRequest: Int? = nil,
        conversationRequest: Int? = nil,
        messageRequest: Int? = nil
    ) {
        scheduler.main { [weak self] in
            if let generation, self?.nodeGeneration != generation { return }
            if let conversationGeneration, self?.conversationGeneration != conversationGeneration { return }
            if let refreshRequest, self?.refreshRequest != refreshRequest { return }
            if let conversationRequest, self?.conversationRequest != conversationRequest { return }
            if let messageRequest, self?.messageRequest != messageRequest { return }
            self?.errorMessage = error.localizedDescription
            if refreshRequest != nil {
                self?.isRefreshing = false
            }
        }
    }
}
