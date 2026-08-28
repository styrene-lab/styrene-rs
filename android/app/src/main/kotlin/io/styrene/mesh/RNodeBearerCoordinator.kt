package io.styrene.mesh

class RNodeBearerCoordinator(
    private val node: RNodePacketChannel,
    private val outbound: RNodeOutboundBuffer,
    private val radioProfile: RNodeRadioProfile,
    private val listener: RNodeController.Listener,
    private val onBearerStopped: (String) -> Unit = {},
) : AutoCloseable {
    private var controller: RNodeController? = null
    private var bearerName: String? = null

    @Synchronized
    fun connect(name: String, openLink: () -> RNodeByteLink): Boolean {
        if (controller != null) return false
        lateinit var candidate: RNodeController
        candidate = RNodeController(node, outbound, radioProfile, listener) { release(candidate, name) }
        controller = candidate
        bearerName = name
        candidate.start(name, openLink)
        return true
    }

    @Synchronized
    fun activeBearer(): String? = bearerName

    @Synchronized
    override fun close() {
        val closing = controller
        controller = null
        bearerName = null
        closing?.stop()
    }

    @Synchronized
    private fun release(stopped: RNodeController, name: String) {
        if (controller === stopped) {
            controller = null
            bearerName = null
            onBearerStopped(name)
        }
    }
}
