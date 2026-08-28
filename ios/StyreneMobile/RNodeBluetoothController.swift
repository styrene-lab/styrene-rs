import CoreBluetooth
import Foundation

protocol RNodePacketChannel: AnyObject {
    func announce() throws
    func pollRnodePacket() throws -> Data?
    func submitRnodePacket(_ packet: Data) throws
}

struct BluetoothRNodeCandidate: Identifiable, Equatable {
    let id: UUID
    let name: String
}

final class RNodeBluetoothController: NSObject, ObservableObject {
    @Published private(set) var summary = "Bluetooth scan not started"
    @Published private(set) var candidates: [BluetoothRNodeCandidate] = []
    @Published private(set) var online = false
    @Published private(set) var rxPackets = 0
    @Published private(set) var txPackets = 0
    @Published private(set) var hasApproval = false

    private static let serviceID = CBUUID(string: "6E400001-B5A3-F393-E0A9-E50E24DCCA9E")
    private static let writeID = CBUUID(string: "6E400002-B5A3-F393-E0A9-E50E24DCCA9E")
    private static let notifyID = CBUUID(string: "6E400003-B5A3-F393-E0A9-E50E24DCCA9E")
    private static let approvedPeripheralKey = "approvedRNodePeripheralID"

    private var central: CBCentralManager!
    private var discovered: [UUID: CBPeripheral] = [:]
    private var peripheral: CBPeripheral?
    private var writeCharacteristic: CBCharacteristic?
    private var notifyCharacteristic: CBCharacteristic?
    private weak var channel: (any RNodePacketChannel)?
    private let worker = DispatchQueue(label: "io.styrene.mesh.rnode")
    private var pollTimer: Timer?
    private var sessionTimeoutTimer: Timer?
    private var scanRequested = false
    private var reconnectingApproved = false
    private let writes = RNodeWriteQueue()
    private let outbound = RNodeOutboundRetention()
    private var pollInProgress = false
    private var decoder = RNodeKissDecoder()
    private var detected = false
    private var configured = false
    private var notificationsSecured = false
    private var preOnlineDisconnects = 0
    private var securityReadPending = false
    private var pairingErrorObserved = false
    private var notifyRequested = false
    private var securityFallback: DispatchWorkItem?
    private var firmware = "unknown"
    private var frequency: UInt32?
    private var bandwidth: UInt32?
    private var txPower: UInt8?
    private var spreadingFactor: UInt8?
    private var codingRate: UInt8?
    private var radioState: UInt8?

    override init() {
        super.init()
        hasApproval = approvedPeripheralID != nil
        central = CBCentralManager(delegate: self, queue: .main)
    }

    func attach(channel: (any RNodePacketChannel)?) {
        self.channel = channel
        outbound.attach(channel)
        guard channel != nil else {
            pollTimer?.invalidate()
            pollTimer = nil
            return
        }
        if online {
            startPacketPump()
        } else if peripheral == nil, approvedPeripheralID != nil {
            scan(reconnectingApproved: true)
        }
    }

    func scan() {
        scan(reconnectingApproved: false)
    }

    func approve(_ candidate: BluetoothRNodeCandidate) {
        guard let peripheral = discovered[candidate.id] else {
            summary = "RNode is no longer discoverable; scan again"
            return
        }
        UserDefaults.standard.set(candidate.id.uuidString, forKey: Self.approvedPeripheralKey)
        hasApproval = true
        preOnlineDisconnects = 0
        central.stopScan()
        connect(peripheral)
    }

    func disconnect() {
        pollTimer?.invalidate()
        pollTimer = nil
        if let peripheral { central.cancelPeripheralConnection(peripheral) }
        outbound.reset()
        resetSession()
        summary = "Bluetooth RNode disconnected"
    }

    func forgetApproval() {
        UserDefaults.standard.removeObject(forKey: Self.approvedPeripheralKey)
        hasApproval = false
        preOnlineDisconnects = 0
        disconnect()
        summary = "Bluetooth RNode approval removed"
    }

    private func scan(reconnectingApproved: Bool) {
        scanRequested = true
        self.reconnectingApproved = reconnectingApproved
        guard central.state == .poweredOn else {
            summary = central.state == .poweredOff ? "Turn on Bluetooth to find RNodes" : "Waiting for Bluetooth"
            return
        }
        discovered.removeAll()
        candidates = []
        summary = reconnectingApproved ? "Looking for the approved Bluetooth RNode" : "Scanning for Bluetooth RNodes"
        if reconnectingApproved, let approvedPeripheralID,
           let known = central.retrievePeripherals(withIdentifiers: [approvedPeripheralID]).first {
            connect(known)
            return
        }
        central.scanForPeripherals(withServices: [Self.serviceID], options: [CBCentralManagerScanOptionAllowDuplicatesKey: false])
    }

