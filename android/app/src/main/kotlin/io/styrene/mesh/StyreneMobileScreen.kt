package io.styrene.mesh

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.AttachFile
import androidx.compose.material.icons.filled.Bolt
import androidx.compose.material.icons.filled.ChevronRight
import androidx.compose.material.icons.filled.ContentCopy
import androidx.compose.material.icons.filled.Description
import androidx.compose.material.icons.filled.Explore
import androidx.compose.material.icons.filled.Group
import androidx.compose.material.icons.filled.Hub
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Mail
import androidx.compose.material.icons.filled.Map
import androidx.compose.material.icons.filled.MoreHoriz
import androidx.compose.material.icons.filled.Person
import androidx.compose.material.icons.filled.Radio
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Security
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Sync
import androidx.compose.material.icons.filled.Terminal
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.Switch
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

enum class ConnectionState(val label: String) {
    Offline("Offline"),
    Starting("Starting"),
    Ready("Node ready"),
    Connected("Mesh connected"),
    Degraded("Degraded"),
}

data class ConversationCard(
    val hash: String,
    val name: String,
    val preview: String,
    val timestamp: String,
    val unread: Int,
    val isPreview: Boolean,
)

data class PersonCard(
    val hash: String,
    val name: String,
    val detail: String,
    val saved: Boolean,
    val preview: Boolean,
)

data class MessageCard(
    val id: String,
    val content: String,
    val outgoing: Boolean,
    val timestamp: String,
    val state: String,
    val route: String? = null,
)

data class MobileUiState(
    val connection: ConnectionState = ConnectionState.Offline,
    val identityHash: String = "",
    val deliveryHash: String = "",
    val daemonVersion: String = "",
    val hubAddress: String? = null,
    val peerCount: Int = 0,
    val linkCount: Int = 0,
    val conversations: List<ConversationCard> = emptyList(),
    val people: List<PersonCard> = emptyList(),
    val selectedConversation: ConversationCard? = null,
    val messages: List<MessageCard> = emptyList(),
    val transportSummary: String = "Local node stopped",
    val bluetoothSummary: String = "Bluetooth scan not started",
    val usbSummary: String = "Scanning for radio",
    val rnodeCandidates: List<RNodeCandidate> = emptyList(),
    val usbAvailable: Boolean = false,
    val rxPackets: Long = 0,
    val txPackets: Long = 0,
    val lastRefresh: String = "Not yet",
    val notice: String? = null,
    val isSending: Boolean = false,
    val draft: String = "",
    val lastQueuedMessageId: String? = null,
    val pageSource: String = "",
    val pageLoading: Boolean = false,
    val pageError: String? = null,
    val pageAddress: String = "",
)

object MobileTestTags {
    const val IdentityAnchor = "messages.identity-anchor"
    const val Conversations = "messages.conversations"
    const val Composer = "messages.composer"
    const val Send = "messages.send"
    const val PreviewLabel = "preview.label"
    const val Setup = "network.connection-setup"
    fun tab(label: String) = "tab.${label.lowercase()}"
    fun conversation(hash: String) = "messages.conversation.$hash"
}

private enum class Destination(val label: String, val icon: ImageVector) {
    Messages("Messages", Icons.Default.Mail),
    People("People", Icons.Default.Group),
    Network("Network", Icons.Default.Hub),
    More("More", Icons.Default.MoreHoriz),
}

private val Ink = Color(0xFF0A1118)
private val Panel = Color(0xFF121D27)
private val PanelRaised = Color(0xFF192735)
private val Paper = Color(0xFFEAF0F2)
private val Mist = Color(0xFF91A3AE)
private val Signal = Color(0xFFFFB45A)
private val Cyan = Color(0xFF6AD8D6)
private val Danger = Color(0xFFFF7C76)

