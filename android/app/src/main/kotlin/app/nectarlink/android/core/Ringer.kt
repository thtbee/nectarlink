// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.content.Context
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.media.RingtoneManager
import android.os.Handler
import android.os.Looper
import android.os.VibrationEffect
import android.os.Vibrator
import android.os.VibratorManager
import android.os.Build

/**
 * "Find my phone": plays the alarm sound on the alarm stream (audible even
 * when the ringer is silent) and vibrates, until stopped or for a minute.
 */
class Ringer(context: Context) {
    private val context = context.applicationContext
    private val main = Handler(Looper.getMainLooper())
    private var player: MediaPlayer? = null
    private val timeout = Runnable { stopRingingNow() }

    fun startRinging() {
        main.post { startRingingNow() }
    }

    fun stopRinging() {
        main.post { stopRingingNow() }
    }

    private fun startRingingNow() {
        stopRingingNow()
        val uri = RingtoneManager.getDefaultUri(RingtoneManager.TYPE_ALARM)
            ?: RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE)
        player = runCatching {
            MediaPlayer().apply {
                setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_ALARM)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                        .build(),
                )
                setDataSource(context, uri)
                isLooping = true
                prepare()
                start()
            }
        }.getOrNull()
        vibrator()?.vibrate(VibrationEffect.createWaveform(longArrayOf(0, 600, 400), 0))
        main.postDelayed(timeout, MAX_RING_MS)
    }

    private fun stopRingingNow() {
        main.removeCallbacks(timeout)
        player?.runCatching { stop(); release() }
        player = null
        vibrator()?.cancel()
    }

    private fun vibrator(): Vibrator? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            context.getSystemService(VibratorManager::class.java)?.defaultVibrator
        } else {
            @Suppress("DEPRECATION")
            context.getSystemService(Vibrator::class.java)
        }

    private companion object {
        const val MAX_RING_MS = 60_000L
    }
}
