package io.styrene.mesh

import java.util.concurrent.atomic.AtomicBoolean
import kotlin.concurrent.thread

class RNodeController(
    node: RNodePacketChannel,
    outbound: RNodeOutboundBuffer,
    radioProfile: RNodeRadioProfile,
    private val listener: Listener,
    private val onStopped: () -> Unit = {},
) {
    interface Listener {
        fun onState(message: String, online: Boolean)
        fun onTraffic(rxPackets: Long, txPackets: Long)
    }

    private val running = AtomicBoolean(false)
    private val session = RNodeSession(
        node = node,
        outbound = outbound,
        profile = radioProfile,
        onState = listener::onState,
        onTraffic = listener::onTraffic,
    )

    @Volatile
    private var link: RNodeByteLink? = null
    @Volatile
    private var worker: Thread? = null

    @Synchronized
    fun start(bearerName: String, openLink: () -> RNodeByteLink) {
        if (!running.compareAndSet(false, true)) return
        val nextWorker = thread(
            start = false,
            name = "styrene-rnode-${bearerName.lowercase()}",
            isDaemon = true,
        ) {
            listener.onState("Opening $bearerName RNode link", false)
            runCatching {
                val openedLink = openLink()
                link = openedLink
                check(running.get()) { "RNode start cancelled" }
                session.initialize(openedLink, running::get)
                while (running.get()) session.pumpOnce(openedLink)
            }.onFailure { error ->
                if (running.get()) listener.onState("RNode error: ${error.message}", false)
            }
            closeLink()
            worker = null
            onStopped()
        }
        worker = nextWorker
        nextWorker.start()
    }

    fun stop() {
        if (!running.getAndSet(false)) return
        link?.let { activeLink -> runCatching { session.shutdown(activeLink) } }
        closeLink()
        val stoppingWorker = worker
        stoppingWorker?.interrupt()
        if (stoppingWorker != null && stoppingWorker !== Thread.currentThread()) {
            stoppingWorker.join(STOP_JOIN_TIMEOUT_MS)
        }
    }

    private fun closeLink() {
        running.set(false)
        val closingLink = link
        link = null
        runCatching { closingLink?.close() }
    }

    private companion object {
        const val STOP_JOIN_TIMEOUT_MS = 1_000L
    }
}
