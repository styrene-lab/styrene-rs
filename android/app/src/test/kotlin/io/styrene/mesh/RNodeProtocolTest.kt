package io.styrene.mesh

import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals

class RNodeProtocolTest {
    @Test
    fun frameEscapesKissBytes() {
        assertContentEquals(
            byteArrayOf(0xc0.toByte(), 0x00, 0xdb.toByte(), 0xdc.toByte(), 0xdb.toByte(), 0xdd.toByte(), 0xc0.toByte()),
            RNodeProtocol.frame(0, byteArrayOf(0xc0.toByte(), 0xdb.toByte())),
        )
    }

    @Test
    fun decoderHandlesFragmentedFrames() {
        val decoder = RNodeProtocol.Decoder()
        val encoded = RNodeProtocol.frame(8, byteArrayOf(0x46))

        assertEquals(emptyList(), decoder.feed(encoded.copyOfRange(0, 2)))
        val frames = decoder.feed(encoded.copyOfRange(2, encoded.size))

        assertEquals(1, frames.size)
        assertEquals(8, frames.single().command)
        assertContentEquals(byteArrayOf(0x46), frames.single().payload)
    }

    @Test
    fun unsignedIntUsesNetworkByteOrder() {
        val encoded = RNodeProtocol.unsignedInt(915_000_000)

        assertEquals(915_000_000, RNodeProtocol.readUnsignedInt(encoded))
    }
}
