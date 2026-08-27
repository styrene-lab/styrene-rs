package io.styrene.mesh

data class RNodeFrame(val command: Int, val payload: ByteArray)

object RNodeProtocol {
    const val CMD_DATA = 0x00
    const val CMD_FREQUENCY = 0x01
    const val CMD_BANDWIDTH = 0x02
    const val CMD_TX_POWER = 0x03
    const val CMD_SF = 0x04
    const val CMD_CR = 0x05
    const val CMD_RADIO_STATE = 0x06
    const val CMD_DETECT = 0x08
    const val CMD_PLATFORM = 0x48
    const val CMD_MCU = 0x49
    const val CMD_FW_VERSION = 0x50

    const val DETECT_REQUEST = 0x73
    const val DETECT_RESPONSE = 0x46
    const val RADIO_OFF = 0x00
    const val RADIO_ON = 0x01

    private const val FEND = 0xc0
    private const val FESC = 0xdb
    private const val TFEND = 0xdc
    private const val TFESC = 0xdd

    fun frame(command: Int, payload: ByteArray): ByteArray {
        val output = ArrayList<Byte>(payload.size + 4)
        output += FEND.toByte()
        output += command.toByte()
        payload.forEach { byte ->
            when (byte.toInt() and 0xff) {
                FEND -> {
                    output += FESC.toByte()
                    output += TFEND.toByte()
                }
                FESC -> {
                    output += FESC.toByte()
                    output += TFESC.toByte()
                }
                else -> output += byte
            }
        }
        output += FEND.toByte()
        return output.toByteArray()
    }

    fun unsignedInt(value: Long): ByteArray = byteArrayOf(
        (value ushr 24).toByte(),
        (value ushr 16).toByte(),
        (value ushr 8).toByte(),
        value.toByte(),
    )

    fun readUnsignedInt(payload: ByteArray): Long {
        require(payload.size == 4)
        return payload.fold(0L) { value, byte -> (value shl 8) or (byte.toLong() and 0xff) }
    }

    class Decoder {
        private val buffer = ArrayList<Byte>()
        private var inFrame = false
        private var escaped = false

        fun feed(input: ByteArray, length: Int = input.size): List<RNodeFrame> {
            val frames = mutableListOf<RNodeFrame>()
            repeat(length) { index ->
                val value = input[index].toInt() and 0xff
                if (escaped) {
                    escaped = false
                    when (value) {
                        TFEND -> buffer += FEND.toByte()
                        TFESC -> buffer += FESC.toByte()
                    }
                    return@repeat
                }

                when (value) {
                    FEND -> {
                        if (inFrame && buffer.isNotEmpty()) {
                            frames += RNodeFrame(
                                command = buffer.first().toInt() and 0xff,
                                payload = buffer.drop(1).toByteArray(),
                            )
                        }
                        buffer.clear()
                        inFrame = true
                    }
                    FESC -> if (inFrame) escaped = true
                    else -> if (inFrame) buffer += value.toByte()
                }
            }
            return frames
        }
    }
}
