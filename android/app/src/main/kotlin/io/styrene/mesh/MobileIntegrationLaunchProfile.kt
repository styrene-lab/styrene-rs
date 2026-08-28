package io.styrene.mesh

import java.io.File

data class MobileIntegrationLaunchProfile(
    val id: String,
    val hubAddress: String,
    val displayName: String,
    val resetState: Boolean,
) {
    fun stateRoot(filesDir: File) = filesDir.resolve("integration").resolve(id)

    fun configuration(filesDir: File): MobileNodeConfiguration {
        val root = stateRoot(filesDir)
        return MobileNodeConfiguration(
            configDir = root.resolve("config").absolutePath,
            dataDir = root.resolve("data").absolutePath,
            hubAddress = hubAddress,
            displayName = displayName,
            enableRnodeChannel = false,
        )
    }

    fun reset(filesDir: File) {
        if (resetState && !stateRoot(filesDir).deleteRecursively()) {
            error("Unable to reset integration profile $id")
        }
    }

    companion object {
        const val EXTRA_PROFILE = "io.styrene.mesh.integration.PROFILE"
        const val EXTRA_HUB_ADDRESS = "io.styrene.mesh.integration.HUB_ADDRESS"
        const val EXTRA_DISPLAY_NAME = "io.styrene.mesh.integration.DISPLAY_NAME"
        const val EXTRA_RESET_STATE = "io.styrene.mesh.integration.RESET_STATE"

        private val validId = Regex("[A-Za-z0-9][A-Za-z0-9._-]{0,63}")

        fun parse(
            profileId: String?,
            hubAddress: String?,
            displayName: String?,
            resetState: Boolean,
        ): MobileIntegrationLaunchProfile? {
            if (profileId == null) {
                require(hubAddress == null && displayName == null && !resetState) {
                    "Integration options require $EXTRA_PROFILE"
                }
                return null
            }
            require(validId.matches(profileId)) { "Invalid integration profile ID" }
            val hub = hubAddress?.trim().orEmpty()
            require(hub.isNotEmpty()) { "Integration profile requires $EXTRA_HUB_ADDRESS" }
            val name = displayName?.trim().takeUnless { it.isNullOrEmpty() } ?: "Android $profileId"
            return MobileIntegrationLaunchProfile(profileId, hub, name, resetState)
        }
    }
}
