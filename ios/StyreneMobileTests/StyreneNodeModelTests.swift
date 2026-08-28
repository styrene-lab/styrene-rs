import XCTest
@testable import StyreneMobile

final class StyreneNodeModelTests: XCTestCase {
    func testIntegrationProfileParsesIsolatedHubConfiguration() throws {
        let profile = try XCTUnwrap(
            MobileIntegrationLaunchProfile.parse(
                arguments: [
                    "StyreneMobile",
                    "--styrene-integration-profile", "ios-a",
                    "--styrene-hub-address", " 127.0.0.1:4242 ",
                    "--styrene-display-name", " iOS A ",
                    "--styrene-reset-state",
                ]
            )
        )

        XCTAssertEqual(profile.id, "ios-a")
        XCTAssertEqual(profile.hubAddress, "127.0.0.1:4242")
        XCTAssertEqual(profile.displayName, "iOS A")
        XCTAssertTrue(profile.resetState)
        XCTAssertEqual(
            profile.storageRoot(applicationSupport: URL(fileURLWithPath: "/app/support")).path,
            "/app/support/Styrene/Integration/ios-a"
        )
    }

    func testIntegrationProfileRejectsTraversalAndUnscopedOptions() {
        XCTAssertThrowsError(
            try MobileIntegrationLaunchProfile.parse(
                arguments: [
                    "StyreneMobile",
                    "--styrene-integration-profile", "../node",
                    "--styrene-hub-address", "127.0.0.1:4242",
                ]
            )
        )
        XCTAssertThrowsError(
            try MobileIntegrationLaunchProfile.parse(
                arguments: ["StyreneMobile", "--styrene-hub-address", "127.0.0.1:4242"]
            )
        )
    }

    func testHydrationFailureShutsDownBootedNode() {
        let node = FakeMobileNode()
        node.listPeersError = TestError.hydration
        let (model, scheduler) = makeModel(node: node)

        model.boot(hubAddress: "", displayName: "Test")
        scheduler.runNextWork()
        scheduler.runNextMain()

        XCTAssertEqual(node.shutdownCount, 1)
        XCTAssertFalse(model.isRunning)
        XCTAssertEqual(model.errorMessage, TestError.hydration.localizedDescription)
    }

    func testShutdownWhileBootIsQueuedRejectsAndShutsDownResult() {
        let node = FakeMobileNode()
        let (model, scheduler) = makeModel(node: node)

        model.boot(hubAddress: "", displayName: "Test")
        model.shutdown()
        XCTAssertTrue(model.isBusy)

        scheduler.runNextWork()
        scheduler.runNextMain()
        scheduler.runNextWork()
        scheduler.runNextMain()

        XCTAssertEqual(node.shutdownCount, 1)
        XCTAssertFalse(model.isRunning)
        XCTAssertFalse(model.isBusy)
        XCTAssertNil(model.status)
    }

    func testRepeatedShutdownOfRunningNodeRemainsIdle() {
        let node = FakeMobileNode()
        let (model, scheduler) = runningModel(node: node)

        model.shutdown()
        model.shutdown()
        scheduler.runNextWork()
        scheduler.runNextMain()

        XCTAssertEqual(node.shutdownCount, 1)
        XCTAssertFalse(model.isBusy)
        XCTAssertFalse(model.isRunning)
    }

    func testCloseAndReopenSamePeerRejectsStaleMessages() {
        let node = FakeMobileNode()
        let (model, scheduler) = runningModel(node: node)
        node.messageResponses = [
            [message(id: "stale", content: "old")],
            [message(id: "current", content: "new")],
        ]

        model.openConversation(peerHash: "peer")
        model.closeConversation()
        model.openConversation(peerHash: "peer")
        scheduler.runNextWork()
        scheduler.runNextWork()
        scheduler.runNextMain()
        scheduler.runNextMain()

        XCTAssertEqual(model.messages.map(\.id), ["current"])
        XCTAssertNil(model.errorMessage)
    }

