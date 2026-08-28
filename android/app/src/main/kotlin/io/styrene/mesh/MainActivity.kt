package io.styrene.mesh

import android.Manifest
import android.app.PendingIntent
import android.bluetooth.BluetoothDevice
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import android.os.Build
import android.os.Bundle
import android.os.Handler
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.result.contract.ActivityResultContracts
import androidx.activity.viewModels
import androidx.compose.runtime.LaunchedEffect
import androidx.core.content.ContextCompat
import java.util.Locale

class MainActivity : ComponentActivity() {
    private val integrationProfile by lazy {
        MobileIntegrationLaunchProfile.parse(
            profileId = intent.getStringExtra(MobileIntegrationLaunchProfile.EXTRA_PROFILE),
            hubAddress = intent.getStringExtra(MobileIntegrationLaunchProfile.EXTRA_HUB_ADDRESS),
            displayName = intent.getStringExtra(MobileIntegrationLaunchProfile.EXTRA_DISPLAY_NAME),
            resetState = intent.getBooleanExtra(MobileIntegrationLaunchProfile.EXTRA_RESET_STATE, false),
        )
    }
    private val model by viewModels<MobileNodeViewModel> {
        val configuration = integrationProfile?.configuration(filesDir) ?: MobileNodeConfiguration(
            configDir = filesDir.resolve("config").absolutePath,
            dataDir = filesDir.resolve("data").absolutePath,
        )
        MobileNodeViewModel.factory(
            configuration,
        )
    }
    private var rnodeCoordinator: RNodeBearerCoordinator? = null
    private lateinit var bluetoothDiscovery: AndroidBleRNodeDiscovery
    private var permissionRequested = false
    private var lastRxPackets = 0L

    private val bluetoothPermissionLauncher = registerForActivityResult(
        ActivityResultContracts.RequestMultiplePermissions(),
    ) { grants ->
        if (grants.values.all { it }) {
            bluetoothDiscovery.scan()
        } else {
            model.updateBluetoothSummary("Bluetooth permission denied")
        }
    }