@Composable
fun StyreneMobileApp(
    state: MobileUiState,
    onAnnounce: () -> Unit,
    onRefresh: () -> Unit,
    onOpenConversation: (ConversationCard) -> Unit,
    onOpenPerson: (PersonCard) -> Unit,
    onCloseConversation: () -> Unit,
    onDraftChanged: (String) -> Unit,
    onSend: () -> Unit,
    onScanRnodes: () -> Unit,
    onConnectRnode: (RNodeCandidate) -> Unit,
    onUseUsb: () -> Unit,
    onRetryNode: () -> Unit,
    onBrowsePage: (String, String) -> Unit,
) {
    var destination by remember { mutableStateOf(Destination.Messages) }
    var showIdentity by remember { mutableStateOf(false) }
    val openConversation = state.selectedConversation
    val closeConversation = {
        onCloseConversation()
    }
    BackHandler(enabled = openConversation != null, onBack = closeConversation)

    MaterialTheme(
        colorScheme = MaterialTheme.colorScheme.copy(
            primary = Signal,
            secondary = Cyan,
            background = Ink,
            surface = Panel,
            onPrimary = Ink,
            onBackground = Paper,
            onSurface = Paper,
        ),
    ) {
        Scaffold(
            containerColor = Ink,
            topBar = {
                AppHeader(
                    state = state,
                    title = openConversation?.name ?: destination.label,
                    showBack = openConversation != null,
                    onBack = closeConversation,
                    messagingRoot = openConversation == null && destination == Destination.Messages,
                    onIdentity = { showIdentity = true },
                    onCompose = { destination = Destination.People },
                )
            },
            bottomBar = {
                if (openConversation == null) {
                    NavigationBar(containerColor = Panel) {
                        Destination.entries.forEach { item ->
                            NavigationBarItem(
                                modifier = Modifier.testTag(MobileTestTags.tab(item.label)),
                                selected = destination == item,
                                onClick = { destination = item },
                                icon = { Icon(item.icon, contentDescription = null) },
                                label = { Text(item.label) },
                            )
                        }
                    }
                }
            },
        ) { padding ->
            Box(Modifier.padding(padding).fillMaxSize()) {
                when {
                    openConversation != null -> ConversationScreen(
                        conversation = openConversation!!,
                        messages = state.messages,
                        sending = state.isSending,
                        draft = state.draft,
                        onDraftChanged = onDraftChanged,
                        onSend = onSend,
                    )
                    destination == Destination.Messages -> MessagesScreen(
                        state = state,
                        onRefresh = onRefresh,
                        onOpen = onOpenConversation,
                    )
                    destination == Destination.People -> PeopleScreen(
                        state = state,
                        onMessage = { person ->
                            val conversation = ConversationCard(
                                hash = person.hash,
                                name = person.name,
                                preview = "New conversation",
                                timestamp = "",
                                unread = 0,
                                isPreview = person.preview,
                            )
                            onOpenPerson(person)
                        },
                    )
                    destination == Destination.Network -> NetworkScreen(
                        state = state,
                        onAnnounce = onAnnounce,
                        onRefresh = onRefresh,
                        onScanRnodes = onScanRnodes,
                        onConnectRnode = onConnectRnode,
                        onUseUsb = onUseUsb,
                        onRetryNode = onRetryNode,
                    )
                    else -> MoreScreen(state, onBrowsePage)
                }
            }
        }
        if (showIdentity) {
            IdentityDialog(state = state, onDismiss = { showIdentity = false })
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AppHeader(
    state: MobileUiState,
    title: String,
    showBack: Boolean,
    onBack: () -> Unit,
    messagingRoot: Boolean,
    onIdentity: () -> Unit,
    onCompose: () -> Unit,
) {
    TopAppBar(
        title = {
            Column {
                Text(title, fontWeight = FontWeight.Bold, fontSize = 19.sp, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    if (messagingRoot) "YOU · ${if (state.deliveryHash.isBlank()) "NOT ROUTABLE" else shortPublicHash(state.deliveryHash)}" else state.connection.label.uppercase(),
                    color = connectionColor(state.connection),
                    fontFamily = FontFamily.Monospace,
                    fontSize = 10.sp,
                    letterSpacing = 1.5.sp,
                )
            }
        },
        navigationIcon = {
            if (showBack) {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                }
            } else if (messagingRoot) {
                Box(Modifier.padding(start = 14.dp).testTag(MobileTestTags.IdentityAnchor).clickable(onClick = onIdentity)) {
                    IdentityGlyph("You", 38.dp)
                }
            } else {
                Box(Modifier.padding(start = 18.dp).size(30.dp).clip(CircleShape).background(Signal), contentAlignment = Alignment.Center) {
                    Text("S", color = Ink, fontWeight = FontWeight.Black)
                }
                Spacer(Modifier.width(10.dp))
            }
        },
        actions = {
            if (messagingRoot) {
                IconButton(onClick = onCompose) {
                    Icon(Icons.Default.Edit, contentDescription = "New message", tint = Ink, modifier = Modifier.clip(RoundedCornerShape(12.dp)).background(Signal).padding(9.dp))
                }
            } else {
                Box(Modifier.padding(end = 16.dp).size(10.dp).clip(CircleShape).background(connectionColor(state.connection)))
            }
        },
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = Ink,
            titleContentColor = Paper,
            navigationIconContentColor = Paper,
        ),
    )
}

@Composable
private fun MessagesScreen(
    state: MobileUiState,
    onRefresh: () -> Unit,
    onOpen: (ConversationCard) -> Unit,
) {
    val rows = state.conversations.ifEmpty { previewConversations() }
    LazyColumn(
        modifier = Modifier.fillMaxSize().testTag(MobileTestTags.Conversations),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            InboxStatusRow(state, rows.sumOf { it.unread }, onRefresh)
        }
        item {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text("CONVERSATIONS", style = MaterialTheme.typography.labelMedium, color = Mist)
                Spacer(Modifier.weight(1f))
                if (state.conversations.isEmpty()) PreviewBadge()
            }
        }
        items(rows, key = { it.hash }) { conversation ->
            ConversationRow(conversation, onOpen)
        }
        state.notice?.let { notice ->
            item { InlineNotice(notice) }
        }
    }
}

@Composable
private fun ConversationRow(conversation: ConversationCard, onOpen: (ConversationCard) -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth().testTag(MobileTestTags.conversation(conversation.hash))
            .clickable { onOpen(conversation) },
        colors = CardDefaults.cardColors(containerColor = Panel),
        shape = RoundedCornerShape(16.dp),
    ) {
        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
            IdentityGlyph(conversation.name)
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(conversation.name, fontWeight = FontWeight.SemiBold, color = Paper)
                    if (conversation.isPreview) {
                        Spacer(Modifier.width(8.dp))
                        PreviewBadge()
                    }
                }
                Spacer(Modifier.height(4.dp))
                Text(
                    conversation.preview,
                    color = Mist,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Column(horizontalAlignment = Alignment.End) {
                Text(conversation.timestamp, color = Mist, fontSize = 11.sp)
                if (conversation.unread > 0) {
                    Spacer(Modifier.height(8.dp))
                    Box(
                        Modifier.clip(CircleShape).background(Signal).padding(horizontal = 8.dp, vertical = 3.dp),
                    ) {
                        Text("${conversation.unread}", color = Ink, fontWeight = FontWeight.Bold, fontSize = 11.sp)
                    }
                }
            }
        }
    }
}

