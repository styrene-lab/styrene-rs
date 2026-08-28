package io.styrene.mesh

import android.annotation.SuppressLint
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothDevice
import android.bluetooth.BluetoothGatt
import android.bluetooth.BluetoothGattCallback
import android.bluetooth.BluetoothGattCharacteristic
import android.bluetooth.BluetoothGattDescriptor
import android.bluetooth.BluetoothGattService
import android.bluetooth.BluetoothManager
import android.bluetooth.BluetoothProfile
import android.bluetooth.BluetoothStatusCodes
import android.bluetooth.le.ScanCallback
import android.bluetooth.le.ScanFilter
import android.bluetooth.le.ScanResult
import android.bluetooth.le.ScanSettings
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.ParcelUuid
import java.util.UUID
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

data class RNodeCandidate(
    val id: String,
    val name: String,
    val paired: Boolean,
)

@SuppressLint("MissingPermission")
class AndroidBleRNodeDiscovery(
    private val context: Context,
    private val listener: Listener,
) : AutoCloseable {
    interface Listener {
        fun onState(message: String)
        fun onCandidates(candidates: List<RNodeCandidate>)
        fun onApprovedDevice(device: BluetoothDevice)
        fun onApprovedDeviceUnavailable()
    }

    private val adapter = context.getSystemService(BluetoothManager::class.java)?.adapter
    private val preferences = context.getSharedPreferences(PREFERENCES, Context.MODE_PRIVATE)
    private val approval = RNodeApprovalPolicy(preferences.getString(KEY_APPROVED_ID, null))
    private val handler = Handler(Looper.getMainLooper())
    private val candidates = linkedMapOf<String, BluetoothDevice>()
    private var scanning = false
    private val stopScanTask = Runnable { stopScan() }

    private val scanCallback = object : ScanCallback() {
        override fun onScanResult(callbackType: Int, result: ScanResult) = observe(result.device)

        override fun onBatchScanResults(results: MutableList<ScanResult>) {
            results.forEach { observe(it.device) }
        }

        override fun onScanFailed(errorCode: Int) {
            scanning = false
            listener.onState("Bluetooth scan failed: $errorCode")
            if (approval.hasApproval()) listener.onApprovedDeviceUnavailable()
        }
    }

    private val bondReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == BluetoothAdapter.ACTION_STATE_CHANGED) {
                if (intent.getIntExtra(BluetoothAdapter.EXTRA_STATE, BluetoothAdapter.ERROR) ==
                    BluetoothAdapter.STATE_ON && approval.hasApproval()
                ) {
                    scan()
                }
                return
            }
            if (intent.action != BluetoothDevice.ACTION_BOND_STATE_CHANGED) return
            val device = intent.bluetoothDevice() ?: return
            if (!approval.isApproved(device.address)) return
            when (device.bondState) {
                BluetoothDevice.BOND_BONDED -> {
                    listener.onState("Paired with ${device.displayName()}; reconnecting")
                    listener.onApprovedDevice(device)
                }
                BluetoothDevice.BOND_NONE -> listener.onState("RNode pairing was not completed")
            }
        }
    }

    init {
        val filter = IntentFilter(BluetoothDevice.ACTION_BOND_STATE_CHANGED).apply {
            addAction(BluetoothAdapter.ACTION_STATE_CHANGED)
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            context.registerReceiver(bondReceiver, filter, Context.RECEIVER_EXPORTED)
        } else {
            @Suppress("DEPRECATION")
            context.registerReceiver(bondReceiver, filter)
        }
    }

    fun scan() {
        val bluetoothAdapter = adapter ?: run {
            listener.onState("Bluetooth LE is unavailable")
            return
        }
        if (!bluetoothAdapter.isEnabled) {
            listener.onState("Turn on Bluetooth to find RNodes")
            return
        }
        stopScan(reportEmpty = false)
        candidates.clear()
        listener.onCandidates(emptyList())
        listener.onState("Scanning for Bluetooth RNodes")
        val filter = ScanFilter.Builder().setServiceUuid(ParcelUuid(NUS_SERVICE_UUID)).build()
        val settings = ScanSettings.Builder().setScanMode(ScanSettings.SCAN_MODE_LOW_LATENCY).build()
        val scanner = bluetoothAdapter.bluetoothLeScanner ?: run {
            listener.onState("Bluetooth LE scanner is unavailable")
            return
        }
        scanner.startScan(listOf(filter), settings, scanCallback)
        scanning = true
        handler.postDelayed(stopScanTask, SCAN_DURATION_MS)
    }

    fun approve(id: String) {
        val device = candidates[id] ?: run {
            listener.onState("RNode is no longer discoverable; scan again")
            return
        }
        approval.approve(id)
        preferences.edit().putString(KEY_APPROVED_ID, id).apply()
        stopScan(reportEmpty = false)
        if (device.bondState == BluetoothDevice.BOND_BONDED) {
            listener.onApprovedDevice(device)
        } else {
            listener.onState("Pairing with ${device.displayName()}; enter the RNode PIN")
            if (!device.createBond()) listener.onState("Could not start RNode pairing")
        }
    }

    fun forgetApproval() {
        approval.forget()
        preferences.edit().remove(KEY_APPROVED_ID).apply()
    }

    private fun observe(device: BluetoothDevice) {
        val id = device.address
        if (approval.shouldReconnect(id, device.bondState == BluetoothDevice.BOND_BONDED)) {
            stopScan(reportEmpty = false)
            listener.onApprovedDevice(device)
            return
        }
        candidates[id] = device
        listener.onCandidates(
            candidates.values.map {
                RNodeCandidate(it.address, it.displayName(), it.bondState == BluetoothDevice.BOND_BONDED)
            },
        )
    }

    private fun stopScan(reportEmpty: Boolean = true) {
        handler.removeCallbacks(stopScanTask)
        if (!scanning) return
        runCatching { adapter?.bluetoothLeScanner?.stopScan(scanCallback) }
        scanning = false
        if (reportEmpty && approval.hasApproval()) {
            listener.onState("Approved Bluetooth RNode is not currently available")
            listener.onApprovedDeviceUnavailable()
        } else if (reportEmpty && candidates.isEmpty()) {
            listener.onState("No Bluetooth RNodes found")
        }
    }

    private fun BluetoothDevice.displayName() = name?.takeIf(String::isNotBlank) ?: "RNode"

    @Suppress("DEPRECATION")
    private fun Intent.bluetoothDevice(): BluetoothDevice? = if (Build.VERSION.SDK_INT >= 33) {
        getParcelableExtra(BluetoothDevice.EXTRA_DEVICE, BluetoothDevice::class.java)
    } else {
        getParcelableExtra(BluetoothDevice.EXTRA_DEVICE)
    }

    override fun close() {
        stopScan(reportEmpty = false)
        context.unregisterReceiver(bondReceiver)
    }

    companion object {
        val NUS_SERVICE_UUID: UUID = UUID.fromString("6e400001-b5a3-f393-e0a9-e50e24dcca9e")
        val NUS_RX_UUID: UUID = UUID.fromString("6e400002-b5a3-f393-e0a9-e50e24dcca9e")
        val NUS_TX_UUID: UUID = UUID.fromString("6e400003-b5a3-f393-e0a9-e50e24dcca9e")
        private const val PREFERENCES = "rnode_bluetooth"
        private const val KEY_APPROVED_ID = "approved_peripheral_id"
        private const val SCAN_DURATION_MS = 10_000L
    }
}

