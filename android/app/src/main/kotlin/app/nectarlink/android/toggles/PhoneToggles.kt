// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.toggles

import android.app.NotificationManager
import android.bluetooth.BluetoothAdapter
import android.bluetooth.BluetoothManager
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.database.ContentObserver
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.media.AudioManager
import android.net.Uri
import android.net.wifi.WifiManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import android.util.Log
import androidx.core.content.ContextCompat
import app.nectarlink.android.elevated.Elevated
import app.nectarlink.core.PhoneToggleValue
import app.nectarlink.core.PhoneToggles as CorePhoneToggles
import kotlin.math.roundToInt

/**
 * Reads, watches, and changes the phone's quick settings for paired PCs
 * (`docs/protocol/toggles.md`).
 */
class PhoneToggles(
    context: Context,
    private val onChanged: (CorePhoneToggles) -> Unit,
    private val onCapabilitiesChanged: () -> Unit,
) {
    private val context = context.applicationContext
    private val mainHandler = Handler(Looper.getMainLooper())
    private val audio = this.context.getSystemService(AudioManager::class.java)
    private val notifications = this.context.getSystemService(NotificationManager::class.java)
    private val camera = this.context.getSystemService(CameraManager::class.java)
    private val wifi = this.context.getSystemService(WifiManager::class.java)
    private val bluetooth = this.context.getSystemService(BluetoothManager::class.java)

    private val torchCameraId: String? = findTorchCamera()
    @Volatile private var torchOn: Boolean = false
    private var started = false
    private var lastSent: CorePhoneToggles? = null

    private val emitRunnable = Runnable { pushIfChanged() }
    private val followUpRunnable = Runnable { pushIfChanged() }

    private val torchCallback = object : CameraManager.TorchCallback() {
        override fun onTorchModeChanged(cameraId: String, enabled: Boolean) {
            if (cameraId == torchCameraId && torchOn != enabled) {
                torchOn = enabled
                schedulePush()
            }
        }
    }

    private val settingsObserver = object : ContentObserver(mainHandler) {
        override fun onChange(selfChange: Boolean) {
            schedulePush()
        }
    }

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == NotificationManager.ACTION_NOTIFICATION_POLICY_ACCESS_GRANTED_CHANGED) {
                onCapabilitiesChanged()
            }
            schedulePush()
        }
    }

    fun start() {
        if (started) return
        started = true
        if (torchCameraId != null) {
            runCatching { camera?.registerTorchCallback(torchCallback, mainHandler) }
                .onFailure { Log.w(TAG, "torch callback failed", it) }
        }
        runCatching {
            context.contentResolver.registerContentObserver(
                Settings.System.getUriFor(Settings.System.SCREEN_BRIGHTNESS),
                false,
                settingsObserver,
            )
            context.contentResolver.registerContentObserver(
                Settings.Global.getUriFor(Settings.Global.BLUETOOTH_ON),
                false,
                settingsObserver,
            )
        }.onFailure { Log.w(TAG, "settings observer failed", it) }

        val filter = IntentFilter().apply {
            addAction(NotificationManager.ACTION_INTERRUPTION_FILTER_CHANGED)
            addAction(NotificationManager.ACTION_NOTIFICATION_POLICY_ACCESS_GRANTED_CHANGED)
            addAction(AudioManager.RINGER_MODE_CHANGED_ACTION)
            addAction(VOLUME_CHANGED_ACTION)
            addAction(WifiManager.WIFI_STATE_CHANGED_ACTION)
            addAction(BluetoothAdapter.ACTION_STATE_CHANGED)
        }
        ContextCompat.registerReceiver(context, receiver, filter, ContextCompat.RECEIVER_EXPORTED)
        pushNow()
    }

    fun stop() {
        if (!started) return
        started = false
        mainHandler.removeCallbacks(emitRunnable)
        mainHandler.removeCallbacks(followUpRunnable)
        if (torchCameraId != null) {
            runCatching { camera?.unregisterTorchCallback(torchCallback) }
        }
        runCatching { context.contentResolver.unregisterContentObserver(settingsObserver) }
        runCatching { context.unregisterReceiver(receiver) }
    }

    /** What `toggles.*` capabilities this phone currently offers. */
    fun capabilities(): List<String> = buildList {
        add("toggles.read")
        add("toggles.ringer")
        add("toggles.volume")
        if (torchCameraId != null) add("toggles.flashlight")
        if (canChangeDnd(context)) add("toggles.dnd")
        if (canChangeBrightness(context)) add("toggles.brightness")
        if (Elevated.running) {
            add("toggles.wifi")
            add("toggles.bluetooth")
        }
    }

    /** Reads the current state of all phone quick settings. */
    fun snapshot(): CorePhoneToggles = CorePhoneToggles(
        dnd = readDnd(),
        ringer = readRinger(),
        flashlight = if (torchCameraId != null) torchOn else null,
        volume = readVolume().toUByte(),
        brightness = readBrightness().toUByte(),
        wifi = readWifi(),
        bluetooth = readBluetooth(),
    )

    /** Forces an immediate snapshot push to `nectarlink-core`. */
    @Synchronized
    fun pushNow() {
        val current = snapshot()
        lastSent = current
        onChanged(current)
    }

    /** Changes one quick setting on the phone (`phone.toggle.set`). */
    fun set(id: String, value: PhoneToggleValue): Boolean {
        val ok = runCatching {
            when (id) {
                "dnd" -> {
                    val on = (value as? PhoneToggleValue.Bool)?.on ?: return false
                    setDnd(on)
                }
                "ringer" -> {
                    val mode = (value as? PhoneToggleValue.Mode)?.mode ?: return false
                    setRinger(mode)
                }
                "flashlight" -> {
                    val on = (value as? PhoneToggleValue.Bool)?.on ?: return false
                    setFlashlight(on)
                }
                "volume" -> {
                    val level = (value as? PhoneToggleValue.Level)?.level?.toInt() ?: return false
                    setVolume(level)
                }
                "brightness" -> {
                    val level = (value as? PhoneToggleValue.Level)?.level?.toInt() ?: return false
                    setBrightness(level)
                }
                "wifi" -> {
                    val on = (value as? PhoneToggleValue.Bool)?.on ?: return false
                    Elevated.runCommand(if (on) "svc wifi enable" else "svc wifi disable")
                }
                "bluetooth" -> {
                    val on = (value as? PhoneToggleValue.Bool)?.on ?: return false
                    Elevated.runCommand(if (on) "svc bluetooth enable" else "svc bluetooth disable")
                }
                else -> false
            }
        }.onFailure { Log.w(TAG, "toggle $id failed", it) }.getOrDefault(false)
        if (ok) {
            pushIfChanged()
            schedulePush()
            mainHandler.removeCallbacks(followUpRunnable)
            mainHandler.postDelayed(followUpRunnable, FOLLOW_UP_MS)
        }
        return ok
    }

    private fun schedulePush() {
        mainHandler.removeCallbacks(emitRunnable)
        mainHandler.postDelayed(emitRunnable, DEBOUNCE_MS)
    }

    @Synchronized
    private fun pushIfChanged() {
        val current = snapshot()
        if (current != lastSent) {
            lastSent = current
            onChanged(current)
        }
    }

    private fun readDnd(): Boolean {
        val filter = notifications?.currentInterruptionFilter ?: return false
        return filter != NotificationManager.INTERRUPTION_FILTER_ALL &&
            filter != NotificationManager.INTERRUPTION_FILTER_UNKNOWN
    }

    private fun setDnd(on: Boolean): Boolean {
        val nm = notifications ?: return false
        if (!nm.isNotificationPolicyAccessGranted) return false
        nm.setInterruptionFilter(
            if (on) NotificationManager.INTERRUPTION_FILTER_PRIORITY
            else NotificationManager.INTERRUPTION_FILTER_ALL,
        )
        return true
    }

    private fun readRinger(): String = when (audio?.ringerMode) {
        AudioManager.RINGER_MODE_SILENT -> "silent"
        AudioManager.RINGER_MODE_VIBRATE -> "vibrate"
        else -> "ring"
    }

    private fun setRinger(mode: String): Boolean {
        val am = audio ?: return false
        val target = when (mode) {
            "ring" -> AudioManager.RINGER_MODE_NORMAL
            "vibrate" -> AudioManager.RINGER_MODE_VIBRATE
            "silent" -> {
                if (notifications?.isNotificationPolicyAccessGranted != true) return false
                AudioManager.RINGER_MODE_SILENT
            }
            else -> return false
        }
        // Leaving silent mode when DND was turned on by silent ringer requires policy access too.
        if (am.ringerMode == AudioManager.RINGER_MODE_SILENT &&
            notifications?.isNotificationPolicyAccessGranted == false
        ) {
            return false
        }
        am.ringerMode = target
        return true
    }

    private fun setFlashlight(on: Boolean): Boolean {
        val camId = torchCameraId ?: return false
        val cm = camera ?: return false
        cm.setTorchMode(camId, on)
        torchOn = on
        return true
    }

    private fun readVolume(): Int {
        val am = audio ?: return 0
        val min = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            am.getStreamMinVolume(AudioManager.STREAM_MUSIC)
        } else {
            0
        }
        val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(min + 1)
        val cur = am.getStreamVolume(AudioManager.STREAM_MUSIC).coerceIn(min, max)
        return (((cur - min) * 100f) / (max - min)).roundToInt().coerceIn(0, 100)
    }

    private fun setVolume(level: Int): Boolean {
        val am = audio ?: return false
        val clamped = level.coerceIn(0, 100)
        val min = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            am.getStreamMinVolume(AudioManager.STREAM_MUSIC)
        } else {
            0
        }
        val max = am.getStreamMaxVolume(AudioManager.STREAM_MUSIC).coerceAtLeast(min + 1)
        val index = min + ((clamped / 100f) * (max - min)).roundToInt()
        am.setStreamVolume(AudioManager.STREAM_MUSIC, index.coerceIn(min, max), 0)
        return true
    }

    private fun readBrightness(): Int {
        val raw = runCatching {
            Settings.System.getInt(context.contentResolver, Settings.System.SCREEN_BRIGHTNESS, 128)
        }.getOrDefault(128)
        return ((raw.coerceIn(0, 255) * 100f) / 255f).roundToInt().coerceIn(0, 100)
    }

    private fun setBrightness(level: Int): Boolean {
        val raw = ((level.coerceIn(0, 100) * 255f) / 100f).roundToInt().coerceIn(0, 255)
        if (Settings.System.canWrite(context)) {
            return Settings.System.putInt(
                context.contentResolver,
                Settings.System.SCREEN_BRIGHTNESS,
                raw,
            )
        }
        if (Elevated.running) {
            return Elevated.runCommand("settings put system screen_brightness $raw")
        }
        return false
    }

    private fun readWifi(): Boolean = runCatching { wifi?.isWifiEnabled == true }.getOrDefault(false)

    private fun readBluetooth(): Boolean =
        runCatching { bluetooth?.adapter?.isEnabled }.getOrNull()
            ?: runCatching {
                Settings.Global.getInt(context.contentResolver, Settings.Global.BLUETOOTH_ON, 0) != 0
            }.getOrDefault(false)

    private fun findTorchCamera(): String? = runCatching {
        if (Build.HARDWARE == "ranchu" || Build.HARDWARE == "goldfish" || Build.MODEL.contains("sdk_gphone")) {
            return null
        }
        val cm = camera ?: return null
        var fallback: String? = null
        for (id in cm.cameraIdList) {
            val chars = cm.getCameraCharacteristics(id)
            if (chars.get(CameraCharacteristics.FLASH_INFO_AVAILABLE) == true) {
                if (chars.get(CameraCharacteristics.LENS_FACING) == CameraCharacteristics.LENS_FACING_BACK) {
                    return id
                }
                if (fallback == null) fallback = id
            }
        }
        fallback
    }.getOrNull()

    companion object {
        private const val TAG = "PhoneToggles"
        private const val DEBOUNCE_MS = 120L
        private const val FOLLOW_UP_MS = 450L
        private const val VOLUME_CHANGED_ACTION = "android.media.VOLUME_CHANGED_ACTION"

        fun hasDndAccess(context: Context): Boolean =
            context.getSystemService(NotificationManager::class.java)?.isNotificationPolicyAccessGranted == true

        fun hasWriteSettings(context: Context): Boolean = Settings.System.canWrite(context)

        fun canChangeDnd(context: Context): Boolean = hasDndAccess(context)

        fun canChangeBrightness(context: Context): Boolean =
            hasWriteSettings(context) || Elevated.running

        fun dndSettingsIntent(): Intent =
            Intent(Settings.ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)

        fun writeSettingsIntent(context: Context): Intent =
            Intent(
                Settings.ACTION_MANAGE_WRITE_SETTINGS,
                Uri.fromParts("package", context.packageName, null),
            ).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    }
}
