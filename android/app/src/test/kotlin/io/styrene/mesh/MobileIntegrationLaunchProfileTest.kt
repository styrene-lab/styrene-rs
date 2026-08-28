package io.styrene.mesh

import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNull

class MobileIntegrationLaunchProfileTest {
    @Test
    fun ordinaryLaunchHasNoIntegrationProfile() {
        assertNull(MobileIntegrationLaunchProfile.parse(null, null, null, false))
    }

    @Test
    fun profileCreatesIsolatedHubConfiguration() {
        val profile = MobileIntegrationLaunchProfile.parse(
            profileId = "android-a",
            hubAddress = " 10.0.2.2:4242 ",
            displayName = " Android A ",
            resetState = true,
        )!!

        val configuration = profile.configuration(File("/app/files"))

        assertEquals("/app/files/integration/android-a/config", configuration.configDir)
        assertEquals("/app/files/integration/android-a/data", configuration.dataDir)
        assertEquals("10.0.2.2:4242", configuration.hubAddress)
        assertEquals("Android A", configuration.displayName)
        assertFalse(configuration.enableRnodeChannel)
    }

    @Test
    fun profileRejectsTraversalAndMissingHub() {
        assertFailsWith<IllegalArgumentException> {
            MobileIntegrationLaunchProfile.parse("../node", "10.0.2.2:4242", null, false)
        }
        assertFailsWith<IllegalArgumentException> {
            MobileIntegrationLaunchProfile.parse("android-a", " ", null, false)
        }
    }

    @Test
    fun integrationOptionsRequireProfileId() {
        assertFailsWith<IllegalArgumentException> {
            MobileIntegrationLaunchProfile.parse(null, "10.0.2.2:4242", null, false)
        }
    }
}
