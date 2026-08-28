package io.styrene.mesh

import android.hardware.usb.UsbDevice
import android.hardware.usb.UsbManager
import com.hoho.android.usbserial.driver.UsbSerialPort
import com.hoho.android.usbserial.driver.UsbSerialProber

interface RNodeByteLink : AutoCloseable {
    val bearerName: String

    fun read(buffer: ByteArray, timeoutMs: Int): Int

    fun write(data: ByteArray, timeoutMs: Int)
}

class UsbRNodeByteLink private constructor(
    private val port: UsbSerialPort,
) : RNodeByteLink {
    override val bearerName = "USB"

    override fun read(buffer: ByteArray, timeoutMs: Int) = port.read(buffer, timeoutMs)

    override fun write(data: ByteArray, timeoutMs: Int) {
        port.write(data, timeoutMs)
    }

    override fun close() = port.close()

    companion object {
        private const val BAUD_RATE = 115_200

        fun open(usbManager: UsbManager, device: UsbDevice): UsbRNodeByteLink {
            val driver = UsbSerialProber.getDefaultProber().findAllDrivers(usbManager)
                .firstOrNull { it.device.deviceId == device.deviceId }
                ?: error("no CP2102 serial driver")
            val connection = usbManager.openDevice(driver.device) ?: error("USB permission unavailable")
            val port = driver.ports.firstOrNull()
                ?: run {
                    connection.close()
                    error("USB device has no serial port")
                }

            try {
                port.open(connection)
                port.setParameters(
                    BAUD_RATE,
                    8,
                    UsbSerialPort.STOPBITS_1,
                    UsbSerialPort.PARITY_NONE,
                )
                return UsbRNodeByteLink(port)
            } catch (error: Throwable) {
                runCatching { port.close() }
                connection.close()
                throw error
            }
        }
    }
}
