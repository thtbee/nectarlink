// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.util.Log
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeUnit
import kotlin.concurrent.thread

/**
 * A PC's "Test call audio" (`call.audio.test`, docs/protocol/calls.md §2.6): what its
 * microphone sends plays on the phone's speaker, so its user hears what the phone got.
 * The core sends the same sound back to the PC. It never touches a call's sound
 * ([CallAudioService]): no call, no Elevated, an ordinary media [AudioTrack].
 */
object CallAudioTest {
    private const val TAG = "CallAudioTest"
    private const val RATE = 16_000

    /** What the speaker's queue holds before it drops: about 0.5 s of 20 ms chunks. */
    private const val QUEUE = 25

    private val lock = Any()
    private var player: Player? = null

    /** A chunk of [pcId]'s microphone, 16 kHz mono 16-bit. Never waits. */
    fun play(pcId: String, pcm: ByteArray) {
        val current = synchronized(lock) {
            player?.takeIf { it.pcId == pcId && it.alive } ?: Player(pcId).also {
                player?.close()
                player = it
            }
        }
        current.offer(pcm)
    }

    /** The test with [pcId] ended: the speaker goes quiet. */
    fun end(pcId: String) {
        val old = synchronized(lock) { player?.takeIf { it.pcId == pcId }?.also { player = null } }
        old?.close()
    }

    /** One test's speaker: a thread that plays what [offer] queues, so the core's event thread never blocks. */
    private class Player(val pcId: String) {
        private val chunks = ArrayBlockingQueue<ByteArray>(QUEUE)

        @Volatile var alive = true
        private val track: AudioTrack? = runCatching {
            val size = AudioTrack.getMinBufferSize(RATE, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT)
            AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                        .build(),
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setSampleRate(RATE)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                        .build(),
                )
                .setBufferSizeInBytes(maxOf(size, RATE / 50 * 2 * 8))
                .setTransferMode(AudioTrack.MODE_STREAM)
                .build()
                .also { it.play() }
        }.onFailure { Log.w(TAG, "can't play the test sound", it) }.getOrNull()

        init {
            thread(isDaemon = true, name = "call-audio-test") {
                try {
                    while (alive) {
                        val chunk = chunks.poll(200, TimeUnit.MILLISECONDS) ?: continue
                        // Whole 16-bit samples only.
                        track?.write(chunk, 0, chunk.size - chunk.size % 2)
                    }
                } catch (_: InterruptedException) {
                } finally {
                    track?.let { runCatching { it.stop() }; it.release() }
                }
            }
        }

        /** Drops the chunk when the speaker is behind, so what plays stays close to live. */
        fun offer(pcm: ByteArray) {
            if (alive) chunks.offer(pcm)
        }

        fun close() {
            alive = false
        }
    }
}
