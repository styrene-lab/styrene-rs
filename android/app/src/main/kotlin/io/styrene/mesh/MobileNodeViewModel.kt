package io.styrene.mesh

import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider

class MobileNodeViewModel(configuration: MobileNodeConfiguration) : ViewModel() {
    var state by mutableStateOf(MobileUiState())
        private set

    private val holder = MobileNodeStateHolder(
        configuration = configuration,
        factory = UniFfiMobileNodeClientFactory(),
        executor = SerialOperationExecutor(),
        dispatcher = ResultDispatcher { action -> Handler(Looper.getMainLooper()).post(action) },
        delays = ScheduledDelayScheduler(),
        nodeCloser = NodeCloser { client ->
            Thread({ client.close() }, "styrene-node-cleanup").start()
        },
        onStateChanged = { state = it },
    )
    private val bearerState = RNodeBearerState()

    fun boot() = holder.boot()
    fun announce() = holder.announce()
    fun refreshDirectory() = holder.refreshDirectory()
    fun openConversation(conversation: ConversationCard) = holder.openConversation(conversation)
    fun openPerson(person: PersonCard) = holder.openPerson(person)
    fun closeConversation() = holder.closeConversation()
    fun updateDraft(value: String) = holder.updateDraft(value)
    fun sendMessage() = holder.sendMessage()
    fun browsePage(host: String, path: String) = holder.browsePage(host, path)
    fun rnodePacketChannel() = holder.rnodePacketChannel()
    fun rnodeOutboundBuffer(channel: RNodePacketChannel) = bearerState.outboundBuffer(channel)
    fun updateUsbSummary(usbSummary: String, transportSummary: String, available: Boolean = false) =
        holder.updateUsbSummary(usbSummary, transportSummary, available)
    fun updateBluetoothSummary(summary: String) = holder.updateBluetoothSummary(summary)
    fun updateRnodeCandidates(candidates: List<RNodeCandidate>) = holder.updateRnodeCandidates(candidates)
    fun updateRnodeState(message: String, online: Boolean) = holder.updateRnodeState(message, online)
    fun updateRnodeTraffic(rxPackets: Long, txPackets: Long) = holder.updateRnodeTraffic(rxPackets, txPackets)
    fun scheduleRefresh() = holder.scheduleRefresh()

    override fun onCleared() {
        bearerState.clear()
        holder.close()
    }

    companion object {
        fun factory(configuration: MobileNodeConfiguration) = object : ViewModelProvider.Factory {
            @Suppress("UNCHECKED_CAST")
            override fun <T : ViewModel> create(modelClass: Class<T>): T = MobileNodeViewModel(configuration) as T
        }
    }
}