    private func connect(_ peripheral: CBPeripheral) {
        guard self.peripheral == nil else { return }
        central.stopScan()
        self.peripheral = peripheral
        peripheral.delegate = self
        summary = "Connecting to \(peripheral.name ?? "RNode")"
        startSetupTimeout(seconds: 45, message: "Bluetooth RNode connection or pairing timed out")
        central.connect(peripheral)
    }

    private func beginSession() {
        enqueue(command: 0x08, payload: [0x73])
        enqueue(command: 0x50, payload: [0])
        enqueue(command: 0x48, payload: [0])
        enqueue(command: 0x49, payload: [0])
        summary = "Detecting RNode over Bluetooth"
        startSetupTimeout(seconds: 6, message: "RNode detection or configuration timed out")
    }

    private func configureSession() {
        guard !configured else { return }
        configured = true
        summary = "Configuring RNode \(firmware) with 915 MHz SF7"
        enqueue(command: 0x01, payload: Self.uint32(915_000_000))
        enqueue(command: 0x02, payload: Self.uint32(125_000))
        enqueue(command: 0x03, payload: [17])
        enqueue(command: 0x04, payload: [7])
        enqueue(command: 0x05, payload: [5])
        enqueue(command: 0x06, payload: [1])
    }

    private func process(_ data: Data) {
        for frame in decoder.feed(data) {
            switch frame.command {
            case 0x00:
                guard !frame.payload.isEmpty, let channel else { continue }
                let packet = Data(frame.payload)
                worker.async { [weak self] in
                    do {
                        try channel.submitRnodePacket(packet)
                        DispatchQueue.main.async {
                            self?.rxPackets += 1
                        }
                    } catch {
                        DispatchQueue.main.async {
                            self?.summary = "RNode packet rejected: \(error.localizedDescription)"
                        }
                    }
                }
            case 0x08:
                detected = frame.payload.first == 0x46
                if detected { configureSession() }
            case 0x50 where frame.payload.count >= 2:
                firmware = "\(frame.payload[0]).\(frame.payload[1])"
            case 0x01: frequency = Self.readUInt32(frame.payload)
            case 0x02: bandwidth = Self.readUInt32(frame.payload)
            case 0x03: txPower = frame.payload.first
            case 0x04: spreadingFactor = frame.payload.first
            case 0x05: codingRate = frame.payload.first
            case 0x06: radioState = frame.payload.first
            default: break
            }
        }
        becomeOnlineIfConfigured()
    }

    private func becomeOnlineIfConfigured() {
        guard detected, !online,
              frequency == 915_000_000,
              bandwidth == 125_000,
              txPower == 17,
              spreadingFactor == 7,
              codingRate == 5,
              radioState == 1 else { return }
        online = true
        preOnlineDisconnects = 0
        sessionTimeoutTimer?.invalidate()
        sessionTimeoutTimer = nil
        securityFallback?.cancel()
        securityFallback = nil
        summary = "RNode \(firmware) online over Bluetooth / 915 MHz SF7"
        startPacketPump()
    }

    private func startPacketPump() {
        guard online, let channel else { return }
        worker.async { try? channel.announce() }
        guard pollTimer == nil else { return }
        pollTimer = Timer.scheduledTimer(withTimeInterval: 0.1, repeats: true) { [weak self] _ in
            self?.pollOutbound()
        }
    }

    private func pollOutbound() {
        guard online, let channel else { return }
        if let packet = outbound.packet {
            guard !outbound.enqueued else { return }
            outbound.markEnqueued()
            enqueue(command: 0x00, payload: [UInt8](packet), completesOutbound: true)
            return
        }
        guard !pollInProgress else { return }
        pollInProgress = true
        worker.async { [weak self] in
            guard let self else { return }
            let result = Result { try channel.pollRnodePacket() }
            DispatchQueue.main.async {
                self.pollInProgress = false
                guard self.channel === channel else { return }
                switch result {
                case .success(let packet?):
                    self.outbound.reserve(packet, for: channel)
                    if self.online { self.pollOutbound() }
                case .success(nil): break
                case .failure(let error):
                    self.summary = "RNode outbound poll failed: \(error.localizedDescription)"
                }
            }
        }
    }