@SuppressLint("MissingPermission")
class BluetoothRNodeByteLink private constructor(
    private val gatt: BluetoothGatt,
    private val writeCharacteristic: BluetoothGattCharacteristic,
    private val events: LinkedBlockingQueue<GattEvent>,
    private val notifications: LinkedBlockingQueue<NotificationEvent>,
    private var mtu: Int,
) : RNodeByteLink {
    override val bearerName = "Bluetooth"
    private var pendingRead = byteArrayOf()
    private var pendingOffset = 0

    override fun read(buffer: ByteArray, timeoutMs: Int): Int {
        if (pendingOffset >= pendingRead.size) {
            when (val event = notifications.poll(timeoutMs.toLong(), TimeUnit.MILLISECONDS) ?: return 0) {
                is NotificationEvent.Data -> {
                    pendingRead = event.value
                    pendingOffset = 0
                }
                NotificationEvent.Disconnected -> error("Bluetooth RNode disconnected")
            }
        }
        val count = minOf(buffer.size, pendingRead.size - pendingOffset)
        pendingRead.copyInto(buffer, endIndex = pendingOffset + count, startIndex = pendingOffset)
        pendingOffset += count
        return count
    }

    @Synchronized
    override fun write(data: ByteArray, timeoutMs: Int) {
        val chunkSize = (mtu - ATT_WRITE_OVERHEAD).coerceAtLeast(DEFAULT_WRITE_BYTES)
        var offset = 0
        while (offset < data.size) {
            val end = minOf(offset + chunkSize, data.size)
            val chunk = data.copyOfRange(offset, end)
            val started = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                gatt.writeCharacteristic(
                    writeCharacteristic,
                    chunk,
                    BluetoothGattCharacteristic.WRITE_TYPE_DEFAULT,
                ) == BluetoothStatusCodes.SUCCESS
            } else {
                @Suppress("DEPRECATION")
                writeCharacteristic.value = chunk
                @Suppress("DEPRECATION")
                gatt.writeCharacteristic(writeCharacteristic)
            }
            check(started) { "Bluetooth write could not be started" }
            waitFor<GattEvent.CharacteristicWrite>(events, timeoutMs.toLong())
            offset = end
        }
    }

    override fun close() {
        gatt.disconnect()
        gatt.close()
    }

    companion object {
        private val CCCD_UUID = UUID.fromString("00002902-0000-1000-8000-00805f9b34fb")
        private const val ATT_WRITE_OVERHEAD = 3
        private const val DEFAULT_WRITE_BYTES = 20
        private const val CONNECT_TIMEOUT_MS = 15_000L

        fun open(context: Context, device: BluetoothDevice): BluetoothRNodeByteLink {
            val events = LinkedBlockingQueue<GattEvent>()
            val notifications = LinkedBlockingQueue<NotificationEvent>()
            val callback = RNodeGattCallback(events, notifications)
            val gatt = device.connectGatt(context, false, callback, BluetoothDevice.TRANSPORT_LE)
                ?: error("Bluetooth GATT connection could not be created")
            try {
                waitFor<GattEvent.Connected>(events, CONNECT_TIMEOUT_MS)
                check(gatt.discoverServices()) { "Bluetooth service discovery could not be started" }
                waitFor<GattEvent.ServicesDiscovered>(events, CONNECT_TIMEOUT_MS)
                val service: BluetoothGattService = gatt.getService(AndroidBleRNodeDiscovery.NUS_SERVICE_UUID)
                    ?: error("RNode Nordic UART service is unavailable")
                val writeCharacteristic = service.getCharacteristic(AndroidBleRNodeDiscovery.NUS_RX_UUID)
                    ?: error("RNode Bluetooth write characteristic is unavailable")
                val notifyCharacteristic = service.getCharacteristic(AndroidBleRNodeDiscovery.NUS_TX_UUID)
                    ?: error("RNode Bluetooth notify characteristic is unavailable")
                check(writeCharacteristic.properties and BluetoothGattCharacteristic.PROPERTY_WRITE != 0) {
                    "RNode Bluetooth characteristic does not support write-with-response"
                }
                check(gatt.setCharacteristicNotification(notifyCharacteristic, true)) {
                    "RNode Bluetooth notifications could not be enabled"
                }
                val descriptor = notifyCharacteristic.getDescriptor(CCCD_UUID)
                    ?: error("RNode Bluetooth notification descriptor is unavailable")
                val descriptorStarted = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    gatt.writeDescriptor(descriptor, BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE) ==
                        BluetoothStatusCodes.SUCCESS
                } else {
                    @Suppress("DEPRECATION")
                    descriptor.value = BluetoothGattDescriptor.ENABLE_NOTIFICATION_VALUE
                    @Suppress("DEPRECATION")
                    gatt.writeDescriptor(descriptor)
                }
                check(descriptorStarted) { "RNode Bluetooth notification subscription could not be started" }
                waitFor<GattEvent.DescriptorWrite>(events, CONNECT_TIMEOUT_MS)

                var mtu = DEFAULT_WRITE_BYTES + ATT_WRITE_OVERHEAD
                if (gatt.requestMtu(REQUESTED_MTU)) {
                    mtu = waitFor<GattEvent.MtuChanged>(events, CONNECT_TIMEOUT_MS).mtu
                }
                return BluetoothRNodeByteLink(gatt, writeCharacteristic, events, notifications, mtu)
            } catch (error: Throwable) {
                gatt.disconnect()
                gatt.close()
                throw error
            }
        }

        private const val REQUESTED_MTU = 517
    }
}

