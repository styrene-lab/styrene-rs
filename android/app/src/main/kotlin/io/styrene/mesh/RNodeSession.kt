package io.styrene.mesh

import android.util.Log
import java.security.MessageDigest

data class RNodeRadioProfile(
    val frequencyHz: Long,
    val bandwidthHz: Long,
    val txPowerDbm: Int,
    val spreadingFactor: Int,
    val codingRate: Int,
    val label: String,
) {
    companion object {
        val US_915_DEVELOPMENT = RNodeRadioProfile(
            frequencyHz = 915_000_000L,
            bandwidthHz = 125_000L,
            txPowerDbm = 17,
            spreadingFactor = 7,
            codingRate = 5,
            label = "915 MHz SF7",
        )
    }
}

class RNodeSession(
    private val node: RNodePacketChannel,
    private val outbound: RNodeOutboundBuffer,
    private val profile: RNodeRadioProfile,
    private val onState: (message: String, online: Boolean) -> Unit,
    private val onTraffic: (rxPackets: Long, txPackets: Long) -> Unit,
    private val commandDelay: (Long) -> Unit = Thread::sleep,
    private val logInfo: (String) -> Unit = { Log.i(TAG, it) },
    private val logWarning: (String, Throwable) -> Unit = { message, error -> Log.w(TAG, message, error) },
) {
    private val decoder = RNodeProtocol.Decoder()
    private var detected = false
    private var firmware = "unknown"
    private var frequency: Long? = null
    private var bandwidth: Long? = null
    private var txPower: Int? = null
    private var spreadingFactor: Int? = null
    private var codingRate: Int? = null
    private var radioState: Int? = null
    private var rxPackets = 0L
    private var txPackets = 0L

    fun initialize(link: RNodeByteLink, isRunning: () -> Boolean) {
        onState("Detecting RNode over ${link.bearerName}", false)
        write(link, RNodeProtocol.CMD_DETECT, byteArrayOf(RNodeProtocol.DETECT_REQUEST.toByte()))
        write(link, RNodeProtocol.CMD_FW_VERSION, byteArrayOf(0))
        write(link, RNodeProtocol.CMD_PLATFORM, byteArrayOf(0))
        write(link, RNodeProtocol.CMD_MCU, byteArrayOf(0))
        readUntil(link, DETECT_TIMEOUT_MS, isRunning) { detected }
        check(detected) { "RNode detect timed out" }

        onState("Configuring RNode $firmware with ${profile.label}", false)
        writeConfig(link, RNodeProtocol.CMD_FREQUENCY, RNodeProtocol.unsignedInt(profile.frequencyHz))
        writeConfig(link, RNodeProtocol.CMD_BANDWIDTH, RNodeProtocol.unsignedInt(profile.bandwidthHz))
        writeConfig(link, RNodeProtocol.CMD_TX_POWER, byteArrayOf(profile.txPowerDbm.toByte()))
        writeConfig(link, RNodeProtocol.CMD_SF, byteArrayOf(profile.spreadingFactor.toByte()))
        writeConfig(link, RNodeProtocol.CMD_CR, byteArrayOf(profile.codingRate.toByte()))
        writeConfig(link, RNodeProtocol.CMD_RADIO_STATE, byteArrayOf(RNodeProtocol.RADIO_ON.toByte()))

        readUntil(link, CONFIG_TIMEOUT_MS, isRunning, ::configurationMatches)
        check(configurationMatches()) {
            "RNode rejected radio configuration: ${configurationSummary()}"
        }

        onState("RNode $firmware online over ${link.bearerName} / ${profile.label}", true)
        node.announce()
    }

    fun pumpOnce(link: RNodeByteLink) {
        readAvailable(link)
        repeat(MAX_OUTBOUND_BATCH) {
            val packet = outbound.next() ?: return
            logInfo("tx_packet len=${packet.size} sha256=${packet.sha256()}")
            write(link, RNodeProtocol.CMD_DATA, packet)
            outbound.acknowledge(packet)
            txPackets += 1
            onTraffic(rxPackets, txPackets)
        }
    }

    fun shutdown(link: RNodeByteLink) {
        write(
            link,
            RNodeProtocol.CMD_RADIO_STATE,
            byteArrayOf(RNodeProtocol.RADIO_OFF.toByte()),
        )
    }

    private fun writeConfig(link: RNodeByteLink, command: Int, payload: ByteArray) {
        write(link, command, payload)
        commandDelay(CONFIG_COMMAND_DELAY_MS)
        readAvailable(link)
    }

    private fun write(link: RNodeByteLink, command: Int, payload: ByteArray) {
        link.write(RNodeProtocol.frame(command, payload), WRITE_TIMEOUT_MS)
    }

    private fun readUntil(
        link: RNodeByteLink,
        timeoutMs: Long,
        isRunning: () -> Boolean,
        condition: () -> Boolean,
    ) {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (isRunning() && !condition() && System.currentTimeMillis() < deadline) {
            readAvailable(link)
        }
    }

    private fun readAvailable(link: RNodeByteLink) {
        val input = ByteArray(READ_BUFFER_BYTES)
        val count = link.read(input, READ_TIMEOUT_MS)
        if (count > 0) process(input, count)
    }

    private fun process(input: ByteArray, length: Int) {
        decoder.feed(input, length).forEach { frame ->
            when (frame.command) {
                RNodeProtocol.CMD_DATA -> processPacket(frame.payload)
                RNodeProtocol.CMD_DETECT -> {
                    detected = frame.payload.firstOrNull()?.toInt()?.and(0xff) ==
                        RNodeProtocol.DETECT_RESPONSE
                }
                RNodeProtocol.CMD_FW_VERSION -> if (frame.payload.size >= 2) {
                    firmware = "${frame.payload[0].toInt() and 0xff}.${frame.payload[1].toInt() and 0xff}"
                }
                RNodeProtocol.CMD_FREQUENCY -> if (frame.payload.size == 4) {
                    frequency = RNodeProtocol.readUnsignedInt(frame.payload)
                }
                RNodeProtocol.CMD_BANDWIDTH -> if (frame.payload.size == 4) {
                    bandwidth = RNodeProtocol.readUnsignedInt(frame.payload)
                }
                RNodeProtocol.CMD_TX_POWER -> txPower = frame.payload.firstUnsigned()
                RNodeProtocol.CMD_SF -> spreadingFactor = frame.payload.firstUnsigned()
                RNodeProtocol.CMD_CR -> codingRate = frame.payload.firstUnsigned()
                RNodeProtocol.CMD_RADIO_STATE -> radioState = frame.payload.firstUnsigned()
            }
        }
    }

    private fun processPacket(packet: ByteArray) {
        if (packet.isEmpty()) return
        logInfo("rx_packet len=${packet.size} sha256=${packet.sha256()}")
        runCatching { node.submit(packet) }
            .onSuccess {
                rxPackets += 1
                onTraffic(rxPackets, txPackets)
            }
            .onFailure { error ->
                if (error.message?.contains("invalid RNS packet") != true) throw error
                logWarning("Dropping malformed RNS packet", error)
                onState("RNode online / dropped ${packet.size}-byte invalid packet", true)
            }
    }

    private fun configurationMatches() =
        frequency == profile.frequencyHz &&
            bandwidth == profile.bandwidthHz &&
            txPower == profile.txPowerDbm &&
            spreadingFactor == profile.spreadingFactor &&
            codingRate == profile.codingRate &&
            radioState == RNodeProtocol.RADIO_ON

    private fun configurationSummary() =
        "freq=$frequency bw=$bandwidth tx=$txPower sf=$spreadingFactor cr=$codingRate state=$radioState"

    private fun ByteArray.firstUnsigned() = firstOrNull()?.toInt()?.and(0xff)

    private fun ByteArray.sha256() = MessageDigest.getInstance("SHA-256")
        .digest(this)
        .joinToString("") { byte -> "%02x".format(byte) }

    companion object {
        private const val TAG = "StyreneRNode"
        private const val READ_BUFFER_BYTES = 2_048
        private const val READ_TIMEOUT_MS = 100
        private const val WRITE_TIMEOUT_MS = 1_000
        private const val DETECT_TIMEOUT_MS = 3_000L
        private const val CONFIG_TIMEOUT_MS = 3_000L
        private const val CONFIG_COMMAND_DELAY_MS = 150L
        private const val MAX_OUTBOUND_BATCH = 16
    }
}

class RNodeOutboundBuffer(private val node: RNodePacketChannel) {
    private var pending: ByteArray? = null

    @Synchronized
    fun next(): ByteArray? {
        pending?.let { return it }
        return node.poll()?.also { pending = it }
    }

    @Synchronized
    fun acknowledge(packet: ByteArray) {
        check(pending === packet) { "RNode outbound acknowledgement does not match pending packet" }
        pending = null
    }
}