    private func enqueue(command: UInt8, payload: [UInt8], completesOutbound: Bool = false) {
        guard let peripheral, writeCharacteristic != nil else { return }
        let framed = RNodeKissEncoder.frame(command: command, payload: payload)
        let chunkSize = peripheral.maximumWriteValueLength(for: .withResponse)
        guard chunkSize > 0 else {
            summary = "RNode reported an invalid Bluetooth write limit"
            return
        }
        guard writes.enqueue(
            Data(framed),
            chunkSize: chunkSize,
            completesOutbound: completesOutbound
        ) else {
            summary = "RNode Bluetooth write queue is at capacity"
            central.cancelPeripheralConnection(peripheral)
            return
        }
        writeNext()
    }

    private func writeNext() {
        guard let peripheral, let writeCharacteristic,
              let write = writes.startNext() else { return }
        peripheral.writeValue(write.data, for: writeCharacteristic, type: .withResponse)
    }

    private func resetSession() {
        pollTimer?.invalidate()
        pollTimer = nil
        sessionTimeoutTimer?.invalidate()
        sessionTimeoutTimer = nil
        peripheral = nil
        writeCharacteristic = nil
        notifyCharacteristic = nil
        writes.reset()
        outbound.markNotEnqueued()
        decoder = RNodeKissDecoder()
        detected = false
        configured = false
        notificationsSecured = false
        securityReadPending = false
        pairingErrorObserved = false
        notifyRequested = false
        online = false
        firmware = "unknown"
        frequency = nil
        bandwidth = nil
        txPower = nil
        spreadingFactor = nil
        codingRate = nil
        radioState = nil
    }

    private func startSetupTimeout(seconds: TimeInterval, message: String) {
        sessionTimeoutTimer?.invalidate()
        sessionTimeoutTimer = Timer.scheduledTimer(withTimeInterval: seconds, repeats: false) { [weak self] _ in
            guard let self, !self.online, let peripheral = self.peripheral else { return }
            self.summary = message
            self.central.cancelPeripheralConnection(peripheral)
        }
    }

    private func isPairingError(_ error: Error) -> Bool {
        let error = error as NSError
        guard error.domain == CBATTErrorDomain else { return false }
        return error.code == CBATTError.Code.insufficientAuthentication.rawValue ||
            error.code == CBATTError.Code.insufficientEncryption.rawValue
    }

    private func beginSecurity(_ peripheral: CBPeripheral, notifyCharacteristic: CBCharacteristic) {
        pairingErrorObserved = false
        if notifyCharacteristic.properties.contains(.read) {
            securityReadPending = true
            summary = "Starting secure RNode pairing"
            peripheral.readValue(for: notifyCharacteristic)
            let fallback = DispatchWorkItem { [weak self, weak peripheral, weak notifyCharacteristic] in
                guard let self, let peripheral, let notifyCharacteristic,
                      peripheral === self.peripheral,
                      self.securityReadPending,
                      !self.pairingErrorObserved else { return }
                self.securityReadPending = false
                self.requestNotifications(peripheral, characteristic: notifyCharacteristic)
            }
            securityFallback = fallback
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.5, execute: fallback)
        } else {
            requestNotifications(peripheral, characteristic: notifyCharacteristic)
        }
    }

    private func requestNotifications(_ peripheral: CBPeripheral, characteristic: CBCharacteristic) {
        guard !notifyRequested else { return }
        notifyRequested = true
        peripheral.setNotifyValue(true, for: characteristic)
    }

    private var approvedPeripheralID: UUID? {
        UserDefaults.standard.string(forKey: Self.approvedPeripheralKey).flatMap(UUID.init(uuidString:))
    }

    static func uint32(_ value: UInt32) -> [UInt8] {
        [
            UInt8(truncatingIfNeeded: value >> 24),
            UInt8(truncatingIfNeeded: value >> 16),
            UInt8(truncatingIfNeeded: value >> 8),
            UInt8(truncatingIfNeeded: value),
        ]
    }

    private static func readUInt32(_ bytes: [UInt8]) -> UInt32? {
        guard bytes.count == 4 else { return nil }
        return bytes.reduce(0) { ($0 << 8) | UInt32($1) }
    }
}

