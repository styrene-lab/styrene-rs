package io.styrene.mesh

import java.text.DateFormat
import java.util.Date
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.TimeUnit

interface OperationExecutor {
    fun execute(action: () -> Unit)
    fun close(finalAction: () -> Unit)
}

fun interface ResultDispatcher {
    fun dispatch(action: () -> Unit)
}

fun interface NodeCloser {
    fun close(client: MobileNodeClient)
}

interface DelayScheduler {
    fun schedule(delayMillis: Long, action: () -> Unit)
    fun close()
}

class SerialOperationExecutor(
    private val executor: ExecutorService = Executors.newSingleThreadExecutor(),
) : OperationExecutor {
    override fun execute(action: () -> Unit) = executor.execute(action)

    override fun close(finalAction: () -> Unit) {
        executor.execute {
            try {
                finalAction()
            } finally {
                executor.shutdown()
            }
        }
    }
}

class ScheduledDelayScheduler(
    private val executor: ScheduledExecutorService = Executors.newSingleThreadScheduledExecutor(),
) : DelayScheduler {
    override fun schedule(delayMillis: Long, action: () -> Unit) {
        executor.schedule(action, delayMillis, TimeUnit.MILLISECONDS)
    }

    override fun close() {
        executor.shutdownNow()
    }
}

