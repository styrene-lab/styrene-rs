package io.styrene.mesh

import java.util.ArrayDeque
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class RNodeSessionTest {
    @Test
    fun initializesAndPumpsPacketsOverBearerNeutralLink() {
        val node = FakePacketChannel().apply {
            outbound += byteArrayOf(0x21, 0x22)
        }
        val link = FakeRNodeLink()
        val states = mutableListOf<Pair<String, Boolean>>()
        val traffic = mutableListOf<Pair<Long, Long>>()
        val session = RNodeSession(
            node = node,
            outbound = RNodeOutboundBuffer(node),
            profile = RNodeRadioProfile.US_915_DEVELOPMENT,
            onState = { message, online -> states += message to online },
            onTraffic = { rx, tx -> traffic += rx to tx },
            commandDelay = {},
            logInfo = {},
            logWarning = { _, _ -> },
        )

        session.initialize(link) { true }
        link.enqueue(RNodeProtocol.CMD_DATA, byteArrayOf(0x11, 0x12))
        session.pumpOnce(link)

        assertEquals(1, node.announces)
        assertContentEquals(byteArrayOf(0x11, 0x12), node.submitted.single())
        assertTrue(states.last().first.contains("over test / 915 MHz SF7"))
        assertEquals(true, states.last().second)
        assertEquals(1L to 1L, traffic.last())
        assertTrue(link.commands.any { it.command == RNodeProtocol.CMD_DATA })
        assertContentEquals(
            byteArrayOf(0x21, 0x22),
            link.commands.last { it.command == RNodeProtocol.CMD_DATA }.payload,
        )
    }

    @Test
    fun shutdownTurnsRadioOff() {
        val node = FakePacketChannel()
        val session = RNodeSession(
            node = node,
            outbound = RNodeOutboundBuffer(node),
            profile = RNodeRadioProfile.US_915_DEVELOPMENT,
            onState = { _, _ -> },
            onTraffic = { _, _ -> },
            commandDelay = {},
            logInfo = {},
            logWarning = { _, _ -> },
        )
        val link = FakeRNodeLink()

        session.shutdown(link)

        val command = link.commands.single()
        assertEquals(RNodeProtocol.CMD_RADIO_STATE, command.command)
        assertContentEquals(byteArrayOf(RNodeProtocol.RADIO_OFF.toByte()), command.payload)
    }

    @Test
    fun outboundBufferRetainsPacketUntilWriteIsAcknowledged() {
        val firstPacket = byteArrayOf(0x31, 0x32)
        val secondPacket = byteArrayOf(0x41, 0x42)
        val node = FakePacketChannel().apply {
            outbound += firstPacket
            outbound += secondPacket
        }
        val buffer = RNodeOutboundBuffer(node)

        val firstAttempt = buffer.next()
        val retry = buffer.next()

        assertTrue(firstAttempt === retry)
        buffer.acknowledge(requireNotNull(retry))
        assertTrue(buffer.next() === secondPacket)
        buffer.acknowledge(secondPacket)
        assertEquals(null, buffer.next())
    }

    @Test
    fun failedWriteIsRetainedForReplacementSession() {
        val packet = byteArrayOf(0x51, 0x52)
        val node = FakePacketChannel().apply { outbound += packet }
        val outbound = RNodeOutboundBuffer(node)
        val failedSession = session(node, outbound)

        runCatching { failedSession.pumpOnce(FakeRNodeLink(failDataWrites = true)) }
            .onSuccess { error("Data write should fail") }

        val replacementLink = FakeRNodeLink()
        session(node, outbound).pumpOnce(replacementLink)

        assertContentEquals(
            packet,
            replacementLink.commands.single { it.command == RNodeProtocol.CMD_DATA }.payload,
        )
        assertEquals(null, outbound.next())
    }

    private fun session(node: RNodePacketChannel, outbound: RNodeOutboundBuffer) = RNodeSession(
        node = node,
        outbound = outbound,
        profile = RNodeRadioProfile.US_915_DEVELOPMENT,
        onState = { _, _ -> },
        onTraffic = { _, _ -> },
        commandDelay = {},
        logInfo = {},
        logWarning = { _, _ -> },
    )

    private class FakePacketChannel : RNodePacketChannel {
        var announces = 0
        val submitted = mutableListOf<ByteArray>()
        val outbound = ArrayDeque<ByteArray>()

        override fun announce() {
            announces += 1
        }

        override fun submit(packet: ByteArray) {
            submitted += packet
        }

        override fun poll(): ByteArray? = outbound.pollFirst()
    }

    private class FakeRNodeLink(private val failDataWrites: Boolean = false) : RNodeByteLink {
        override val bearerName = "test"
        val commands = mutableListOf<RNodeFrame>()
        private val reads = ArrayDeque<ByteArray>()

        override fun read(buffer: ByteArray, timeoutMs: Int): Int {
            val next = reads.pollFirst() ?: return 0
            next.copyInto(buffer)
            return next.size
        }

        override fun write(data: ByteArray, timeoutMs: Int) {
            val command = RNodeProtocol.Decoder().feed(data).single()
            if (failDataWrites && command.command == RNodeProtocol.CMD_DATA) error("write failed")
            commands += command
            when (command.command) {
                RNodeProtocol.CMD_DETECT -> enqueue(
                    RNodeProtocol.CMD_DETECT,
                    byteArrayOf(RNodeProtocol.DETECT_RESPONSE.toByte()),
                )
                RNodeProtocol.CMD_FW_VERSION -> enqueue(RNodeProtocol.CMD_FW_VERSION, byteArrayOf(1, 74))
                RNodeProtocol.CMD_FREQUENCY,
                RNodeProtocol.CMD_BANDWIDTH,
                RNodeProtocol.CMD_TX_POWER,
                RNodeProtocol.CMD_SF,
                RNodeProtocol.CMD_CR,
                RNodeProtocol.CMD_RADIO_STATE,
                -> enqueue(command.command, command.payload)
            }
        }

        fun enqueue(command: Int, payload: ByteArray) {
            reads += RNodeProtocol.frame(command, payload)
        }

        override fun close() = Unit
    }
}