extension RNodeBluetoothController: CBCentralManagerDelegate {
    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        if central.state == .poweredOn, scanRequested {
            scan(reconnectingApproved: reconnectingApproved)
        } else if central.state == .poweredOff {
            summary = "Turn on Bluetooth to find RNodes"
        }
    }

    func centralManager(
        _ central: CBCentralManager,
        didDiscover peripheral: CBPeripheral,
        advertisementData: [String: Any],
        rssi RSSI: NSNumber
    ) {
        if peripheral.identifier == approvedPeripheralID {
            connect(peripheral)
            return
        }
        discovered[peripheral.identifier] = peripheral
        candidates = discovered.values
            .map { BluetoothRNodeCandidate(id: $0.identifier, name: $0.name ?? "RNode") }
            .sorted { $0.name < $1.name }
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        summary = "Discovering RNode Bluetooth service"
        peripheral.discoverServices([Self.serviceID])
    }

    func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?) {
        let shouldReconnect = channel != nil && peripheral.identifier == approvedPeripheralID
        resetSession()
        summary = "Bluetooth RNode connection failed: \(error?.localizedDescription ?? "unknown error")"
        if shouldReconnect, preOnlineDisconnects == 0 {
            preOnlineDisconnects = 1
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self, weak peripheral] in
                guard let self, let peripheral else { return }
                self.connect(peripheral)
            }
        } else if shouldReconnect {
            summary = "Approved RNode remains unavailable. Tap Scan to retry."
        }
    }

    func centralManager(_ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral, error: Error?) {
        let shouldReconnect = channel != nil && peripheral.identifier == approvedPeripheralID
        let completedSecurity = notificationsSecured
        let wasOnline = online
        let pairingWasRejected = pairingErrorObserved
        resetSession()
        summary = error.map { "Bluetooth RNode disconnected: \($0.localizedDescription)" } ?? "Bluetooth RNode disconnected"
        guard shouldReconnect else { return }
        if pairingWasRejected {
            summary = "Secure pairing was rejected. Confirm the RNode itself shows its six-digit pairing PIN, then tap Connect."
        } else if wasOnline || completedSecurity {
            preOnlineDisconnects = 0
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self, weak peripheral] in
                guard let self, let peripheral else { return }
                self.connect(peripheral)
            }
        } else if preOnlineDisconnects == 0 {
            preOnlineDisconnects = 1
            summary = "RNode disconnected during pairing; checking the new bond once"
            DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self, weak peripheral] in
                guard let self, let peripheral else { return }
                self.connect(peripheral)
            }
        } else {
            summary = "Pairing was rejected. Forget the RNode, clear its old bond, and enter pairing mode again."
        }
    }

}

extension RNodeBluetoothController: CBPeripheralDelegate {
    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        guard peripheral === self.peripheral else { return }
        guard error == nil,
              let service = peripheral.services?.first(where: { $0.uuid == Self.serviceID }) else {
            summary = "RNode Nordic UART service is unavailable"
            central.cancelPeripheralConnection(peripheral)
            return
        }
        peripheral.discoverCharacteristics([Self.writeID, Self.notifyID], for: service)
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        guard peripheral === self.peripheral else { return }
        guard error == nil else {
            summary = "RNode Bluetooth characteristic discovery failed"
            central.cancelPeripheralConnection(peripheral)
            return
        }
        writeCharacteristic = service.characteristics?.first(where: { $0.uuid == Self.writeID })
        notifyCharacteristic = service.characteristics?.first(where: { $0.uuid == Self.notifyID })
        guard let writeCharacteristic, writeCharacteristic.properties.contains(.write),
              let notifyCharacteristic,
              notifyCharacteristic.properties.contains(.notify) ||
                  notifyCharacteristic.properties.contains(.indicate) else {
            summary = "RNode Bluetooth UART characteristics are incompatible"
            central.cancelPeripheralConnection(peripheral)
            return
        }
        beginSecurity(peripheral, notifyCharacteristic: notifyCharacteristic)
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic, error: Error?) {
        guard peripheral === self.peripheral else { return }
        if let error, isPairingError(error) {
            summary = "Complete the RNode pairing request on this iPhone"
            return
        }
        guard error == nil, characteristic.isNotifying else {
            summary = "RNode Bluetooth notifications could not be enabled"
            central.cancelPeripheralConnection(peripheral)
            return
        }
        notificationsSecured = true
        beginSession()
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        guard peripheral === self.peripheral else { return }
        if securityReadPending, characteristic === notifyCharacteristic {
            securityReadPending = false
            securityFallback?.cancel()
            securityFallback = nil
            if let error, isPairingError(error) {
                pairingErrorObserved = true
                summary = "Waiting for the RNode to accept secure pairing"
                return
            }
            requestNotifications(peripheral, characteristic: characteristic)
            return
        }
        guard error == nil, characteristic.uuid == Self.notifyID, let value = characteristic.value else { return }
        process(value)
    }

    func peripheral(_ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: Error?) {
        guard peripheral === self.peripheral, characteristic === writeCharacteristic else { return }
        if let error, isPairingError(error) {
            writes.finishActive(retry: true)
            summary = "Complete the RNode pairing request on this iPhone"
            return
        }
        let completedWrite = writes.finishActive(retry: false)
        if let error {
            summary = "RNode Bluetooth write failed: \(error.localizedDescription)"
            central.cancelPeripheralConnection(peripheral)
            return
        }
        if completedWrite?.completesOutbound == true {
            outbound.acknowledge()
            txPackets += 1
        }
        writeNext()
        if completedWrite?.completesOutbound == true { pollOutbound() }
    }
}

