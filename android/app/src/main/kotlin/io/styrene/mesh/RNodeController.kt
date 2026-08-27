package io.styrene.mesh

import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.util.Log
import com.hoho.android.usbserial.driver.UsbSerialPort
import com.hoho.android.usbserial.driver.UsbSerialProber
import java.security.MessageDigest
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread
import uniffi.styrene_mobile_ffi.MobileNode

class RNodeController(
    private val usbManager: UsbManager,
    private val node: MobileNode,
    private val listener: Listener,
) {
    interface Listener {
        fun onState(message: String, online: Boolean)
        fun onTraffic(rxPackets: Long, txPackets: Long)
    }

    private val running = AtomicBoolean(false)
    private val decoder = RNodeProtocol.Decoder()
    private var port: UsbSerialPort? = null
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

    fun start(device: UsbDevice) {
        if (!running.compareAndSet(false, true)) return
        thread(name = "styrene-rnode", isDaemon = true) {
            runCatching { run(device) }
                .onFailure { error ->
                    listener.onState("RNode error: ${error.message}", false)
                    closePort()
                }
        }
    }

    fun stop() {
        if (!running.getAndSet(false)) return
        runCatching {
            port?.write(
                RNodeProtocol.frame(
                    RNodeProtocol.CMD_RADIO_STATE,
                    byteArrayOf(RNodeProtocol.RADIO_OFF.toByte()),
                ),
                WRITE_TIMEOUT_MS,
            )
        }
        closePort()
    }

    private fun run(device: UsbDevice) {
        listener.onState("Opening Heltec serial port", false)
        val driver = UsbSerialProber.getDefaultProber().findAllDrivers(usbManager)
            .firstOrNull { it.device.deviceId == device.deviceId }
            ?: error("no CP2102 serial driver")
        val connection = usbManager.openDevice(driver.device) ?: error("USB permission unavailable")
        val serialPort = driver.ports.firstOrNull() ?: error("USB device has no serial port")
        port = serialPort
        serialPort.open(connection)
        serialPort.setParameters(
            BAUD_RATE,
            8,
            UsbSerialPort.STOPBITS_1,
            UsbSerialPort.PARITY_NONE,
        )

        listener.onState("Detecting RNode", false)
        write(RNodeProtocol.CMD_DETECT, byteArrayOf(RNodeProtocol.DETECT_REQUEST.toByte()))
        write(RNodeProtocol.CMD_FW_VERSION, byteArrayOf(0))
        write(RNodeProtocol.CMD_PLATFORM, byteArrayOf(0))
        write(RNodeProtocol.CMD_MCU, byteArrayOf(0))
        readUntil(DETECT_TIMEOUT_MS) { detected }
        check(detected) { "RNode detect timed out" }

        listener.onState("Configuring RNode $firmware", false)
        writeConfig(RNodeProtocol.CMD_FREQUENCY, RNodeProtocol.unsignedInt(FREQUENCY_HZ))
        writeConfig(RNodeProtocol.CMD_BANDWIDTH, RNodeProtocol.unsignedInt(BANDWIDTH_HZ))
        writeConfig(RNodeProtocol.CMD_TX_POWER, byteArrayOf(TX_POWER_DBM.toByte()))
        writeConfig(RNodeProtocol.CMD_SF, byteArrayOf(SPREADING_FACTOR.toByte()))
        writeConfig(RNodeProtocol.CMD_CR, byteArrayOf(CODING_RATE.toByte()))
        writeConfig(RNodeProtocol.CMD_RADIO_STATE, byteArrayOf(RNodeProtocol.RADIO_ON.toByte()))

        readUntil(CONFIG_TIMEOUT_MS) { configurationMatches() }
        check(configurationMatches()) {
            "RNode rejected radio configuration: ${configurationSummary()}"
        }

        listener.onState("RNode $firmware online / 915 MHz SF7", true)
        node.announce()

        val input = ByteArray(2048)
        while (running.get()) {
            val count = serialPort.read(input, READ_TIMEOUT_MS)
            if (count > 0) process(input, count)
            drainOutbound()
        }
    }

    private fun writeConfig(command: Int, payload: ByteArray) {
        write(command, payload)
        Thread.sleep(CONFIG_COMMAND_DELAY_MS)
        readAvailable()
    }

    private fun write(command: Int, payload: ByteArray) {
        val data = RNodeProtocol.frame(command, payload)
        val serialPort = port ?: error("serial port closed")
        serialPort.write(data, WRITE_TIMEOUT_MS)
    }

    private fun readUntil(timeoutMs: Long, condition: () -> Boolean) {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (running.get() && !condition() && System.currentTimeMillis() < deadline) {
            readAvailable()
        }
    }

    private fun readAvailable() {
        val input = ByteArray(2048)
        val count = port?.read(input, READ_TIMEOUT_MS) ?: return
        if (count > 0) process(input, count)
    }

    private fun process(input: ByteArray, length: Int) {
        decoder.feed(input, length).forEach { frame ->
            when (frame.command) {
                RNodeProtocol.CMD_DATA -> {
                    if (frame.payload.isNotEmpty()) {
                        Log.i(TAG, "rx_packet len=${frame.payload.size} sha256=${frame.payload.sha256()}")
                        runCatching { node.submitRnodePacket(frame.payload) }
                            .onSuccess {
                                rxPackets += 1
                                listener.onTraffic(rxPackets, txPackets)
                            }
                            .onFailure { error ->
                                if (error.message?.contains("invalid RNS packet") != true) {
                                    throw error
                                }
                                Log.w(TAG, "Dropping malformed RNS packet", error)
                                listener.onState(
                                    "RNode online / dropped ${frame.payload.size}-byte invalid packet",
                                    true,
                                )
                            }
                    }
                }
                RNodeProtocol.CMD_DETECT -> {
                    detected = frame.payload.firstOrNull()?.toInt()?.and(0xff) ==
                        RNodeProtocol.DETECT_RESPONSE
                }
                RNodeProtocol.CMD_FW_VERSION -> if (frame.payload.size >= 2) {
                    firmware = "${frame.payload[0].toInt() and 0xff}." +
                        "${frame.payload[1].toInt() and 0xff}"
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

    private fun drainOutbound() {
        repeat(MAX_OUTBOUND_BATCH) {
            val packet = node.pollRnodePacket() ?: return
            Log.i(TAG, "tx_packet len=${packet.size} sha256=${packet.sha256()}")
            write(RNodeProtocol.CMD_DATA, packet)
            txPackets += 1
            listener.onTraffic(rxPackets, txPackets)
        }
    }

    private fun configurationMatches() =
        frequency == FREQUENCY_HZ &&
            bandwidth == BANDWIDTH_HZ &&
            txPower == TX_POWER_DBM &&
            spreadingFactor == SPREADING_FACTOR &&
            codingRate == CODING_RATE &&
            radioState == RNodeProtocol.RADIO_ON

    private fun configurationSummary() =
        "freq=$frequency bw=$bandwidth tx=$txPower sf=$spreadingFactor cr=$codingRate state=$radioState"

    private fun ByteArray.firstUnsigned() = firstOrNull()?.toInt()?.and(0xff)

    private fun ByteArray.sha256() = MessageDigest.getInstance("SHA-256")
        .digest(this)
        .joinToString("") { byte -> "%02x".format(byte) }

    private fun closePort() {
        running.set(false)
        runCatching { port?.close() }
        port = null
    }

    companion object {
        private const val TAG = "StyreneRNode"
        private const val BAUD_RATE = 115_200
        private const val FREQUENCY_HZ = 915_000_000L
        private const val BANDWIDTH_HZ = 125_000L
        private const val TX_POWER_DBM = 17
        private const val SPREADING_FACTOR = 7
        private const val CODING_RATE = 5
        private const val READ_TIMEOUT_MS = 100
        private const val WRITE_TIMEOUT_MS = 1_000
        private const val DETECT_TIMEOUT_MS = 3_000L
        private const val CONFIG_TIMEOUT_MS = 3_000L
        private const val CONFIG_COMMAND_DELAY_MS = 150L
        private const val MAX_OUTBOUND_BATCH = 16
    }
}
