package io.styrene.mesh

import android.app.Activity
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.graphics.Color
import android.graphics.Typeface
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.Build
import android.os.Bundle
import android.view.Gravity
import android.view.View
import android.widget.Button
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import java.util.Locale
import java.util.concurrent.Executors
import uniffi.styrene_mobile_ffi.MobileConfig
import uniffi.styrene_mobile_ffi.MobileNode
import uniffi.styrene_mobile_ffi.PeerInfo

class MainActivity : Activity() {
    private val worker = Executors.newSingleThreadExecutor()
    private var node: MobileNode? = null
    private var rnodeController: RNodeController? = null
    private var permissionRequested = false
    private var lastRxPackets = 0L
    private var selectedPeerHash: String? = null
    private lateinit var coreValue: TextView
    private lateinit var identityValue: TextView
    private lateinit var deliveryValue: TextView
    private lateinit var transportValue: TextView
    private lateinit var usbValue: TextView
    private lateinit var selectedPeerValue: TextView
    private lateinit var peerDirectory: LinearLayout
    private lateinit var conversationDirectory: LinearLayout
    private lateinit var messageInput: EditText
    private lateinit var messagesValue: TextView
    private val usbPermissionReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action != ACTION_USB_PERMISSION) return
            permissionRequested = false
            val device = intent.usbDevice() ?: return
            if (intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)) {
                startRnode(device)
            } else {
                transportValue.setStatus("USB permission denied", false)
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        window.statusBarColor = BG
        window.navigationBarColor = BG
        registerUsbPermissionReceiver()
        setContentView(buildScreen())
        refreshUsb()
        bootNode()
    }

    override fun onDestroy() {
        rnodeController?.stop()
        node?.shutdown()
        node?.close()
        worker.shutdownNow()
        unregisterReceiver(usbPermissionReceiver)
        super.onDestroy()
    }

    private fun buildScreen(): View {
        val content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(dp(28), dp(30), dp(28), dp(30))
            setBackgroundColor(BG)
        }

        content.addView(label("STYRENE / FIELD NODE", 13f, ACCENT).apply {
            letterSpacing = 0.18f
        })
        content.addView(label("Mesh inbox", 34f, TEXT).apply {
            setTypeface(Typeface.create("sans-serif-condensed", Typeface.BOLD))
            setPadding(0, dp(10), 0, dp(8))
        })
        content.addView(label("Discover people, open a conversation, and communicate without infrastructure.", 15f, MUTED).apply {
            setPadding(0, 0, 0, dp(24))
        })

        coreValue = value("Starting native core...")
        identityValue = value("Pending")
        deliveryValue = value("Pending")
        transportValue = value("Pending")
        usbValue = value("Scanning USB host bus...")
        selectedPeerValue = value("No peer selected")
        peerDirectory = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        conversationDirectory = LinearLayout(this).apply { orientation = LinearLayout.VERTICAL }
        messagesValue = value("No messages")

        content.addView(card("NATIVE CORE", coreValue))
        content.addView(card("IDENTITY", identityValue))
        content.addView(card("LXMF DELIVERY", deliveryValue))
        content.addView(card("TRANSPORT", transportValue))
        content.addView(card("USB HOST", usbValue))

        messageInput = input("Message")
        content.addView(card("SELECTED PEER", selectedPeerValue))
        content.addView(card("PEOPLE", peerDirectory))
        content.addView(Button(this).apply {
            text = "DISCOVER PEERS"
            isAllCaps = false
            setTextColor(TEXT)
            textSize = 14f
            typeface = Typeface.create(Typeface.MONOSPACE, Typeface.BOLD)
            setBackgroundColor(CARD)
            setOnClickListener {
                node?.announce()
                refreshDirectory()
            }
        }, LinearLayout.LayoutParams(-1, dp(54)).apply {
            bottomMargin = dp(10)
        })
        content.addView(card("CONVERSATIONS", conversationDirectory))
        content.addView(messageInput)
        content.addView(Button(this).apply {
            text = "SEND LXMF CHAT"
            isAllCaps = false
            setTextColor(BG)
            textSize = 14f
            typeface = Typeface.create(Typeface.MONOSPACE, Typeface.BOLD)
            setBackgroundColor(ACCENT)
            setOnClickListener { sendMessage() }
        }, LinearLayout.LayoutParams(-1, dp(54)).apply {
            topMargin = dp(10)
        })
        content.addView(Button(this).apply {
            text = "REFRESH INBOX"
            isAllCaps = false
            setTextColor(TEXT)
            textSize = 14f
            typeface = Typeface.create(Typeface.MONOSPACE, Typeface.BOLD)
            setBackgroundColor(CARD)
            setOnClickListener { refreshDirectory() }
        }, LinearLayout.LayoutParams(-1, dp(54)).apply {
            topMargin = dp(10)
        })
        content.addView(card("MESSAGES", messagesValue).apply {
            (layoutParams as LinearLayout.LayoutParams).topMargin = dp(10)
        })

        content.addView(Button(this).apply {
            text = "RESCAN USB"
            isAllCaps = false
            setTextColor(BG)
            textSize = 14f
            typeface = Typeface.create(Typeface.MONOSPACE, Typeface.BOLD)
            setBackgroundColor(ACCENT)
            setPadding(dp(18), dp(12), dp(18), dp(12))
            setOnClickListener { refreshUsb() }
        }, LinearLayout.LayoutParams(-1, dp(54)).apply {
            topMargin = dp(10)
        })

        content.addView(label("E8A  /  ANDROID 15  /  ARM64", 12f, MUTED).apply {
            gravity = Gravity.CENTER
            setPadding(0, dp(26), 0, 0)
            letterSpacing = 0.12f
        })

        return ScrollView(this).apply { addView(content) }
    }

    private fun bootNode() {
        worker.execute {
            runCatching {
                val instance = MobileNode.boot(
                    MobileConfig(
                        configDir = filesDir.resolve("config").absolutePath,
                        dataDir = filesDir.resolve("data").absolutePath,
                        hubAddress = null,
                        hubDeliveryHash = null,
                        displayName = "E8A field node",
                        identityBackend = "plaintext_file",
                        interfaces = emptyList(),
                        enableRnodeChannel = true,
                    ),
                )
                node = instance
                val status = instance.status()
                Triple(instance.identityHash(), instance.deliveryHash(), status.daemonVersion)
            }.onSuccess { (identity, delivery, version) ->
                runOnUiThread {
                    coreValue.setStatus("Styrene $version loaded", true)
                    identityValue.setStatus(identity, true)
                    deliveryValue.setStatus(delivery ?: "Unavailable", delivery != null)
                    transportValue.setStatus("Rust transport ready; opening RNode", false)
                    refreshUsb()
                    refreshDirectory()
                }
            }.onFailure { error ->
                runOnUiThread {
                    coreValue.setStatus("Boot failed: ${error.message}", false)
                    identityValue.setStatus("Unavailable", false)
                    deliveryValue.setStatus("Unavailable", false)
                    transportValue.setStatus("Unavailable", false)
                }
            }
        }
    }

    private fun refreshUsb() {
        val manager = getSystemService(USB_SERVICE) as UsbManager
        val devices = manager.deviceList.values.sortedBy { it.deviceName }
        val heltec = devices.firstOrNull { it.vendorId == 0x10c4 && it.productId == 0xea60 }

        if (heltec != null) {
            usbValue.setStatus(
                "Heltec CP2102 detected\n${usbId(heltec.vendorId, heltec.productId)} " +
                    "/ ${heltec.interfaceCount} interfaces",
                true,
            )
            node?.let { ensureRnode(heltec) }
        } else if (devices.isEmpty()) {
            usbValue.setStatus("Host supported; no peripheral attached", false)
        } else {
            usbValue.setStatus(
                devices.joinToString("\n") {
                    "${usbId(it.vendorId, it.productId)} / ${it.interfaceCount} interfaces"
                },
                false,
            )
        }
    }

    private fun sendMessage() {
        val peer = selectedPeerHash
        val content = messageInput.text.toString()
        if (peer == null || content.isBlank()) {
            messagesValue.setStatus("Select a discovered peer and enter a message", false)
            return
        }
        val mobileNode = node ?: return
        messagesValue.setStatus("Sending...", false)
        worker.execute {
            runCatching { mobileNode.sendChat(peer, content) }
                .onSuccess { id ->
                    runOnUiThread {
                        messageInput.text.clear()
                        messagesValue.setStatus("Sent ${id.take(12)}", true)
                        refreshMessages(peer)
                    }
                }
                .onFailure { error ->
                    runOnUiThread {
                        messagesValue.setStatus("Send failed: ${error.message}", false)
                    }
                }
        }
    }

    private fun refreshDirectory() {
        val mobileNode = node ?: return
        worker.execute {
            runCatching {
                Triple(
                    mobileNode.listPeers(),
                    mobileNode.listContacts().associateBy { it.peerHash },
                    mobileNode.listConversations(),
                )
            }.onSuccess { (peers, contacts, conversations) ->
                runOnUiThread {
                    renderPeers(peers, contacts.mapValues { it.value.alias })
                    conversationDirectory.removeAllViews()
                    if (conversations.isEmpty()) {
                        conversationDirectory.addView(label("No conversations yet", 14f, MUTED))
                    } else {
                        conversations.sortedByDescending { it.lastActivity }.forEach { conversation ->
                            val name = contacts[conversation.peerHash]?.alias
                                ?: peers.firstOrNull { it.destinationHash == conversation.peerHash }?.name
                                ?: shortHash(conversation.peerHash)
                            conversationDirectory.addView(directoryButton(
                                "$name  /  ${conversation.messageCount} messages" +
                                    if (conversation.unreadCount > 0u) "  /  ${conversation.unreadCount} new" else "",
                            ) { refreshMessages(conversation.peerHash) })
                        }
                    }
                }
                refreshMessages(selectedPeerHash)
            }.onFailure { error ->
                runOnUiThread {
                    messagesValue.setStatus("Directory refresh failed: ${error.message}", false)
                }
            }
        }
    }

    private fun renderPeers(peers: List<PeerInfo>, aliases: Map<String, String?>) {
        peerDirectory.removeAllViews()
        if (peers.isEmpty()) {
            peerDirectory.addView(label("Listening for peer announces...", 14f, MUTED))
            return
        }
        peers.sortedBy { aliases[it.destinationHash] ?: it.name ?: it.destinationHash }.forEach { peer ->
            val name = aliases[peer.destinationHash] ?: peer.name ?: "Unnamed peer"
            peerDirectory.addView(directoryButton("$name  /  ${peer.status}") {
                selectedPeerHash = peer.destinationHash
                selectedPeerValue.setStatus("$name\n${shortHash(peer.destinationHash)}", true)
                refreshMessages(peer.destinationHash)
            })
        }
    }

    private fun refreshMessages(peerHash: String?) {
        val mobileNode = node ?: return
        if (peerHash == null) {
            messagesValue.setStatus("Select a peer or conversation", false)
            return
        }
        worker.execute {
            runCatching { mobileNode.getMessages(peerHash, 50u).sortedBy { it.timestamp } }
                .onSuccess { messages ->
                val text = if (messages.isEmpty()) {
                    "No messages"
                } else {
                    messages.joinToString("\n") {
                        "${if (it.isOutgoing) ">" else "<"} ${it.content}"
                    }
                }
                runOnUiThread { messagesValue.setStatus(text, messages.isNotEmpty()) }
                }.onFailure { error ->
                    runOnUiThread {
                        messagesValue.setStatus("Refresh failed: ${error.message}", false)
                    }
                }
        }
    }

    private fun ensureRnode(device: UsbDevice) {
        if (rnodeController != null) return
        val manager = getSystemService(USB_SERVICE) as UsbManager
        if (manager.hasPermission(device)) {
            startRnode(device)
            return
        }
        if (permissionRequested) return

        permissionRequested = true
        transportValue.setStatus("Allow USB access to open the RNode", false)
        val permissionIntent = PendingIntent.getBroadcast(
            this,
            0,
            Intent(ACTION_USB_PERMISSION).setPackage(packageName),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
        )
        manager.requestPermission(device, permissionIntent)
    }

    private fun startRnode(device: UsbDevice) {
        val mobileNode = node ?: return
        if (rnodeController != null) return

        rnodeController = RNodeController(
            usbManager = getSystemService(USB_SERVICE) as UsbManager,
            node = mobileNode,
            listener = object : RNodeController.Listener {
                override fun onState(message: String, online: Boolean) {
                    runOnUiThread { transportValue.setStatus(message, online) }
                }

                override fun onTraffic(rxPackets: Long, txPackets: Long) {
                    runOnUiThread {
                        transportValue.setStatus(
                            "RNode online / RX $rxPackets / TX $txPackets",
                            true,
                        )
                        if (rxPackets > lastRxPackets) {
                            lastRxPackets = rxPackets
                            transportValue.postDelayed({ refreshDirectory() }, 1_500)
                        }
                    }
                }
            },
        ).also { it.start(device) }
    }

    private fun registerUsbPermissionReceiver() {
        val filter = IntentFilter(ACTION_USB_PERMISSION)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(usbPermissionReceiver, filter, RECEIVER_NOT_EXPORTED)
        } else {
            @Suppress("DEPRECATION")
            registerReceiver(usbPermissionReceiver, filter)
        }
    }

    @Suppress("DEPRECATION")
    private fun Intent.usbDevice(): UsbDevice? = if (Build.VERSION.SDK_INT >= 33) {
        getParcelableExtra(UsbManager.EXTRA_DEVICE, UsbDevice::class.java)
    } else {
        getParcelableExtra(UsbManager.EXTRA_DEVICE)
    }

    private fun card(title: String, body: View): View = LinearLayout(this).apply {
        orientation = LinearLayout.VERTICAL
        setPadding(dp(18), dp(16), dp(18), dp(17))
        setBackgroundColor(CARD)
        addView(label(title, 11f, MUTED).apply { letterSpacing = 0.16f })
        addView(body)
    }.also {
        it.layoutParams = LinearLayout.LayoutParams(-1, -2).apply { bottomMargin = dp(10) }
    }

    private fun value(text: String) = label(text, 15f, TEXT).apply {
        setPadding(0, dp(7), 0, 0)
        setTypeface(Typeface.MONOSPACE)
        setLineSpacing(0f, 1.12f)
    }

    private fun input(hint: String) = EditText(this).apply {
        this.hint = hint
        setHintTextColor(MUTED)
        setTextColor(TEXT)
        textSize = 14f
        setSingleLine(true)
        typeface = Typeface.MONOSPACE
        setPadding(dp(14), dp(10), dp(14), dp(10))
        setBackgroundColor(CARD)
        layoutParams = LinearLayout.LayoutParams(-1, dp(52)).apply { bottomMargin = dp(10) }
    }

    private fun directoryButton(text: String, onClick: () -> Unit) = Button(this).apply {
        this.text = text
        isAllCaps = false
        gravity = Gravity.START or Gravity.CENTER_VERTICAL
        setTextColor(TEXT)
        textSize = 14f
        typeface = Typeface.create("sans-serif", Typeface.NORMAL)
        setBackgroundColor(Color.TRANSPARENT)
        setPadding(0, dp(8), 0, dp(8))
        setOnClickListener { onClick() }
    }

    private fun shortHash(hash: String) = if (hash.length > 12) {
        "${hash.take(6)}...${hash.takeLast(6)}"
    } else {
        hash
    }

    private fun label(text: String, size: Float, color: Int) = TextView(this).apply {
        this.text = text
        textSize = size
        setTextColor(color)
    }

    private fun TextView.setStatus(value: String, healthy: Boolean) {
        text = value
        setTextColor(if (healthy) ACCENT else AMBER)
    }

    private fun usbId(vendorId: Int, productId: Int) = String.format(
        Locale.US,
        "%04x:%04x",
        vendorId,
        productId,
    )

    private fun dp(value: Int) = (value * resources.displayMetrics.density).toInt()

    companion object {
        private const val ACTION_USB_PERMISSION = "io.styrene.mesh.USB_PERMISSION"
        private val BG = Color.rgb(7, 14, 13)
        private val CARD = Color.rgb(15, 28, 24)
        private val TEXT = Color.rgb(228, 240, 234)
        private val MUTED = Color.rgb(130, 153, 143)
        private val ACCENT = Color.rgb(116, 238, 177)
        private val AMBER = Color.rgb(241, 181, 92)
    }
}
