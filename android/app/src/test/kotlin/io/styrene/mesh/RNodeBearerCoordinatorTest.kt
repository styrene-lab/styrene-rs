package io.styrene.mesh

import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class RNodeBearerCoordinatorTest {
    @Test
    fun closeCancelsOpeningBearerBeforeReplacementConnects() {
        val opening = CountDownLatch(1)
        val stopped = CountDownLatch(1)
        val coordinator = coordinator()

        assertTrue(
            coordinator.connect("Bluetooth") {
                opening.countDown()
                try {
                    CountDownLatch(1).await()
                    error("Opening bearer was not cancelled")
                } finally {
                    stopped.countDown()
                }
            },
        )
        assertTrue(opening.await(1, TimeUnit.SECONDS))

        coordinator.close()

        assertTrue(stopped.await(1, TimeUnit.SECONDS))
        assertTrue(coordinator.connect("USB") { FakeRNodeLink() })
        coordinator.close()
    }

    @Test
    fun activeBearerRejectsConcurrentReplacement() {
        val opening = CountDownLatch(1)
        val coordinator = coordinator()

        assertTrue(
            coordinator.connect("Bluetooth") {
                opening.countDown()
                CountDownLatch(1).await()
                FakeRNodeLink()
            },
        )
        assertTrue(opening.await(1, TimeUnit.SECONDS))

        assertFalse(coordinator.connect("USB") { FakeRNodeLink() })
        coordinator.close()
    }

    private fun coordinator() = RNodeBearerCoordinator(
        node = FakePacketChannel(),
        outbound = RNodeOutboundBuffer(FakePacketChannel()),
        radioProfile = RNodeRadioProfile.US_915_DEVELOPMENT,
        listener = object : RNodeController.Listener {
            override fun onState(message: String, online: Boolean) = Unit
            override fun onTraffic(rxPackets: Long, txPackets: Long) = Unit
        },
    )

    private class FakePacketChannel : RNodePacketChannel {
        override fun announce() = Unit
        override fun submit(packet: ByteArray) = Unit
        override fun poll(): ByteArray? = null
    }

    private class FakeRNodeLink : RNodeByteLink {
        override val bearerName = "test"
        override fun read(buffer: ByteArray, timeoutMs: Int) = 0
        override fun write(data: ByteArray, timeoutMs: Int) = Unit
        override fun close() = Unit
    }
}
