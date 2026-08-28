package io.styrene.mesh

class RNodeApprovalPolicy(approvedId: String?) {
    private var approvedId = approvedId

    @Synchronized
    fun approve(id: String) {
        approvedId = id
    }

    @Synchronized
    fun forget() {
        approvedId = null
    }

    @Synchronized
    fun hasApproval() = approvedId != null

    @Synchronized
    fun isApproved(id: String) = id == approvedId

    fun shouldReconnect(id: String, bonded: Boolean) = bonded && isApproved(id)
}

class RNodeBearerState {
    private var channel: RNodePacketChannel? = null
    private var outbound: RNodeOutboundBuffer? = null

    @Synchronized
    fun outboundBuffer(channel: RNodePacketChannel): RNodeOutboundBuffer {
        if (this.channel !== channel) {
            this.channel = channel
            outbound = RNodeOutboundBuffer(channel)
        }
        return requireNotNull(outbound)
    }

    @Synchronized
    fun clear() {
        channel = null
        outbound = null
    }
}