    func testOnlyMatchingSendCompletionInvokesDraftClear() {
        let node = FakeMobileNode()
        let (model, scheduler) = runningModel(node: node)
        node.messageResponses = [
            [message(id: "initial", content: "initial")],
            [message(id: "stale-send", content: "stale")],
            [message(id: "reopened", content: "reopened")],
            [message(id: "current-send", content: "current")],
        ]

        model.openConversation(peerHash: "peer")
        scheduler.runNextWork()
        scheduler.runNextMain()

        var staleClearCount = 0
        var currentClearCount = 0
        model.send("first") { staleClearCount += 1 }
        model.closeConversation()
        model.openConversation(peerHash: "peer")
        model.send("second") { currentClearCount += 1 }

        scheduler.runNextWork()
        scheduler.runNextWork()
        scheduler.runNextWork()
        scheduler.runNextMain()
        scheduler.runNextMain()
        scheduler.runNextMain()

        XCTAssertEqual(staleClearCount, 0)
        XCTAssertEqual(currentClearCount, 1)
        XCTAssertEqual(model.messages.map(\.id), ["current-send"])
        XCTAssertFalse(model.isSending)
    }

    private func makeModel(node: FakeMobileNode) -> (StyreneNodeModel, TestScheduler) {
        let scheduler = TestScheduler()
        return (
            StyreneNodeModel(bootFactory: FakeBootFactory(node: node), scheduler: scheduler),
            scheduler
        )
    }

    func testRNodeKissDecoderHandlesFragmentedEscapedFrame() {
        let encoded = RNodeKissEncoder.frame(command: 0, payload: [0x11, 0xC0, 0xDB, 0x22])
        var decoder = RNodeKissDecoder()

        XCTAssertTrue(decoder.feed(Data(encoded.prefix(3))).isEmpty)
        let frames = decoder.feed(Data(encoded.dropFirst(3)))

        XCTAssertEqual(frames.count, 1)
        XCTAssertEqual(frames[0].command, 0)
        XCTAssertEqual(frames[0].payload, [0x11, 0xC0, 0xDB, 0x22])
    }

    func testRNodeConfigurationUsesNetworkByteOrder() {
        XCTAssertEqual(
            RNodeBluetoothController.uint32(915_000_000),
            [0x36, 0x89, 0xCA, 0xC0]
        )
        XCTAssertEqual(
            RNodeBluetoothController.uint32(125_000),
            [0x00, 0x01, 0xE8, 0x48]
        )
    }

    func testRNodeKissDecoderRejectsOversizedFrameAndRecovers() {
        var decoder = RNodeKissDecoder()
        let oversized = Data([0xC0, 0x00] + Array(repeating: 0x41, count: RNodeKissDecoder.maximumFrameBytes))

        XCTAssertTrue(decoder.feed(oversized).isEmpty)
        let frames = decoder.feed(Data(RNodeKissEncoder.frame(command: 0, payload: [0x22])))

        XCTAssertEqual(frames.count, 1)
        XCTAssertEqual(frames[0].payload, [0x22])
    }

    func testRNodeKissDecoderRejectsInvalidEscapeAndRecovers() {
        var decoder = RNodeKissDecoder()

        XCTAssertTrue(decoder.feed(Data([0xC0, 0x00, 0xDB, 0x41, 0xC0])).isEmpty)
        let frames = decoder.feed(Data(RNodeKissEncoder.frame(command: 0, payload: [0x33])))

        XCTAssertEqual(frames.count, 1)
        XCTAssertEqual(frames[0].payload, [0x33])
    }

    func testOutboundRetentionSurvivesSameChannelReattachmentOnly() {
        let first = FakePacketChannel()
        let replacement = FakePacketChannel()
        let retention = RNodeOutboundRetention()
        retention.reserve(Data([0x11]), for: first)
        retention.markEnqueued()

        retention.attach(nil)
        retention.markNotEnqueued()
        retention.attach(first)
        XCTAssertEqual(retention.packet, Data([0x11]))
        XCTAssertFalse(retention.enqueued)

        retention.attach(replacement)
        XCTAssertNil(retention.packet)
    }

    func testRNodeWriteQueueChunksAndSerializesOneWriteAtATime() {
        let queue = RNodeWriteQueue()
        let data = Data(0..<45)

        XCTAssertTrue(queue.enqueue(data, chunkSize: 20, completesOutbound: true))
        XCTAssertEqual(queue.pendingCount, 3)
        XCTAssertEqual(queue.startNext()?.data, Data(0..<20))
        XCTAssertNil(queue.startNext())
        XCTAssertFalse(queue.finishActive(retry: false)?.completesOutbound ?? true)
        XCTAssertEqual(queue.startNext()?.data, Data(20..<40))
        XCTAssertFalse(queue.finishActive(retry: false)?.completesOutbound ?? true)
        XCTAssertEqual(queue.startNext()?.data, Data(40..<45))
        XCTAssertTrue(queue.finishActive(retry: false)?.completesOutbound ?? false)
        XCTAssertNil(queue.startNext())
    }

