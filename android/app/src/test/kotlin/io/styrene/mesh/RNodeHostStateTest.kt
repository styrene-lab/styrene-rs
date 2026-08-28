package io.styrene.mesh

import java.util.ArrayDeque
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertSame
import kotlin.test.assertTrue

class RNodeHostStateTest {
    @Test
    fun unknownAdvertisementsNeverAutoConnect() {
        val approval = RNodeApprovalPolicy(null)

        assertFalse(approval.shouldReconnect("unknown", bonded = false))
        assertFalse(approval.shouldReconnect("unknown", bonded = true))
    }

    @Test
    fun onlyApprovedBondedDeviceReconnects() {
        val approval = RNodeApprovalPolicy("approved")

        assertFalse(approval.shouldReconnect("other", bonded = true))
        assertFalse(approval.shouldReconnect("approved", bonded = false))
        assertTrue(approval.shouldReconnect("approved", bonded = true))
    }

    @Test
    fun bearerDetachDoesNotClearApprovalAndOnlyForgetDoes() {
        val approval = RNodeApprovalPolicy("approved")
        val bearerState = RNodeBearerState()

        bearerState.clear()

        assertTrue(approval.shouldReconnect("approved", bonded = true))
        approval.forget()
        assertFalse(approval.shouldReconnect("approved", bonded = true))
    }

    @Test
    fun sameChannelRetainsPendingPacketAcrossHostRecreation() {
        val packet = byteArrayOf(0x11)
        val channel = FakePacketChannel(packet)
        val state = RNodeBearerState()
        val firstHostBuffer = state.outboundBuffer(channel)

        assertSame(packet, firstHostBuffer.next())

        val replacementHostBuffer = state.outboundBuffer(channel)
        assertSame(firstHostBuffer, replacementHostBuffer)
        assertSame(packet, replacementHostBuffer.next())
    }

    @Test
    fun replacingChannelDiscardsOldRetainedState() {
        val oldPacket = byteArrayOf(0x21)
        val replacementPacket = byteArrayOf(0x31)
        val state = RNodeBearerState()
        val oldBuffer = state.outboundBuffer(FakePacketChannel(oldPacket))
        assertSame(oldPacket, oldBuffer.next())

        val replacementBuffer = state.outboundBuffer(FakePacketChannel(replacementPacket))

        assertSame(replacementPacket, replacementBuffer.next())
        replacementBuffer.acknowledge(replacementPacket)
        assertNull(replacementBuffer.next())
    }

    private class FakePacketChannel(vararg packets: ByteArray) : RNodePacketChannel {
        private val outbound = ArrayDeque(packets.toList())

        override fun announce() = Unit
        override fun submit(packet: ByteArray) = Unit
        override fun poll(): ByteArray? = outbound.pollFirst()
    }
}