class MobileNodeStateHolder(
    private val configuration: MobileNodeConfiguration,
    private val factory: MobileNodeClientFactory,
    private val executor: OperationExecutor,
    private val dispatcher: ResultDispatcher,
    private val delays: DelayScheduler,
    private val nodeCloser: NodeCloser,
    private val onStateChanged: (MobileUiState) -> Unit = {},
) : AutoCloseable {
    var state: MobileUiState = MobileUiState(hubAddress = configuration.hubAddress)
        private set

    private var client: MobileNodeClient? = null
    private var closed = false
    private var lifecycleGeneration = 0L
    private var bootRequest = 0L
    private var directoryRequest = 0L
    private var conversationRequest = 0L
    private var conversationGeneration = 0L
    private var sendRequest = 0L
    private var pageRequest = 0L
    private var announceRequest = 0L
    private var delayedRefreshRequest = 0L
    private var draftRevision = 0L
    private var rnodeOnline = false

    @Synchronized
    fun boot() {
        if (closed || client != null || state.connection == ConnectionState.Starting) return
        val generation = lifecycleGeneration
        val request = ++bootRequest
        update(state.copy(connection = ConnectionState.Starting, notice = "Starting secure node"))
        executor.execute {
            var created: MobileNodeClient? = null
            val result = runCatching {
                created = factory.create(configuration)
                val status = created!!.status()
                BootResult(created!!, created!!.identityHash(), created!!.deliveryHash(), status)
            }
            dispatcher.dispatch {
                result.onSuccess { bootResult ->
                    val stale = synchronized(this) {
                        closed || generation != lifecycleGeneration || request != bootRequest || client != null
                    }
                    if (stale) {
                        nodeCloser.close(bootResult.client)
                    } else {
                        synchronized(this) {
                            client = bootResult.client
                            update(
                                state.copy(
                                    connection = ConnectionState.Ready,
                                    identityHash = bootResult.identity,
                                    deliveryHash = bootResult.delivery,
                                    daemonVersion = bootResult.status.daemonVersion,
                                    peerCount = bootResult.status.peerCount,
                                    linkCount = bootResult.status.linkCount,
                                    notice = "Node ready; looking for a radio",
                                ),
                            )
                        }
                        refreshDirectory()
                    }
                }.onFailure { error ->
                    created?.let(nodeCloser::close)
                    synchronized(this) {
                        if (!closed && generation == lifecycleGeneration && request == bootRequest) {
                            update(state.copy(connection = ConnectionState.Offline, notice = "Node failed: ${error.message}"))
                        }
                    }
                }
            }
        }
    }

    @Synchronized
    fun announce() {
        val operationClient = client ?: return
        val generation = lifecycleGeneration
        val request = ++announceRequest
        update(state.copy(notice = "Announcing identity"))
        executor.execute {
            val result = runCatching { operationClient.announce() }
            dispatcher.dispatch {
                synchronized(this) {
                    if (!isCurrent(operationClient, generation) || request != announceRequest) return@dispatch
                    update(state.copy(notice = result.fold({ "Identity announced" }, { "Announce failed: ${it.message}" })))
                    if (result.isSuccess) scheduleRefresh()
                }
            }
        }
    }

    @Synchronized
    fun refreshDirectory() {
        val operationClient = client ?: return
        val generation = lifecycleGeneration
        val request = ++directoryRequest
        executor.execute {
            val result = runCatching { loadDirectory(operationClient) }
            dispatcher.dispatch {
                synchronized(this) {
                    if (!isCurrent(operationClient, generation) || request != directoryRequest) return@dispatch
                    result.onSuccess {
                        update(
                            state.copy(
                                people = it.people,
                                conversations = it.conversations,
                                peerCount = it.peerCount,
                                linkCount = it.linkCount,
                                connection = if (it.connected || rnodeOnline) ConnectionState.Connected else ConnectionState.Ready,
                                lastRefresh = "Just now",
                            ),
                        )
                    }.onFailure { update(state.copy(notice = "Refresh failed: ${it.message}")) }
                }
            }
        }
    }

    @Synchronized
    fun openConversation(conversation: ConversationCard) {
        val selection = ++conversationGeneration
        conversationRequest++
        sendRequest++
        draftRevision++
        update(
            state.copy(
                selectedConversation = conversation,
                messages = if (conversation.isPreview) previewMessages(conversation.hash) else emptyList(),
                draft = "",
                isSending = false,
            ),
        )
        if (!conversation.isPreview) loadConversation(conversation, selection)
    }

    fun openPerson(person: PersonCard) = openConversation(
        ConversationCard(person.hash, person.name, "New conversation", "", 0, person.preview),
    )

    @Synchronized
    fun closeConversation() {
        conversationGeneration++
        conversationRequest++
        sendRequest++
        draftRevision++
        update(state.copy(selectedConversation = null, messages = emptyList(), draft = "", isSending = false))
    }

    @Synchronized
    fun updateDraft(value: String) {
        draftRevision++
        update(state.copy(draft = value))
    }

    @Synchronized
    fun sendMessage() {
        val conversation = state.selectedConversation ?: return
        val content = state.draft.trim()
        if (content.isBlank() || state.isSending) return
        if (conversation.isPreview) {
            draftRevision++
            update(
                state.copy(
                    messages = state.messages + MessageCard(
                        "preview-${state.messages.size}", content, true, "Now", "Preview", "Direct · Preview composer",
                    ),
                    draft = "",
                    notice = "Preview message composed; no packet sent",
                ),
            )
            return
        }
        val operationClient = client ?: return
        val generation = lifecycleGeneration
        val selection = conversationGeneration
        val request = ++sendRequest
        val submittedRevision = draftRevision
        update(state.copy(isSending = true))
        executor.execute {
            val result = runCatching { operationClient.sendChat(conversation.hash, content) }
            dispatcher.dispatch {
                var reload = false
                synchronized(this) {
                    if (!isCurrent(operationClient, generation) ||
                        request != sendRequest || selection != conversationGeneration ||
                        state.selectedConversation?.hash != conversation.hash
                    ) return@dispatch
                    result.onSuccess { messageId ->
                        update(
                            state.copy(
                                isSending = false,
                                draft = if (draftRevision == submittedRevision) "" else state.draft,
                                lastQueuedMessageId = messageId,
                                notice = "Message queued",
                            ),
                        )
                        reload = true
                    }.onFailure {
                        update(state.copy(isSending = false, notice = "Send failed: ${it.message}"))
                    }
                }
                if (reload) loadConversation(conversation, selection)
            }
        }
    }

    @Synchronized
    fun browsePage(host: String, path: String) {
        val operationClient = client ?: return
        if (state.pageLoading) return
        val destination = host.trim()
        val nativePath = path.trim()
        val generation = lifecycleGeneration
        val request = ++pageRequest
        update(state.copy(pageLoading = true, pageError = null, pageSource = "", pageAddress = ""))
        executor.execute {
            val result = runCatching { operationClient.browsePage(destination, nativePath) }
            dispatcher.dispatch {
                synchronized(this) {
                    if (!isCurrent(operationClient, generation) || request != pageRequest) return@dispatch
                    result.onSuccess {
                        update(state.copy(pageLoading = false, pageSource = it, pageAddress = "$destination:$nativePath"))
                    }.onFailure {
                        update(state.copy(pageLoading = false, pageError = it.message ?: "Page fetch failed"))
                    }
                }
            }
        }
    }

    @Synchronized
    fun rnodePacketChannel(): RNodePacketChannel? = client?.rnodePacketChannel

    @Synchronized
    fun updateUsbSummary(usbSummary: String, transportSummary: String, available: Boolean = false) {
        if (closed) return
        update(state.copy(usbSummary = usbSummary, transportSummary = transportSummary, usbAvailable = available))
    }

    @Synchronized
    fun updateBluetoothSummary(summary: String) {
        if (closed) return
        update(state.copy(bluetoothSummary = summary))
    }

    @Synchronized
    fun updateRnodeCandidates(candidates: List<RNodeCandidate>) {
        if (closed) return
        update(state.copy(rnodeCandidates = candidates))
    }

    @Synchronized
    fun updateRnodeState(message: String, online: Boolean) {
        if (closed) return
        rnodeOnline = online
        val connection = when {
            online -> ConnectionState.Connected
            message.startsWith("RNode error:") -> ConnectionState.Degraded
            state.connection == ConnectionState.Connected -> ConnectionState.Ready
            else -> state.connection
        }
        update(state.copy(connection = connection, transportSummary = message))
    }

    @Synchronized
    fun updateRnodeTraffic(rxPackets: Long, txPackets: Long) {
        if (closed) return
        rnodeOnline = true
        update(
            state.copy(
                connection = ConnectionState.Connected,
                transportSummary = "RNode online",
                rxPackets = rxPackets,
                txPackets = txPackets,
            ),
        )
    }

    @Synchronized
    fun scheduleRefresh(delayMillis: Long = 1_500) {
        if (closed) return
        val generation = lifecycleGeneration
        val request = ++delayedRefreshRequest
        delays.schedule(delayMillis) {
            dispatcher.dispatch {
                synchronized(this) {
                    if (closed || generation != lifecycleGeneration || request != delayedRefreshRequest) return@dispatch
                }
                refreshDirectory()
            }
        }
    }

    @Synchronized
    override fun close() {
        if (closed) return
        closed = true
        lifecycleGeneration++
        bootRequest++
        directoryRequest++
        conversationRequest++
        sendRequest++
        pageRequest++
        announceRequest++
        delayedRefreshRequest++
        val closingClient = client
        client = null
        delays.close()
        update(state.copy(connection = ConnectionState.Offline, isSending = false, pageLoading = false))
        executor.close { closingClient?.close() }
    }

    private fun loadConversation(conversation: ConversationCard, selection: Long) {
        val operationClient: MobileNodeClient
        val generation: Long
        val request: Long
        synchronized(this) {
            operationClient = client ?: return
            generation = lifecycleGeneration
            request = ++conversationRequest
        }
        executor.execute {
            val result = runCatching {
                operationClient.markRead(conversation.hash)
                operationClient.getMessages(conversation.hash, 100).sortedBy { it.timestamp }.map {
                    MessageCard(
                        it.id,
                        it.content,
                        it.outgoing,
                        DateFormat.getTimeInstance(DateFormat.SHORT).format(Date(it.timestamp * 1_000)),
                        if (it.outgoing) "Outgoing" else "Received",
                        if (it.outgoing) "Route evidence unavailable" else null,
                    )
                }
            }
            dispatcher.dispatch {
                var refresh = false
                synchronized(this) {
                    if (!isCurrent(operationClient, generation) || request != conversationRequest ||
                        selection != conversationGeneration || state.selectedConversation?.hash != conversation.hash
                    ) return@dispatch
                    result.onSuccess {
                        update(state.copy(messages = it))
                        refresh = true
                    }.onFailure { update(state.copy(notice = "Conversation failed: ${it.message}")) }
                }
                if (refresh) refreshDirectory()
            }
        }
    }

    private fun loadDirectory(node: MobileNodeClient): DirectoryResult {
        val peers = node.listPeers()
        val contacts = node.listContacts().associateBy { it.peerHash }
        val conversations = node.listConversations()
        val status = node.status()
        return DirectoryResult(
            people = peers.map { peer ->
                PersonCard(
                    peer.hash,
                    contacts[peer.hash]?.alias ?: peer.name ?: "Unnamed peer",
                    peer.status,
                    contacts.containsKey(peer.hash),
                    false,
                )
            } + contacts.values.filter { contact -> peers.none { it.hash == contact.peerHash } }.map { contact ->
                PersonCard(
                    contact.peerHash,
                    contact.alias ?: shortHash(contact.peerHash),
                    "Saved contact; not recently discovered",
                    true,
                    false,
                )
            },
            conversations = conversations.sortedByDescending { it.lastActivity }.map { conversation ->
                ConversationCard(
                    conversation.peerHash,
                    contacts[conversation.peerHash]?.alias
                        ?: peers.firstOrNull { it.hash == conversation.peerHash }?.name
                        ?: shortHash(conversation.peerHash),
                    "${conversation.messageCount} messages",
                    DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT)
                        .format(Date(conversation.lastActivity * 1_000)),
                    conversation.unreadCount,
                    false,
                )
            },
            peerCount = status.peerCount,
            linkCount = status.linkCount,
            connected = node.isConnected(),
        )
    }

    @Synchronized
    private fun isCurrent(operationClient: MobileNodeClient, generation: Long) =
        !closed && generation == lifecycleGeneration && client === operationClient

    @Synchronized
    private fun update(newState: MobileUiState) {
        state = newState
        onStateChanged(newState)
    }

    private fun shortHash(hash: String) = if (hash.length > 12) "${hash.take(6)}...${hash.takeLast(6)}" else hash

    private data class BootResult(
        val client: MobileNodeClient,
        val identity: String,
        val delivery: String,
        val status: NodeStatusSnapshot,
    )

    private data class DirectoryResult(
        val people: List<PersonCard>,
        val conversations: List<ConversationCard>,
        val peerCount: Int,
        val linkCount: Int,
        val connected: Boolean,
    )
}