private sealed interface GattEvent {
    data object Connected : GattEvent
    data object ServicesDiscovered : GattEvent
    data object DescriptorWrite : GattEvent
    data object CharacteristicWrite : GattEvent
    data class MtuChanged(val mtu: Int) : GattEvent
    data class Failed(val operation: String, val status: Int) : GattEvent
    data object Disconnected : GattEvent
}

private sealed interface NotificationEvent {
    data class Data(val value: ByteArray) : NotificationEvent
    data object Disconnected : NotificationEvent
}

private inline fun <reified T : GattEvent> waitFor(
    events: LinkedBlockingQueue<GattEvent>,
    timeoutMs: Long,
): T {
    val deadline = System.currentTimeMillis() + timeoutMs
    while (true) {
        val remaining = deadline - System.currentTimeMillis()
        check(remaining > 0) { "Bluetooth ${T::class.simpleName} timed out" }
        when (val event = events.poll(remaining, TimeUnit.MILLISECONDS)) {
            is T -> return event
            is GattEvent.Failed -> error("Bluetooth ${event.operation} failed with status ${event.status}")
            GattEvent.Disconnected -> error("Bluetooth RNode disconnected")
            null -> error("Bluetooth ${T::class.simpleName} timed out")
            else -> Unit
        }
    }
}