struct RNodeQueuedWrite {
    let data: Data
    let completesOutbound: Bool
}

final class RNodeWriteQueue {
    static let maximumPendingChunks = 64

    private var pending: [RNodeQueuedWrite] = []
    private(set) var active: RNodeQueuedWrite?

    var pendingCount: Int { pending.count }

    func enqueue(_ data: Data, chunkSize: Int, completesOutbound: Bool) -> Bool {
        guard chunkSize > 0, !data.isEmpty else { return false }
        let chunkCount = (data.count + chunkSize - 1) / chunkSize
        guard pending.count + chunkCount <= Self.maximumPendingChunks else { return false }
        var offset = 0
        while offset < data.count {
            let end = min(offset + chunkSize, data.count)
            pending.append(
                RNodeQueuedWrite(
                    data: Data(data[offset..<end]),
                    completesOutbound: completesOutbound && end == data.count
                )
            )
            offset = end
        }
        return true
    }

    func startNext() -> RNodeQueuedWrite? {
        guard active == nil, !pending.isEmpty else { return nil }
        active = pending.removeFirst()
        return active
    }

    @discardableResult
    func finishActive(retry: Bool) -> RNodeQueuedWrite? {
        let completed = active
        active = nil
        if retry, let completed { pending.insert(completed, at: 0) }
        return completed
    }

    func reset() {
        pending.removeAll(keepingCapacity: true)
        active = nil
    }
}

struct RNodeKissFrame {
    let command: UInt8
    let payload: [UInt8]
}

enum RNodeKissEncoder {
    static func frame(command: UInt8, payload: [UInt8]) -> [UInt8] {
        var output: [UInt8] = [0xC0, command]
        for byte in payload {
            switch byte {
            case 0xC0: output.append(contentsOf: [0xDB, 0xDC])
            case 0xDB: output.append(contentsOf: [0xDB, 0xDD])
            default: output.append(byte)
            }
        }
        output.append(0xC0)
        return output
    }
}

struct RNodeKissDecoder {
    static let maximumFrameBytes = 1_024

    private var buffer: [UInt8] = []
    private var inFrame = false
    private var escaped = false
    private var discarding = false

    mutating func feed(_ data: Data) -> [RNodeKissFrame] {
        var frames: [RNodeKissFrame] = []
        for byte in data {
            if escaped {
                escaped = false
                switch byte {
                case 0xDC: append(0xC0)
                case 0xDD: append(0xDB)
                default: discardFrame()
                }
                continue
            }
            switch byte {
            case 0xC0:
                if inFrame, !discarding, let command = buffer.first {
                    frames.append(RNodeKissFrame(command: command, payload: Array(buffer.dropFirst())))
                }
                buffer.removeAll(keepingCapacity: true)
                inFrame = true
                discarding = false
            case 0xDB where inFrame && !discarding: escaped = true
            default:
                if inFrame, !discarding { append(byte) }
            }
        }
        return frames
    }

    private mutating func append(_ byte: UInt8) {
        buffer.append(byte)
        if buffer.count > Self.maximumFrameBytes { discardFrame() }
    }

    private mutating func discardFrame() {
        buffer.removeAll(keepingCapacity: true)
        escaped = false
        discarding = true
    }
}

final class RNodeOutboundRetention {
    private weak var owner: (any RNodePacketChannel)?
    private(set) var packet: Data?
    private(set) var enqueued = false

    func attach(_ channel: (any RNodePacketChannel)?) {
        guard let channel, packet != nil else { return }
        if owner !== channel { reset() }
    }

    func reserve(_ packet: Data, for channel: any RNodePacketChannel) {
        self.packet = packet
        owner = channel
        enqueued = false
    }

    func markEnqueued() {
        enqueued = true
    }

    func markNotEnqueued() {
        enqueued = false
    }

    func acknowledge() {
        reset()
    }

    func reset() {
        packet = nil
        owner = nil
        enqueued = false
    }
}