    private val usbPermissionReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action != ACTION_USB_PERMISSION) return
            permissionRequested = false
            val device = intent.usbDevice() ?: return
            if (intent.getBooleanExtra(UsbManager.EXTRA_PERMISSION_GRANTED, false)) {
                startRnode(device)
            } else {
                model.updateRnodeState("USB permission denied", false)
            }
        }
    }

    private val usbDeviceReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == UsbManager.ACTION_USB_DEVICE_DETACHED &&
                rnodeCoordinator?.activeBearer() == "USB"
            ) {
                rnodeCoordinator?.close()
                rnodeCoordinator = null
                model.updateRnodeState("USB RNode disconnected", false)
            }
            refreshUsb()
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        if (savedInstanceState == null) integrationProfile?.reset(filesDir)
        registerUsbPermissionReceiver()
        registerUsbDeviceReceiver()
        bluetoothDiscovery = AndroidBleRNodeDiscovery(
            this,
            object : AndroidBleRNodeDiscovery.Listener {
                override fun onState(message: String) = runOnUiThread {
                    model.updateBluetoothSummary(message)
                }

                override fun onCandidates(candidates: List<RNodeCandidate>) = runOnUiThread {
                    model.updateRnodeCandidates(candidates)
                }

                override fun onApprovedDevice(device: BluetoothDevice) {
                    connectBluetooth(device)
                }

                override fun onApprovedDeviceUnavailable() {
                    Handler(Looper.getMainLooper()).postDelayed({
                        if (!isFinishing && !isDestroyed && model.rnodePacketChannel() != null) scanBluetooth()
                    }, BLUETOOTH_RESCAN_DELAY_MS)
                }
            },
        )
        setContent {
            val state = model.state
            LaunchedEffect(state.identityHash) {
                if (state.identityHash.isNotBlank()) {
                    refreshUsb()
                    scanBluetooth()
                }
            }
            StyreneMobileApp(
                state = state,
                onAnnounce = model::announce,
                onRefresh = model::refreshDirectory,
                onOpenConversation = model::openConversation,
                onOpenPerson = model::openPerson,
                onCloseConversation = model::closeConversation,
                onDraftChanged = model::updateDraft,
                onSend = model::sendMessage,
                onScanRnodes = ::scanBluetooth,
                onConnectRnode = { bluetoothDiscovery.approve(it.id) },
                onUseUsb = ::useUsbFallback,
                onRetryNode = model::boot,
                onBrowsePage = model::browsePage,
            )
        }
        refreshUsb()
        model.boot()
    }

    override fun onDestroy() {
        rnodeCoordinator?.close()
        rnodeCoordinator = null
        bluetoothDiscovery.close()
        unregisterReceiver(usbPermissionReceiver)
        unregisterReceiver(usbDeviceReceiver)
        super.onDestroy()
    }

    private fun refreshUsb() {
        val manager = getSystemService(USB_SERVICE) as UsbManager
        val devices = manager.deviceList.values.sortedBy { it.deviceName }
        val heltec = devices.firstOrNull { it.vendorId == 0x10c4 && it.productId == 0xea60 }
        when {
            heltec != null -> {
                model.updateUsbSummary(
                    "Heltec ${usbId(heltec.vendorId, heltec.productId)} detected",
                    "USB fallback available",
                    available = true,
                )
            }
            devices.isEmpty() -> model.updateUsbSummary("No USB radio attached", "Local node ready")
            else -> model.updateUsbSummary("${devices.size} unsupported USB device(s)", "Local node ready")
        }
    }

    private fun useUsbFallback() {
        if (rnodeCoordinator?.activeBearer() != null) {
            model.updateRnodeState("Disconnect the active RNode before selecting USB", false)
            return
        }
        val manager = getSystemService(USB_SERVICE) as UsbManager
        val device = manager.deviceList.values.firstOrNull { it.vendorId == 0x10c4 && it.productId == 0xea60 }
            ?: run {
                model.updateUsbSummary("No USB radio attached", "Local node ready")
                return
            }
        requestUsb(device)
    }

    private fun requestUsb(device: UsbDevice) {
        val manager = getSystemService(USB_SERVICE) as UsbManager
        if (manager.hasPermission(device)) {
            startRnode(device)
            return
        }
        if (permissionRequested) return
        permissionRequested = true
        val permissionIntent = PendingIntent.getBroadcast(
            this,
            0,
            Intent(ACTION_USB_PERMISSION).setPackage(packageName),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_MUTABLE,
        )
        manager.requestPermission(device, permissionIntent)
    }

    private fun startRnode(device: UsbDevice) {
        val coordinator = coordinator() ?: return
        coordinator.connect("USB") {
            UsbRNodeByteLink.open(getSystemService(USB_SERVICE) as UsbManager, device)
        }
    }

    private fun scanBluetooth() {
        if (bluetoothPermissions().all {
                ContextCompat.checkSelfPermission(this, it) == android.content.pm.PackageManager.PERMISSION_GRANTED
            }
        ) {
            bluetoothDiscovery.scan()
        } else {
            bluetoothPermissionLauncher.launch(bluetoothPermissions())
        }
    }

    private fun connectBluetooth(device: BluetoothDevice) {
        val coordinator = coordinator() ?: return
        if (!coordinator.connect("Bluetooth") { BluetoothRNodeByteLink.open(this, device) }) {
            model.updateBluetoothSummary("An RNode bearer is already active")
        }
    }

    private fun coordinator(): RNodeBearerCoordinator? {
        rnodeCoordinator?.let { return it }
        val channel = model.rnodePacketChannel() ?: return null
        return RNodeBearerCoordinator(
            node = channel,
            outbound = model.rnodeOutboundBuffer(channel),
            radioProfile = RNodeRadioProfile.US_915_DEVELOPMENT,
            listener = object : RNodeController.Listener {
                override fun onState(message: String, online: Boolean) {
                    runOnUiThread { model.updateRnodeState(message, online) }
                }

                override fun onTraffic(rxPackets: Long, txPackets: Long) {
                    runOnUiThread {
                        model.updateRnodeTraffic(rxPackets, txPackets)
                        if (rxPackets > lastRxPackets) {
                            lastRxPackets = rxPackets
                            model.scheduleRefresh()
                        }
                    }
                }
            },
            onBearerStopped = { bearer ->
                if (bearer == "Bluetooth") {
                    Handler(Looper.getMainLooper()).postDelayed({
                        if (!isFinishing && !isDestroyed && model.rnodePacketChannel() != null) scanBluetooth()
                    }, BLUETOOTH_RECONNECT_DELAY_MS)
                }
            },
        ).also { rnodeCoordinator = it }
    }

    private fun bluetoothPermissions(): Array<String> = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
        arrayOf(Manifest.permission.BLUETOOTH_SCAN, Manifest.permission.BLUETOOTH_CONNECT)
    } else {
        arrayOf(Manifest.permission.ACCESS_FINE_LOCATION)
    }

    private fun registerUsbPermissionReceiver() {
        val filter = IntentFilter(ACTION_USB_PERMISSION)
        ContextCompat.registerReceiver(
            this,
            usbPermissionReceiver,
            filter,
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
    }

    private fun registerUsbDeviceReceiver() {
        val filter = IntentFilter().apply {
            addAction(UsbManager.ACTION_USB_DEVICE_ATTACHED)
            addAction(UsbManager.ACTION_USB_DEVICE_DETACHED)
        }
        ContextCompat.registerReceiver(
            this,
            usbDeviceReceiver,
            filter,
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
    }

    @Suppress("DEPRECATION")
    private fun Intent.usbDevice(): UsbDevice? = if (Build.VERSION.SDK_INT >= 33) {
        getParcelableExtra(UsbManager.EXTRA_DEVICE, UsbDevice::class.java)
    } else {
        getParcelableExtra(UsbManager.EXTRA_DEVICE)
    }

    private fun usbId(vendorId: Int, productId: Int) = String.format(Locale.US, "%04x:%04x", vendorId, productId)

    companion object {
        private const val ACTION_USB_PERMISSION = "io.styrene.mesh.USB_PERMISSION"
        private const val BLUETOOTH_RECONNECT_DELAY_MS = 1_000L
        private const val BLUETOOTH_RESCAN_DELAY_MS = 5_000L
    }
}