@Composable
private fun ConversationScreen(
    conversation: ConversationCard,
    messages: List<MessageCard>,
    sending: Boolean,
    draft: String,
    onDraftChanged: (String) -> Unit,
    onSend: () -> Unit,
) {
    var showAttachments by remember { mutableStateOf(false) }
    var showDelivery by remember { mutableStateOf(false) }
    val submit = {
        if (draft.isNotBlank() && !sending) {
            onSend()
        }
    }
    Column(Modifier.fillMaxSize()) {
        if (conversation.isPreview) {
            Row(
                Modifier.fillMaxWidth().background(Signal.copy(alpha = 0.12f)).padding(12.dp),
                horizontalArrangement = Arrangement.Center,
            ) {
                Text("PREVIEW THREAD  •  NO PACKETS WILL BE SENT", color = Signal, fontSize = 11.sp)
            }
        }
        LazyColumn(
            modifier = Modifier.weight(1f).fillMaxWidth(),
            contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            items(messages, key = { it.id }) { message -> MessageBubble(message) }
            if (messages.isEmpty()) {
                item { EmptyState("No messages yet", "Write the first message when a route is available.") }
            }
        }
        Row(
            Modifier.fillMaxWidth().background(Panel).padding(start = 58.dp, top = 6.dp),
        ) {
            TextButton(onClick = { showDelivery = true }) {
                Text("Direct · delivery options", color = Mist, fontSize = 11.sp)
            }
        }
        Row(
            Modifier.fillMaxWidth().background(Panel).padding(12.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            IconButton(
                onClick = { showAttachments = true },
                modifier = Modifier.clip(RoundedCornerShape(12.dp)).background(PanelRaised),
            ) {
                Icon(Icons.Default.AttachFile, contentDescription = "Attachments", tint = Paper)
            }
            Spacer(Modifier.width(8.dp))
            OutlinedTextField(
                value = draft,
                onValueChange = onDraftChanged,
                modifier = Modifier.weight(1f).testTag(MobileTestTags.Composer),
                placeholder = { Text("Message ${conversation.name}") },
                maxLines = 4,
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                keyboardActions = KeyboardActions(onSend = { submit() }),
            )
            Spacer(Modifier.width(8.dp))
            IconButton(
                onClick = submit,
                enabled = draft.isNotBlank() && !sending,
                modifier = Modifier.testTag(MobileTestTags.Send).clip(CircleShape).background(Signal),
            ) {
                Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Send", tint = Ink)
            }
        }
    }
    if (showAttachments) {
        AlertDialog(
            onDismissRequest = { showAttachments = false },
            title = { Text("Attachments are not available yet") },
            text = { Text("LXMF attachments are supported by the daemon, but attachment transfer is not exported through the mobile API.", color = Mist) },
            confirmButton = { TextButton(onClick = { showAttachments = false }) { Text("OK") } },
            containerColor = PanelRaised,
        )
    }
    if (showDelivery) {
        DeliveryOptionsDialog(onDismiss = { showDelivery = false })
    }
}

@Composable
private fun MessageBubble(message: MessageCard) {
    val showRouteEvidence = LocalContext.current.getSharedPreferences("mobile_ui", android.content.Context.MODE_PRIVATE)
        .getBoolean("route_evidence", true)
    Row(
        Modifier.fillMaxWidth(),
        horizontalArrangement = if (message.outgoing) Arrangement.End else Arrangement.Start,
    ) {
        Column(
            Modifier.fillMaxWidth(0.82f).clip(
                RoundedCornerShape(
                    topStart = 20.dp,
                    topEnd = 20.dp,
                    bottomStart = if (message.outgoing) 20.dp else 5.dp,
                    bottomEnd = if (message.outgoing) 5.dp else 20.dp,
                ),
            ).background(if (message.outgoing) Signal else PanelRaised).padding(14.dp),
        ) {
            Text(message.content, color = if (message.outgoing) Ink else Paper)
            Spacer(Modifier.height(6.dp))
            Text(
                "${message.timestamp}  •  ${message.state}",
                color = if (message.outgoing) Ink.copy(alpha = 0.62f) else Mist,
                fontSize = 10.sp,
                fontFamily = FontFamily.Monospace,
            )
            message.route?.takeIf { showRouteEvidence }?.let { route ->
                Spacer(Modifier.height(4.dp))
                Text(
                    "⌁ $route${if (message.id.startsWith("preview")) " · PREVIEW" else ""}",
                    color = if (message.outgoing) Ink.copy(alpha = .62f) else Cyan,
                    fontSize = 9.sp,
                    fontFamily = FontFamily.Monospace,
                )
            }
        }
    }
}

@Composable
private fun PeopleScreen(state: MobileUiState, onMessage: (PersonCard) -> Unit) {
    var savedOnly by remember { mutableStateOf(true) }
    var selected by remember { mutableStateOf<PersonCard?>(null) }
    val source = state.people.ifEmpty { previewPeople() }
    val people = if (savedOnly) source.filter { it.saved } else source

    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                FilterChip(selected = savedOnly, onClick = { savedOnly = true }, label = { Text("Contacts") })
                FilterChip(selected = !savedOnly, onClick = { savedOnly = false }, label = { Text("Discovered") })
            }
        }
        if (people.isEmpty()) {
            item { EmptyState("No saved contacts", "Discovered identities can be saved after verification.") }
        }
        items(people, key = { it.hash }) { person ->
            Card(
                modifier = Modifier.fillMaxWidth().clickable { selected = person },
                colors = CardDefaults.cardColors(containerColor = Panel),
                shape = RoundedCornerShape(16.dp),
            ) {
                Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    IdentityGlyph(person.name)
                    Spacer(Modifier.width(14.dp))
                    Column(Modifier.weight(1f)) {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            Text(person.name, color = Paper, fontWeight = FontWeight.SemiBold)
                            if (person.preview) {
                                Spacer(Modifier.width(8.dp))
                                PreviewBadge()
                            }
                        }
                        Text(person.detail, color = Mist, fontSize = 13.sp)
                    }
                    Icon(Icons.Default.ChevronRight, contentDescription = null, tint = Mist)
                }
            }
        }
    }
    selected?.let {
        PersonDialog(
            person = it,
            onDismiss = { selected = null },
            onMessage = {
                selected = null
                onMessage(it)
            },
        )
    }
}

