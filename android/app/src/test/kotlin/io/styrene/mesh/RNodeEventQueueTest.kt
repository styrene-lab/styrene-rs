package io.styrene.mesh

import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertNull

class RNodeEventQueueTest {
    @Test
    fun preservesEventsInOrderWithinCapacity() {
        val queue = RNodeEventQueue<Int>(2) { -1 }

        queue.publish(1)
        queue.publish(2)

        assertEquals(1, queue.poll(0))
        assertEquals(2, queue.poll(0))
        assertNull(queue.poll(0))
    }

    @Test
    fun overflowReplacesPendingEventsWithExplicitCapacityEvent() {
        val queue = RNodeEventQueue<Int>(2) { -1 }

        queue.publish(1)
        queue.publish(2)
        queue.publish(3)

        assertEquals(-1, queue.poll(0))
        assertNull(queue.poll(0))
    }

    @Test
    fun bluetoothWritesUseNegotiatedMtuMinusAttOverhead() {
        val chunks = rnodeWriteChunks(ByteArray(100) { it.toByte() }, mtu = 50)

        assertEquals(listOf(47, 47, 6), chunks.map(ByteArray::size))
        assertContentEquals(ByteArray(100) { it.toByte() }, chunks.reduce(ByteArray::plus))
    }

    @Test
    fun bluetoothWritesFallBackToTwentyByteChunksForSmallOrInvalidMtu() {
        val data = ByteArray(45) { it.toByte() }

        assertEquals(listOf(20, 20, 5), rnodeWriteChunks(data, mtu = 23).map(ByteArray::size))
        assertEquals(listOf(20, 20, 5), rnodeWriteChunks(data, mtu = 0).map(ByteArray::size))
    }
}
