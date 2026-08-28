package io.styrene.mesh

import uniffi.styrene_mobile_ffi.MobileConfig
import uniffi.styrene_mobile_ffi.MobileNode

data class MobileNodeConfiguration(
    val configDir: String,
    val dataDir: String,
    val hubAddress: String? = null,
    val displayName: String = "Android field node",
    val enableRnodeChannel: Boolean = true,
)
data class NodeStatusSnapshot(val daemonVersion: String, val peerCount: Int, val linkCount: Int)
data class PeerSnapshot(val hash: String, val name: String?, val status: String)
data class ContactSnapshot(val peerHash: String, val alias: String?)
data class ConversationSnapshot(
    val peerHash: String,
    val unreadCount: Int,
    val messageCount: Int,
    val lastActivity: Long,
)
data class MessageSnapshot(val id: String, val content: String, val timestamp: Long, val outgoing: Boolean)

interface RNodePacketChannel {
    fun announce()
    fun submit(packet: ByteArray)
    fun poll(): ByteArray?
}

interface MobileNodeClient : AutoCloseable {
    val rnodePacketChannel: RNodePacketChannel
    fun identityHash(): String
    fun deliveryHash(): String
    fun status(): NodeStatusSnapshot
    fun isConnected(): Boolean
    fun announce()
    fun listPeers(): List<PeerSnapshot>
    fun listContacts(): List<ContactSnapshot>
    fun listConversations(): List<ConversationSnapshot>
    fun markRead(peerHash: String)
    fun getMessages(peerHash: String, limit: Int): List<MessageSnapshot>
    fun sendChat(peerHash: String, content: String): String
    fun browsePage(host: String, path: String): String
}

fun interface MobileNodeClientFactory {
    fun create(configuration: MobileNodeConfiguration): MobileNodeClient
}

class UniFfiMobileNodeClientFactory : MobileNodeClientFactory {
    override fun create(configuration: MobileNodeConfiguration): MobileNodeClient = UniFfiMobileNodeClient(
        MobileNode.boot(
            MobileConfig(
                configDir = configuration.configDir,
                dataDir = configuration.dataDir,
                hubAddress = configuration.hubAddress,
                hubDeliveryHash = null,
                displayName = configuration.displayName,
                identityBackend = "plaintext_file",
                interfaces = emptyList(),
                enableRnodeChannel = configuration.enableRnodeChannel,
            ),
        ),
    )
}

private class UniFfiMobileNodeClient(private val node: MobileNode) : MobileNodeClient {
    override val rnodePacketChannel = object : RNodePacketChannel {
        override fun announce() = node.announce()
        override fun submit(packet: ByteArray) = node.submitRnodePacket(packet)
        override fun poll(): ByteArray? = node.pollRnodePacket()
    }

    override fun identityHash() = node.identityHash()
    override fun deliveryHash() = node.deliveryHash().orEmpty()
    override fun status() = node.status().let {
        NodeStatusSnapshot(it.daemonVersion, it.peerCount.toInt(), it.linkCount.toInt())
    }
    override fun isConnected() = node.isConnected()
    override fun announce() = node.announce()
    override fun listPeers() = node.listPeers().map { PeerSnapshot(it.destinationHash, it.name, it.status) }
    override fun listContacts() = node.listContacts().map { ContactSnapshot(it.peerHash, it.alias) }
    override fun listConversations() = node.listConversations().map {
        ConversationSnapshot(it.peerHash, it.unreadCount.toInt(), it.messageCount.toInt(), it.lastActivity)
    }
    override fun markRead(peerHash: String) = node.markRead(peerHash)
    override fun getMessages(peerHash: String, limit: Int) = node.getMessages(peerHash, limit.toUInt()).map {
        MessageSnapshot(it.id, it.content, it.timestamp, it.isOutgoing)
    }
    override fun sendChat(peerHash: String, content: String) = node.sendChat(peerHash, content)
    override fun browsePage(host: String, path: String) = node.browsePage(host, path)

    override fun close() {
        shutdownAndClose(node::shutdown, node::close)
    }
}

internal fun shutdownAndClose(shutdown: () -> Unit, close: () -> Unit) {
    try {
        shutdown()
    } finally {
        close()
    }
}