@Composable
private fun PersonDialog(person: PersonCard, onDismiss: () -> Unit, onMessage: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { IdentityGlyph(person.name) },
        title = { Text(person.name) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(person.detail, color = Mist)
                Text(person.hash, fontFamily = FontFamily.Monospace, fontSize = 12.sp)
                HorizontalDivider(color = Mist.copy(alpha = 0.25f))
                Text("Discovery is not connectivity. Route and link evidence will appear here when the mobile API exposes it.")
            }
        },
        confirmButton = { TextButton(onClick = onMessage) { Text("Message") } },
        dismissButton = {
            TextButton(onClick = {}, enabled = false) {
                Text(if (person.saved) "Edit unavailable" else "Save unavailable")
            }
        },
        containerColor = PanelRaised,
    )
}

@Composable
private fun NetworkScreen(
    state: MobileUiState,
    onAnnounce: () -> Unit,
    onRefresh: () -> Unit,
    onScanRnodes: () -> Unit,
    onConnectRnode: (RNodeCandidate) -> Unit,
    onUseUsb: () -> Unit,
    onRetryNode: () -> Unit,
) {
    val nodeAvailable = state.connection != ConnectionState.Offline &&
        state.connection != ConnectionState.Starting
    LazyColumn(
        modifier = Modifier.fillMaxSize().testTag(MobileTestTags.Setup),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            SignalBrief(
                eyebrow = "NETWORK POSTURE",
                title = state.connection.label,
                detail = "${state.peerCount} peers  •  ${state.linkCount} links  •  refreshed ${state.lastRefresh.lowercase()}",
                action = if (state.connection == ConnectionState.Offline) "Start" else "Refresh",
                onAction = if (state.connection == ConnectionState.Offline) onRetryNode else onRefresh,
                enabled = state.connection != ConnectionState.Starting,
            )
        }
        item { MeshPathCard(state) }
        item {
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                Button(
                    onClick = onAnnounce,
                    modifier = Modifier.weight(1f),
                    colors = ButtonDefaults.buttonColors(containerColor = Signal, contentColor = Ink),
                    enabled = nodeAvailable,
                ) {
                    Icon(Icons.Default.Bolt, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text("Announce")
                }
                OutlinedButton(onClick = onRefresh, modifier = Modifier.weight(1f), enabled = nodeAvailable) {
                    Icon(Icons.Default.Refresh, contentDescription = null)
                    Spacer(Modifier.width(8.dp))
                    Text("Observe")
                }
            }
        }
        item { SectionLabel("INTERFACES") }
        item {
            InterfaceCard(
                icon = Icons.Default.Radio,
                title = "RNode radio",
                state = state.transportSummary,
                detail = "RX ${state.rxPackets}  •  TX ${state.txPackets}",
                action = "Scan",
                onAction = onScanRnodes,
            )
        }
        item {
            InterfaceCard(
                icon = Icons.Default.Radio,
                title = "Bluetooth",
                state = state.bluetoothSummary,
                detail = "Preferred RNode bearer",
                action = "Scan",
                onAction = onScanRnodes,
            )
        }
        items(state.rnodeCandidates, key = { it.id }) { candidate ->
            InterfaceCard(
                icon = Icons.Default.Radio,
                title = candidate.name,
                state = if (candidate.paired) "Paired RNode" else "Approval and pairing required",
                detail = "Bluetooth device ${candidate.id.takeLast(5)}",
                action = "Connect",
                onAction = { onConnectRnode(candidate) },
            )
        }
        item {
            InterfaceCard(
                icon = Icons.Default.Settings,
                title = "USB fallback",
                state = state.usbSummary,
                detail = "USB never preempts Bluetooth",
                action = if (state.usbAvailable) "Use USB" else null,
                onAction = onUseUsb,
            )
        }
        item {
            InterfaceCard(
                icon = Icons.Default.Hub,
                title = "Direct TCP",
                state = state.hubAddress ?: "No active profile",
                detail = if (state.hubAddress == null) {
                    "Not configured for this host session"
                } else {
                    "Configured when the embedded node started"
                },
                action = null,
                onAction = {},
            )
        }
        item { FieldMapPreview() }
        state.notice?.let { item { InlineNotice(it) } }
    }
}

