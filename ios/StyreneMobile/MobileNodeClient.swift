import Foundation

protocol MobileNodeClient: AnyObject {
    func announce() throws
    func browsePage(host: String, path: String) throws -> String
    func deliveryHash() -> String?
    func getMessages(peerHash: String, limit: UInt32) throws -> [MessageEntry]
    func identityHash() -> String
    func listContacts() throws -> [ContactEntry]
    func listConversations() throws -> [ConversationInfo]
    func listPeers() throws -> [PeerInfo]
    func markRead(peerHash: String) throws
    func pollHub() throws -> PollResult
    func sendChat(peerHash: String, content: String) throws -> String
    func shutdown()
    func status() throws -> NodeStatus
}

protocol MobileNodeBooting {
    func boot(config: MobileConfig) throws -> any MobileNodeClient
}

protocol NodeScheduling {
    func work(_ operation: @escaping () -> Void)
    func main(_ operation: @escaping () -> Void)
    func main(after delay: TimeInterval, _ operation: @escaping () -> Void)
}

struct MobileNodeBootFactory: MobileNodeBooting {
    func boot(config: MobileConfig) throws -> any MobileNodeClient {
        GeneratedMobileNodeClient(node: try MobileNode.boot(config: config))
    }
}

struct DispatchNodeScheduler: NodeScheduling {
    private let queue = DispatchQueue(label: "io.styrene.mesh.node", qos: .userInitiated)

    func work(_ operation: @escaping () -> Void) {
        queue.async(execute: operation)
    }

    func main(_ operation: @escaping () -> Void) {
        DispatchQueue.main.async(execute: operation)
    }

    func main(after delay: TimeInterval, _ operation: @escaping () -> Void) {
        DispatchQueue.main.asyncAfter(deadline: .now() + delay, execute: operation)
    }
}

private final class GeneratedMobileNodeClient: MobileNodeClient, RNodePacketChannel {
    private let node: MobileNode

    init(node: MobileNode) {
        self.node = node
    }

    func announce() throws { try node.announce() }
    func browsePage(host: String, path: String) throws -> String { try node.browsePage(host: host, path: path) }
    func deliveryHash() -> String? { node.deliveryHash() }
    func getMessages(peerHash: String, limit: UInt32) throws -> [MessageEntry] { try node.getMessages(peerHash: peerHash, limit: limit) }
    func identityHash() -> String { node.identityHash() }
    func listContacts() throws -> [ContactEntry] { try node.listContacts() }
    func listConversations() throws -> [ConversationInfo] { try node.listConversations() }
    func listPeers() throws -> [PeerInfo] { try node.listPeers() }
    func markRead(peerHash: String) throws { try node.markRead(peerHash: peerHash) }
    func pollHub() throws -> PollResult { try node.pollHub() }
    func pollRnodePacket() throws -> Data? { try node.pollRnodePacket() }
    func sendChat(peerHash: String, content: String) throws -> String { try node.sendChat(peerHash: peerHash, content: content) }
    func shutdown() { node.shutdown() }
    func status() throws -> NodeStatus { try node.status() }
    func submitRnodePacket(_ packet: Data) throws { try node.submitRnodePacket(packet: packet) }
}
