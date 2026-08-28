package io.styrene.mesh

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class MobileNodeStateHolderTest {
    @Test
    fun staleSuccessfulBootIsClosed() {
        val fixture = Fixture()

        fixture.holder.boot()
        fixture.worker.runNext()
        fixture.holder.close()
        fixture.results.runNext()

        assertEquals(1, fixture.client.closeCount)
        assertEquals(ConnectionState.Offline, fixture.holder.state.connection)
    }

    @Test
    fun closeAndReopenSamePeerRejectsOldConversationResult() {
        val fixture = Fixture().booted()
        fixture.client.messageResults.addLast(listOf(message("old")))
        fixture.client.messageResults.addLast(listOf(message("new")))
        val conversation = conversation("peer-a")

        fixture.holder.openConversation(conversation)
        fixture.worker.runNext()
        fixture.holder.closeConversation()
        fixture.holder.openConversation(conversation)
        fixture.worker.runNext()
        fixture.results.runNext()
        fixture.results.runNext()

        assertEquals(listOf("new"), fixture.holder.state.messages.map { it.content })
    }

    @Test
    fun retainedSendingStatePreventsDuplicateSend() {
        val fixture = Fixture().booted()
        fixture.openReadyConversation()
        fixture.holder.updateDraft("only once")

        fixture.holder.sendMessage()
        fixture.holder.sendMessage()
        fixture.worker.runNext()
        fixture.results.runNext()

        assertEquals(listOf("peer-a" to "only once"), fixture.client.sends)
        assertEquals("message-id", fixture.holder.state.lastQueuedMessageId)
    }

    @Test
    fun openingRnodeDoesNotDegradeReadyNode() {
        val fixture = Fixture().booted()

        fixture.holder.updateRnodeState("Opening Bluetooth RNode link", false)
        assertEquals(ConnectionState.Ready, fixture.holder.state.connection)

        fixture.holder.updateRnodeState("RNode error: disconnected", false)
        assertEquals(ConnectionState.Degraded, fixture.holder.state.connection)
    }

    @Test
    fun staleSendCompletionDoesNotClearReopenedDraft() {
        val fixture = Fixture().booted()
        fixture.openReadyConversation()
        fixture.holder.updateDraft("old draft")
        fixture.holder.sendMessage()

        fixture.worker.runNext()
        fixture.holder.closeConversation()
        fixture.holder.openConversation(conversation("peer-a"))
        fixture.holder.updateDraft("new draft")
        fixture.results.runNext()

        assertEquals("new draft", fixture.holder.state.draft)
        assertFalse(fixture.holder.state.isSending)
    }

    @Test
    fun nativeClientClosesBeforeExecutor() {
        val events = mutableListOf<String>()
        val fixture = Fixture(events).booted()

        fixture.holder.close()
        fixture.worker.runAll()

        assertEquals(listOf("node-close", "executor-close"), events.takeLast(2))
        assertTrue(fixture.delays.closed)
    }

    @Test
    fun nativeShutdownPrecedesHandleClose() {
        val events = mutableListOf<String>()

        shutdownAndClose(
            shutdown = { events += "shutdown" },
            close = { events += "close" },
        )

        assertEquals(listOf("shutdown", "close"), events)
    }

    @Test
    fun nativeHandleClosesWhenShutdownFails() {
        val events = mutableListOf<String>()

        runCatching {
            shutdownAndClose(
                shutdown = {
                    events += "shutdown"
                    error("shutdown failed")
                },
                close = { events += "close" },
            )
        }

        assertEquals(listOf("shutdown", "close"), events)
    }

    private class Fixture(private val events: MutableList<String> = mutableListOf()) {
        val worker = ManualExecutor(events)
        val results = ManualDispatcher()
        val delays = ManualDelays()
        val client = FakeClient(events)
        val holder = MobileNodeStateHolder(
            configuration = MobileNodeConfiguration("config", "data"),
            factory = MobileNodeClientFactory { client },
            executor = worker,
            dispatcher = results,
            delays = delays,
            nodeCloser = NodeCloser { it.close() },
        )

        fun booted(): Fixture {
            holder.boot()
            worker.runNext()
            results.runNext()
            worker.runNext()
            results.runNext()
            return this
        }

        fun openReadyConversation() {
            client.messageResults.addLast(emptyList())
            holder.openConversation(conversation("peer-a"))
            worker.runNext()
            results.runNext()
            worker.runNext()
            results.runNext()
        }
    }

    private class ManualExecutor(private val events: MutableList<String>) : OperationExecutor {
        private val tasks = ArrayDeque<() -> Unit>()

        override fun execute(action: () -> Unit) {
            tasks += action
        }

        override fun close(finalAction: () -> Unit) {
            tasks += {
                finalAction()
                events += "executor-close"
            }
        }

        fun runNext() = tasks.removeFirst().invoke()
        fun runAll() {
            while (tasks.isNotEmpty()) runNext()
        }
    }

    private class ManualDispatcher : ResultDispatcher {
        private val tasks = ArrayDeque<() -> Unit>()
        override fun dispatch(action: () -> Unit) {
            tasks += action
        }
        fun runNext() = tasks.removeFirst().invoke()
    }

    private class ManualDelays : DelayScheduler {
        var closed = false
        override fun schedule(delayMillis: Long, action: () -> Unit) = Unit
        override fun close() {
            closed = true
        }
    }

    private class FakeClient(private val events: MutableList<String>) : MobileNodeClient {
        var closeCount = 0
        val sends = mutableListOf<Pair<String, String>>()
        val messageResults = ArrayDeque<List<MessageSnapshot>>()
        override val rnodePacketChannel = object : RNodePacketChannel {
            override fun announce() = Unit
            override fun submit(packet: ByteArray) = Unit
            override fun poll(): ByteArray? = null
        }

        override fun identityHash() = "identity"
        override fun deliveryHash() = "delivery"
        override fun status() = NodeStatusSnapshot("test", 0, 0)
        override fun isConnected() = false
        override fun announce() = Unit
        override fun listPeers() = emptyList<PeerSnapshot>()
        override fun listContacts() = emptyList<ContactSnapshot>()
        override fun listConversations() = emptyList<ConversationSnapshot>()
        override fun markRead(peerHash: String) = Unit
        override fun getMessages(peerHash: String, limit: Int) = messageResults.removeFirst()
        override fun sendChat(peerHash: String, content: String): String {
            sends += peerHash to content
            return "message-id"
        }
        override fun browsePage(host: String, path: String) = "page"
        override fun close() {
            closeCount++
            events += "node-close"
        }
    }

    companion object {
        private fun conversation(hash: String) = ConversationCard(hash, "Peer", "", "", 0, false)
        private fun message(content: String) = MessageSnapshot(content, content, 1, false)
    }
}