@Composable
private fun MeshPathCard(state: MobileUiState) {
    Card(colors = CardDefaults.cardColors(containerColor = Panel), shape = RoundedCornerShape(22.dp)) {
        Column(Modifier.padding(18.dp)) {
            Text("ACTIVE PATH", color = Mist, fontSize = 11.sp, letterSpacing = 1.2.sp)
            Spacer(Modifier.height(18.dp))
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                PathNode(Icons.Default.AccountCircle, "PHONE", state.connection != ConnectionState.Offline)
                PathLine(state.connection != ConnectionState.Offline)
                PathNode(Icons.Default.Radio, "RADIO", state.connection == ConnectionState.Connected)
                PathLine(state.linkCount > 0)
                PathNode(Icons.Default.Explore, "MESH", state.linkCount > 0)
            }
            Spacer(Modifier.height(16.dp))
            Text(state.bluetoothSummary, color = Mist, fontSize = 12.sp)
        }
    }
}

@Composable
private fun PathNode(icon: ImageVector, label: String, active: Boolean) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            Modifier.size(42.dp).clip(CircleShape).background(if (active) Signal else PanelRaised),
            contentAlignment = Alignment.Center,
        ) {
            Icon(icon, contentDescription = null, tint = if (active) Ink else Mist)
        }
        Spacer(Modifier.height(6.dp))
        Text(label, color = if (active) Paper else Mist, fontSize = 9.sp, fontFamily = FontFamily.Monospace)
    }
}

@Composable
private fun PathLine(active: Boolean) {
    Box(
        Modifier.width(52.dp).height(2.dp).background(if (active) Signal else Mist.copy(alpha = 0.22f)),
    )
}

@Composable
private fun FieldMapPreview() {
    Card(colors = CardDefaults.cardColors(containerColor = Panel), shape = RoundedCornerShape(22.dp)) {
        Column(Modifier.padding(18.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Default.Map, contentDescription = null, tint = Cyan)
                Spacer(Modifier.width(10.dp))
                Text("Field map", fontWeight = FontWeight.SemiBold)
                Spacer(Modifier.weight(1f))
                PreviewBadge()
            }
            Spacer(Modifier.height(12.dp))
            Canvas(
                Modifier.fillMaxWidth().height(150.dp).clip(RoundedCornerShape(16.dp)).background(Ink),
            ) {
                val grid = Mist.copy(alpha = 0.12f)
                repeat(5) { index ->
                    val x = size.width * index / 4
                    drawLine(grid, start = androidx.compose.ui.geometry.Offset(x, 0f), end = androidx.compose.ui.geometry.Offset(x, size.height))
                }
                repeat(4) { index ->
                    val y = size.height * index / 3
                    drawLine(grid, start = androidx.compose.ui.geometry.Offset(0f, y), end = androidx.compose.ui.geometry.Offset(size.width, y))
                }
                val points = listOf(0.18f to 0.68f, 0.52f to 0.38f, 0.81f to 0.58f)
                drawLine(Signal.copy(alpha = 0.5f), androidx.compose.ui.geometry.Offset(size.width * .18f, size.height * .68f), androidx.compose.ui.geometry.Offset(size.width * .52f, size.height * .38f), 3f)
                drawLine(Cyan.copy(alpha = 0.5f), androidx.compose.ui.geometry.Offset(size.width * .52f, size.height * .38f), androidx.compose.ui.geometry.Offset(size.width * .81f, size.height * .58f), 3f)
                points.forEachIndexed { index, point ->
                    drawCircle(if (index == 1) Signal else Cyan, 10f, androidx.compose.ui.geometry.Offset(size.width * point.first, size.height * point.second))
                }
            }
            Spacer(Modifier.height(10.dp))
            Text("Location and route are separate observations. Live map telemetry is not yet exported to mobile.", color = Mist, fontSize = 12.sp)
        }
    }
}

@Composable
private fun MoreScreen(state: MobileUiState, onBrowsePage: (String, String) -> Unit) {
    var detail by remember { mutableStateOf<MoreItem?>(null) }
    val items = listOf(
        MoreItem("Identity", "Public hashes and secure custody", Icons.Default.Security),
        MoreItem("Propagation", "Background delivery and sync", Icons.Default.Sync),
        MoreItem("Pages", "Micron information access", Icons.Default.Description),
        MoreItem("Settings", "Connections, notifications, appearance", Icons.Default.Settings),
        MoreItem("Diagnostics", "Redacted runtime evidence", Icons.Default.Terminal),
        MoreItem("About", "Capabilities and build information", Icons.Default.Info),
    )
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        item {
            IdentityCard(state)
        }
        items(items) { item ->
            Card(
                modifier = Modifier.fillMaxWidth().clickable { detail = item },
                colors = CardDefaults.cardColors(containerColor = Panel),
                shape = RoundedCornerShape(16.dp),
            ) {
                Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(item.icon, contentDescription = null, tint = Signal)
                    Spacer(Modifier.width(14.dp))
                    Column(Modifier.weight(1f)) {
                        Text(item.title, fontWeight = FontWeight.SemiBold)
                        Text(item.subtitle, color = Mist, fontSize = 12.sp)
                    }
                    Icon(Icons.Default.ChevronRight, contentDescription = null, tint = Mist)
                }
            }
        }
    }
    detail?.let { item ->
        when (item.title) {
            "Identity" -> IdentityDialog(state, onDismiss = { detail = null })
            "Pages" -> PagesDialog(state, onBrowsePage, onDismiss = { detail = null })
            "Settings" -> SettingsDialog(onDismiss = { detail = null })
            else -> MoreDialog(item, state, onDismiss = { detail = null })
        }
    }
}

