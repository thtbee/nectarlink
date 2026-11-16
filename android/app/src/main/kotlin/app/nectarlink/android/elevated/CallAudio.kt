// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.annotation.SuppressLint
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioRecord
import android.media.AudioTrack
import java.io.IOException
import java.net.Socket
import kotlin.concurrent.thread

/**
 * A call's sound for a PC (`call.audio`, docs/protocol/calls.md §2.6), in the
 * shell process: only it holds `CALL_AUDIO_INTERCEPTION` (Android 13+). On the
 * connection, after the `CALL` line, the helper answers `OK` or `ERR reason`,
 * then writes the other party's voice (`VOICE_DOWNLINK`) and plays what it
 * reads into the call (`USAGE_CALL_ASSISTANT`), both 16 kHz mono 16-bit PCM.
 * It ends when the call does or the connection closes.
 *
 * What `AudioManager.getCallDownlinkExtractionAudioRecord` and
 * `getCallUplinkInjectionAudioTrack` build, without the `Context` this
 * process doesn't have; the phone's audio HAL must have the telephony Rx/Tx
 * devices, which [interceptable] checks.
 */
internal object CallAudio {
    const val RATE = 16_000

    /** 20 ms of 16-bit mono. */
    const val CHUNK_BYTES = RATE / 50 * 2

    private const val MODE_IN_CALL = 2
    private const val CALL_REDIRECT_PSTN = 1
    private const val VOICE_DOWNLINK = 3
    private const val USAGE_CALL_ASSISTANT = 17

    /** How often (in chunks) the call is checked to still be going: once a second. */
    private const val MODE_CHECK_CHUNKS = 50

    fun serve(client: Socket) {
        val out = client.getOutputStream()
        var record: AudioRecord? = null
        var track: AudioTrack? = null
        try {
            val audio = audioService()
            val mode = { audio.javaClass.getMethod("getMode").invoke(audio) as Int }
            if (mode() != MODE_IN_CALL) return refuse(out, "no call in progress")
            if (!interceptable(audio)) return refuse(out, "this phone doesn't let apps reach a call's sound")
            track = runCatching(::uplinkTrack).getOrElse { return refuse(out, "can't play into the call: $it") }
            record = runCatching(::downlinkRecord).getOrElse { return refuse(out, "can't hear the call: $it") }
            out.write("OK\n".toByteArray())
            out.flush()

            val playing = track
            playing.play()
            thread(isDaemon = true, name = "call-uplink") {
                val chunk = ByteArray(CHUNK_BYTES)
                runCatching {
                    val input = client.getInputStream()
                    while (true) {
                        val n = input.read(chunk)
                        if (n <= 0) break
                        // Never waits: what the call can't take now is dropped, so the PC's
                        // voice stays within the track's buffer (about 80 ms) of live, even
                        // when the PC's microphone runs a little fast or is switched.
                        if (playing.write(chunk, 0, n - n % 2, AudioTrack.WRITE_NON_BLOCKING) < 0) break
                    }
                }
                // The PC stopped: the downlink below ends with the connection.
                runCatching { client.shutdownOutput() }
            }

            record.startRecording()
            val chunk = ByteArray(CHUNK_BYTES)
            var chunks = 0
            while (true) {
                val n = record.read(chunk, 0, chunk.size)
                if (n < 0) break
                out.write(chunk, 0, n)
                out.flush()
                if (++chunks % MODE_CHECK_CHUNKS == 0 && mode() != MODE_IN_CALL) break
            }
        } catch (_: IOException) {
            // The app closed the connection.
        } catch (e: Exception) {
            System.err.println("call sound failed: $e")
        } finally {
            record?.let { runCatching { it.stop() }; it.release() }
            track?.let { runCatching { it.stop() }; it.release() }
            runCatching { client.close() }
        }
    }

    /**
     * Answers whether this phone lets the helper reach a call's sound (`OK` or
     * `ERR reason`), without a call: the app offers `call.audio` only then.
     */
    fun check(client: Socket) {
        client.use {
            val out = it.getOutputStream()
            val ok = runCatching { interceptable(audioService()) }.getOrDefault(false)
            if (ok) {
                out.write("OK\n".toByteArray())
                out.flush()
            } else {
                refuse(out, "this phone doesn't let apps reach a call's sound")
            }
        }
    }

    private fun refuse(out: java.io.OutputStream, why: String) {
        runCatching {
            out.write("ERR $why\n".toByteArray())
            out.flush()
        }
    }

    @SuppressLint("PrivateApi", "DiscouragedPrivateApi")
    private fun audioService(): Any {
        val binder = Class.forName("android.os.ServiceManager")
            .getMethod("getService", String::class.java).invoke(null, "audio") as android.os.IBinder
        return Class.forName("android.media.IAudioService\$Stub")
            .getMethod("asInterface", android.os.IBinder::class.java).invoke(null, binder)!!
    }

    /** Whether the HAL has the telephony Rx/Tx devices (Android 13+). */
    private fun interceptable(audio: Any): Boolean =
        runCatching { audio.javaClass.getMethod("isPstnCallAudioInterceptable").invoke(audio) as Boolean }
            .getOrDefault(false)

    @SuppressLint("PrivateApi", "DiscouragedPrivateApi")
    private fun uplinkTrack(): AudioTrack {
        val attrs = AudioAttributes.Builder().setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
        attrs.javaClass.getMethod("setSystemUsage", Int::class.javaPrimitiveType).invoke(attrs, USAGE_CALL_ASSISTANT)
        val builder = AudioTrack.Builder()
            .setAudioAttributes(attrs.build())
            .setAudioFormat(format(AudioFormat.CHANNEL_OUT_MONO))
            .setBufferSizeInBytes(CHUNK_BYTES * 4)
        builder.javaClass.getMethod("setCallRedirectionMode", Int::class.javaPrimitiveType)
            .invoke(builder, CALL_REDIRECT_PSTN)
        return builder.build().also { check(it.state == AudioTrack.STATE_INITIALIZED) { "not initialized" } }
    }

    @SuppressLint("PrivateApi", "DiscouragedPrivateApi", "MissingPermission")
    private fun downlinkRecord(): AudioRecord {
        val attrs = AudioAttributes.Builder()
        attrs.javaClass.getMethod("setInternalCapturePreset", Int::class.javaPrimitiveType).invoke(attrs, VOICE_DOWNLINK)
        val builder = AudioRecord.Builder()
            .setAudioFormat(format(AudioFormat.CHANNEL_IN_MONO))
            .setBufferSizeInBytes(CHUNK_BYTES * 4)
        builder.javaClass.getMethod("setAudioAttributes", AudioAttributes::class.java).invoke(builder, attrs.build())
        builder.javaClass.getMethod("setCallRedirectionMode", Int::class.javaPrimitiveType)
            .invoke(builder, CALL_REDIRECT_PSTN)
        return builder.build().also { check(it.state == AudioRecord.STATE_INITIALIZED) { "not initialized" } }
    }

    private fun format(channels: Int): AudioFormat =
        AudioFormat.Builder().setSampleRate(RATE).setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setChannelMask(channels).build()
}