    func testRNodeWriteQueueRetriesActiveChunkBeforeLaterChunks() {
        let queue = RNodeWriteQueue()
        XCTAssertTrue(queue.enqueue(Data(0..<25), chunkSize: 20, completesOutbound: true))

        let first = queue.startNext()
        queue.finishActive(retry: true)

        XCTAssertEqual(queue.startNext()?.data, first?.data)
        XCTAssertFalse(queue.finishActive(retry: false)?.completesOutbound ?? true)
        XCTAssertTrue(queue.startNext()?.completesOutbound ?? false)
    }

    func testRNodeWriteQueueRejectsCapacityWithoutPartialEnqueue() {
        let queue = RNodeWriteQueue()
        for byte in 0..<RNodeWriteQueue.maximumPendingChunks {
            XCTAssertTrue(queue.enqueue(Data([UInt8(byte)]), chunkSize: 20, completesOutbound: false))
        }

        XCTAssertFalse(queue.enqueue(Data([0xFF]), chunkSize: 20, completesOutbound: true))
        XCTAssertEqual(queue.pendingCount, RNodeWriteQueue.maximumPendingChunks)
    }

    private func runningModel(node: FakeMobileNode) -> (StyreneNodeModel, TestScheduler) {
        let (model, scheduler) = makeModel(node: node)
        model.boot(hubAddress: "", displayName: "Test")
        scheduler.runNextWork()
        scheduler.runNextMain()
        XCTAssertTrue(model.isRunning)
        return (model, scheduler)
    }

    private func message(id: String, content: String) -> MessageEntry {
        MessageEntry(
            id: id,
            sourceHash: "peer",
            destinationHash: "local",
            content: content,
            timestamp: 1,
            isOutgoing: false
        )
    }
}

private enum TestError: LocalizedError {
    case hydration

    var errorDescription: String? { "Hydration failed" }
}

private final class TestScheduler: NodeScheduling {
    private var workItems: [() -> Void] = []
    private var mainItems: [() -> Void] = []

    func work(_ operation: @escaping () -> Void) {
        workItems.append(operation)
    }

    func main(_ operation: @escaping () -> Void) {
        mainItems.append(operation)
    }

    func main(after delay: TimeInterval, _ operation: @escaping () -> Void) {
        mainItems.append(operation)
    }

    func runNextWork() {
        XCTAssertFalse(workItems.isEmpty, "Expected queued work")
        workItems.removeFirst()()
    }

    func runNextMain() {
        XCTAssertFalse(mainItems.isEmpty, "Expected queued main work")
        mainItems.removeFirst()()
    }
}

private struct FakeBootFactory: MobileNodeBooting {
    let node: FakeMobileNode

    func boot(config: MobileConfig) throws -> any MobileNodeClient {
        node
    }
}

private final class FakeMobileNode: MobileNodeClient {
    var listPeersError: Error?
    var messageResponses: [[MessageEntry]] = []
    private(set) var shutdownCount = 0

    func announce() throws {}
    func browsePage(host: String, path: String) throws -> String { "" }
    func deliveryHash() -> String? { "delivery" }

    func getMessages(peerHash: String, limit: UInt32) throws -> [MessageEntry] {
        guard !messageResponses.isEmpty else { return [] }
        return messageResponses.removeFirst()
    }

    func identityHash() -> String { "identity" }
    func listContacts() throws -> [ContactEntry] { [] }
    func listConversations() throws -> [ConversationInfo] { [] }

    func listPeers() throws -> [PeerInfo] {
        if let listPeersError { throw listPeersError }
        return []
    }

    func markRead(peerHash: String) throws {}
    func pollHub() throws -> PollResult { PollResult(messageCount: 0, messages: []) }
    func sendChat(peerHash: String, content: String) throws -> String { "message-id" }
    func shutdown() { shutdownCount += 1 }

    func status() throws -> NodeStatus {
        NodeStatus(
            identityHash: "identity",
            daemonVersion: "test",
            transportActive: false,
            peerCount: 0,
            linkCount: 0,
            uptimeSecs: 0
        )
    }
}

private final class FakePacketChannel: RNodePacketChannel {
    func announce() throws {}
    func pollRnodePacket() throws -> Data? { nil }
    func submitRnodePacket(_ packet: Data) throws {}
}