private data class MoreItem(val title: String, val subtitle: String, val icon: ImageVector)

@Composable
private fun MoreDialog(item: MoreItem, state: MobileUiState, onDismiss: () -> Unit) {
    val body = when (item.title) {
        "Propagation" -> "No propagation peer configured. Production UI will show last sync, queued transfers, checkpoints, and failures without conflating the legacy local queue."
        "Diagnostics" -> "Styrene ${state.daemonVersion.ifBlank { "not running" }}\n${state.transportSummary}\nRX ${state.rxPackets} / TX ${state.txPackets}\n\nExports must be bounded and redact keys, payloads, and credentials."
        else -> "Native Styrene compact communicator mockup. Capability states come from the daemon; preview data is always labeled."
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { Icon(item.icon, contentDescription = null, tint = Signal) },
        title = { Text(item.title) },
        text = { Text(body, color = Mist) },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Done") } },
        containerColor = PanelRaised,
    )
}

@Composable
private fun IdentityDialog(state: MobileUiState, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val clipboard = context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
    val publicHash = state.deliveryHash.ifBlank { state.identityHash }
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = { IdentityGlyph("You", 52.dp) },
        title = { Text("Your Styrene identity") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("IDENTITY", color = Mist, fontSize = 10.sp, fontFamily = FontFamily.Monospace)
                Text(state.identityHash.ifBlank { "Created when the node starts" }, fontSize = 12.sp, fontFamily = FontFamily.Monospace)
                Text("LXMF DELIVERY", color = Mist, fontSize = 10.sp, fontFamily = FontFamily.Monospace)
                Text(state.deliveryHash.ifBlank { "No routable delivery destination" }, fontSize = 12.sp, fontFamily = FontFamily.Monospace)
                Text("Your public hashes can be shared. Private key material never appears here.", color = Mist, fontSize = 12.sp)
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Done") } },
        dismissButton = {
            TextButton(
                onClick = { clipboard.setPrimaryClip(android.content.ClipData.newPlainText("Styrene public hash", publicHash)) },
                enabled = publicHash.isNotBlank(),
            ) { Text("Copy public hash") }
        },
        containerColor = PanelRaised,
    )
}

@Composable
private fun PagesDialog(state: MobileUiState, onBrowsePage: (String, String) -> Unit, onDismiss: () -> Unit) {
    val preferences = LocalContext.current.getSharedPreferences("mobile_ui", android.content.Context.MODE_PRIVATE)
    val pagesEnabled = preferences.getBoolean("experimental_pages", true)
    var host by rememberSaveable { mutableStateOf(preferences.getString("page_host", "").orEmpty()) }
    var path by rememberSaveable { mutableStateOf(preferences.getString("page_path", "/page/index.mu").orEmpty()) }
    val nodeAvailable = state.connection != ConnectionState.Offline && state.connection != ConnectionState.Starting
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Micron pages") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text("EXPERIMENTAL · BASIC SOURCE API", color = Signal, fontSize = 10.sp, fontFamily = FontFamily.Monospace)
                Text("Fetches raw source. Rendering, links, forms, files, and page-host discovery require typed mobile page sessions.", color = Mist, fontSize = 12.sp)
                if (!pagesEnabled) { Text("Enable the experimental browser in Settings to fetch pages.", color = Signal, fontSize = 12.sp) }
                OutlinedTextField(host, {
                    host = it
                    preferences.edit().putString("page_host", it).apply()
                }, label = { Text("Destination hash") }, singleLine = true)
                OutlinedTextField(path, {
                    path = it
                    preferences.edit().putString("page_path", it).apply()
                }, label = { Text("Native page path") }, singleLine = true)
                state.pageError?.let { Text(it, color = Danger, fontSize = 12.sp) }
                if (state.pageSource.isNotBlank()) {
                    Text(state.pageAddress, color = Cyan, fontFamily = FontFamily.Monospace, fontSize = 10.sp)
                    Text(
                        state.pageSource,
                        modifier = Modifier.fillMaxWidth(),
                        fontFamily = FontFamily.Monospace,
                        fontSize = 11.sp,
                    )
                }
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onBrowsePage(host, path) },
                enabled = pagesEnabled && nodeAvailable && host.isNotBlank() && path.isNotBlank() && !state.pageLoading,
            ) { Text(if (state.pageLoading) "Fetching" else "Fetch") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Close") } },
        containerColor = PanelRaised,
    )
}