private class RNodeGattCallback(
    private val events: LinkedBlockingQueue<GattEvent>,
    private val notifications: LinkedBlockingQueue<NotificationEvent>,
) : BluetoothGattCallback() {
    override fun onConnectionStateChange(gatt: BluetoothGatt, status: Int, newState: Int) {
        when {
            newState == BluetoothProfile.STATE_DISCONNECTED -> {
                if (status == BluetoothGatt.GATT_SUCCESS) {
                    events += GattEvent.Disconnected
                } else {
                    events += GattEvent.Failed("connection", status)
                }
                notifications += NotificationEvent.Disconnected
            }
            status != BluetoothGatt.GATT_SUCCESS -> events += GattEvent.Failed("connection", status)
            newState == BluetoothProfile.STATE_CONNECTED -> events += GattEvent.Connected
        }
    }

    override fun onServicesDiscovered(gatt: BluetoothGatt, status: Int) {
        events += if (status == BluetoothGatt.GATT_SUCCESS) {
            GattEvent.ServicesDiscovered
        } else {
            GattEvent.Failed("service discovery", status)
        }
    }

    override fun onDescriptorWrite(gatt: BluetoothGatt, descriptor: BluetoothGattDescriptor, status: Int) {
        events += if (status == BluetoothGatt.GATT_SUCCESS) {
            GattEvent.DescriptorWrite
        } else {
            GattEvent.Failed("notification subscription", status)
        }
    }

    override fun onMtuChanged(gatt: BluetoothGatt, mtu: Int, status: Int) {
        events += if (status == BluetoothGatt.GATT_SUCCESS) {
            GattEvent.MtuChanged(mtu)
        } else {
            GattEvent.Failed("MTU negotiation", status)
        }
    }

    override fun onCharacteristicWrite(
        gatt: BluetoothGatt,
        characteristic: BluetoothGattCharacteristic,
        status: Int,
    ) {
        events += if (status == BluetoothGatt.GATT_SUCCESS) {
            GattEvent.CharacteristicWrite
        } else {
            GattEvent.Failed("write", status)
        }
    }

    @Deprecated("Used on Android 12 and earlier")
    @Suppress("DEPRECATION")
    override fun onCharacteristicChanged(gatt: BluetoothGatt, characteristic: BluetoothGattCharacteristic) {
        notifications += NotificationEvent.Data(characteristic.value.copyOf())
    }

    override fun onCharacteristicChanged(
        gatt: BluetoothGatt,
        characteristic: BluetoothGattCharacteristic,
        value: ByteArray,
    ) {
        notifications += NotificationEvent.Data(value.copyOf())
    }
}