@Composable
private fun SettingsDialog(onDismiss: () -> Unit) {
    val preferences = LocalContext.current.getSharedPreferences("mobile_ui", android.content.Context.MODE_PRIVATE)
    var routeEvidence by rememberSaveable { mutableStateOf(preferences.getBoolean("route_evidence", true)) }
    var pages by rememberSaveable { mutableStateOf(preferences.getBoolean("experimental_pages", true)) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Settings and capabilities") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                SettingSwitch("Show route evidence", "Local display preference", routeEvidence) {
                    routeEvidence = it
                    preferences.edit().putBoolean("route_evidence", it).apply()
                }
                SettingSwitch("Experimental Micron browser", "Basic raw-source API", pages) {
                    pages = it
                    preferences.edit().putBoolean("experimental_pages", it).apply()
                }
                UnavailableCapability("Attachments", "Requires mobile attachment transfer API")
                UnavailableCapability("Delivery receipts", "Requires typed lifecycle and receipt evidence")
                UnavailableCapability("Advanced delivery methods", "Requires requested and actual method projection")
                UnavailableCapability("Automatic propagation", "Requires propagation policy and queue state")
                UnavailableCapability("Background receive", "Requires Android foreground-service integration")
                UnavailableCapability("Notifications", "Requires host delivery and conversation mute state")
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Done") } },
        containerColor = PanelRaised,
    )
}

@Composable
private fun SettingSwitch(title: String, detail: String, checked: Boolean, onChecked: (Boolean) -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Column(Modifier.weight(1f)) {
            Text(title, fontSize = 13.sp)
            Text(detail, color = Mist, fontSize = 10.sp)
        }
        Switch(checked = checked, onCheckedChange = onChecked)
    }
}

@Composable
private fun UnavailableCapability(title: String, reason: String) {
    Row(verticalAlignment = Alignment.Top) {
        Text("LOCK", color = Mist, fontSize = 9.sp, fontFamily = FontFamily.Monospace, modifier = Modifier.width(42.dp))
        Column {
            Text(title, color = Paper, fontSize = 13.sp)
            Text(reason, color = Mist, fontSize = 10.sp)
        }
    }
}

@Composable
private fun IdentityCard(state: MobileUiState) {
    val context = LocalContext.current
    val clipboard = context.getSystemService(android.content.Context.CLIPBOARD_SERVICE) as android.content.ClipboardManager
    val publicHash = state.deliveryHash.ifBlank { state.identityHash }
    Card(colors = CardDefaults.cardColors(containerColor = PanelRaised), shape = RoundedCornerShape(16.dp)) {
        Column(Modifier.padding(16.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                IdentityGlyph("You")
                Spacer(Modifier.width(12.dp))
                Column(Modifier.weight(1f)) {
                    Text("Your Styrene identity", fontWeight = FontWeight.Bold)
                    Text(
                        if (state.deliveryHash.isBlank()) "Not routable" else shortPublicHash(state.deliveryHash),
                        color = Mist,
                        fontFamily = FontFamily.Monospace,
                        fontSize = 12.sp,
                    )
                }
                IconButton(
                    onClick = { clipboard.setPrimaryClip(android.content.ClipData.newPlainText("Styrene public hash", publicHash)) },
                    enabled = publicHash.isNotBlank(),
                ) {
                    Icon(Icons.Default.ContentCopy, contentDescription = "Copy public hash", tint = Signal)
                }
            }
        }
    }
}

@Composable
private fun SignalBrief(
    eyebrow: String,
    title: String,
    detail: String,
    action: String,
    onAction: () -> Unit,
    enabled: Boolean = true,
) {
    Card(colors = CardDefaults.cardColors(containerColor = PanelRaised), shape = RoundedCornerShape(20.dp)) {
        Column(Modifier.padding(20.dp)) {
            Text(eyebrow, color = Signal, fontFamily = FontFamily.Monospace, fontSize = 10.sp, letterSpacing = 1.6.sp)
            Spacer(Modifier.height(8.dp))
            Text(title, fontSize = 25.sp, fontWeight = FontWeight.Bold)
            Spacer(Modifier.height(6.dp))
            Text(detail, color = Mist)
            Spacer(Modifier.height(16.dp))
            TextButton(onClick = onAction, enabled = enabled, colors = ButtonDefaults.textButtonColors(contentColor = Signal)) {
                Text(action)
                Spacer(Modifier.width(6.dp))
                Icon(Icons.Default.ChevronRight, contentDescription = null, modifier = Modifier.size(18.dp))
            }
        }
    }
}

@Composable
private fun InboxStatusRow(state: MobileUiState, unread: Int, onRefresh: () -> Unit) {
    val canRefresh = state.connection != ConnectionState.Offline && state.connection != ConnectionState.Starting
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp)).background(Panel).padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(8.dp).clip(CircleShape).background(connectionColor(state.connection)))
        Spacer(Modifier.width(8.dp))
        Text(state.connection.label, fontWeight = FontWeight.SemiBold, fontSize = 12.sp)
        Text(" · ${state.peerCount} peers · $unread unread", color = Mist, fontSize = 12.sp)
        Spacer(Modifier.weight(1f))
        IconButton(onClick = onRefresh, enabled = canRefresh, modifier = Modifier.size(32.dp)) {
            Icon(Icons.Default.Refresh, contentDescription = "Refresh", tint = Signal, modifier = Modifier.size(18.dp))
        }
    }
}

@Composable
private fun DeliveryOptionsDialog(onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Delivery options") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Text("DIRECT · CURRENT MOBILE DEFAULT", color = Signal, fontSize = 10.sp, fontFamily = FontFamily.Monospace)
                Text("The mobile API queues plain text with the default direct method.", color = Mist, fontSize = 12.sp)
                UnavailableCapability("Opportunistic", "Method selection is not exported to mobile")
                UnavailableCapability("Propagated", "Requires propagation state and fallback evidence")
                UnavailableCapability("Paper", "Paper URI outcomes are not exported to mobile")
                Text("Method, bearer, and receipt state are separate. Future route evidence can identify LoRa, public TCP, or a WireGuard peer tunnel.", color = Mist, fontSize = 12.sp)
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text("Done") } },
        containerColor = PanelRaised,
    )
}

@Composable
private fun InterfaceCard(
    icon: ImageVector,
    title: String,
    state: String,
    detail: String,
    action: String?,
    onAction: () -> Unit,
) {
    Card(colors = CardDefaults.cardColors(containerColor = Panel), shape = RoundedCornerShape(16.dp)) {
        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(42.dp).clip(CircleShape).background(Cyan.copy(alpha = .12f)), contentAlignment = Alignment.Center) {
                Icon(icon, contentDescription = null, tint = Cyan)
            }
            Spacer(Modifier.width(14.dp))
            Column(Modifier.weight(1f)) {
                Text(title, fontWeight = FontWeight.SemiBold)
                Text(state, color = Paper, fontSize = 13.sp)
                Text(detail, color = Mist, fontSize = 11.sp)
            }
            action?.let { TextButton(onClick = onAction) { Text(it) } }
        }
    }
}

@Composable
private fun IdentityGlyph(name: String, size: androidx.compose.ui.unit.Dp = 44.dp) {
    Box(
        Modifier.size(size).clip(RoundedCornerShape(12.dp)).background(Cyan.copy(alpha = .16f))
            .border(1.dp, Cyan.copy(alpha = .35f), RoundedCornerShape(12.dp)),
        contentAlignment = Alignment.Center,
    ) {
        Text(name.take(2).uppercase(), color = Cyan, fontFamily = FontFamily.Monospace, fontWeight = FontWeight.Bold)
    }
}

@Composable
private fun SectionLabel(text: String) {
    Text(text, color = Mist, fontSize = 11.sp, letterSpacing = 1.5.sp, fontFamily = FontFamily.Monospace)
}

@Composable
private fun PreviewBadge() {
    Box(Modifier.testTag(MobileTestTags.PreviewLabel).clip(CircleShape).background(Cyan.copy(alpha = .13f)).padding(horizontal = 7.dp, vertical = 3.dp)) {
        Text("PREVIEW", color = Cyan, fontSize = 8.sp, fontFamily = FontFamily.Monospace, letterSpacing = 1.sp)
    }
}

@Composable
private fun InlineNotice(text: String) {
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(14.dp)).background(Signal.copy(alpha = .1f)).padding(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Default.Info, contentDescription = null, tint = Signal, modifier = Modifier.size(18.dp))
        Spacer(Modifier.width(9.dp))
        Text(text, color = Signal, fontSize = 12.sp)
    }
}

@Composable
private fun EmptyState(title: String, detail: String) {
    Column(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(Panel).padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(Icons.Default.Add, contentDescription = null, tint = Mist)
        Spacer(Modifier.height(10.dp))
        Text(title, fontWeight = FontWeight.SemiBold)
        Text(detail, color = Mist, fontSize = 12.sp)
    }
}

private fun connectionColor(state: ConnectionState) = when (state) {
    ConnectionState.Connected -> Cyan
    ConnectionState.Ready -> Signal
    ConnectionState.Starting -> Signal
    ConnectionState.Degraded -> Danger
    ConnectionState.Offline -> Mist
}

private fun shortPublicHash(hash: String) = if (hash.length > 18) {
    "${hash.take(9)}…${hash.takeLast(9)}"
} else {
    hash
}

fun previewConversations() = listOf(
    ConversationCard("preview-red", "Classroom Red", "Meet at the west gate after sunset.", "18:42", 2, true),
    ConversationCard("preview-relay", "Hill Relay", "Propagation window opens in 12 minutes.", "17:06", 0, true),
    ConversationCard("preview-yellow", "Field Team Yellow", "Telemetry bundle received.", "Yesterday", 0, true),
)

fun previewPeople() = listOf(
    PersonCard("7ab9b2e4139d7a915f4b813fd98a2611", "Classroom Red", "Saved contact  •  seen 2m ago", true, true),
    PersonCard("2190f04ad551cee8cd9854ba3d16a977", "Hill Relay", "Discovered  •  2 hops", true, true),
    PersonCard("2a9d603aec973592515f43d112a6e96f", "Unknown 2a9d60", "Announced nearby  •  not verified", false, true),
)

fun previewMessages(seed: String) = listOf(
    MessageCard("$seed-1", "Signal check from the ridge. Can you copy?", false, "18:36", "Received", "Direct · LoRa · 2 hops"),
    MessageCard("$seed-2", "Copy. Direct path is marginal; propagation is available.", true, "18:38", "Delivered", "Direct · Public TCP"),
    MessageCard("$seed-3", "Meet at the west gate after sunset.", false, "18:42", "Received", "Direct · WireGuard peer"),
)
