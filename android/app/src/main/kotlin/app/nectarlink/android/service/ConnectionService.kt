// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.service

import android.app.KeyguardManager
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleService
import androidx.lifecycle.ProcessLifecycleOwner
import androidx.lifecycle.lifecycleScope
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.banking.BankingPause
import app.nectarlink.android.banking.PauseDecider
import app.nectarlink.android.banking.awaitScreenWake
import app.nectarlink.android.banking.bankingPollWaitMs
import app.nectarlink.android.clipboard.SendActivity
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.ui.MainActivity
import app.nectarlink.android.ui.Preferences
import app.nectarlink.android.ui.RingActivity
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * Keeps this phone reachable by its paired PCs while the app is in the
 * background: a foreground service (type "connected device") with an
 * ongoing notification, plus, while the screen is on, the Wi-Fi multicast
 * lock that local discovery needs on Android.
 */
class ConnectionService : LifecycleService() {
    private var multicastLock: WifiManager.MulticastLock? = null
    private var screenReceiver: BroadcastReceiver? = null
    /** False when [registerScreenWake] did not stick. The poll must not idle for 15s with no wake. */
    @Volatile private var screenWakeRegistered = false
    /** One pending screen return is enough. A burst must not queue a spin. */
    private val screenWake = Channel<Unit>(Channel.CONFLATED)
    private var foregroundStarted = false

    override fun onCreate() {
        super.onCreate()
        createConnectionChannel(this)
        val core = (application as NectarlinkApplication).core
        if (!startInForeground(core.state.value)) {
            stopSelf()
            return
        }
        foregroundStarted = true
        createSecondaryChannels(this)
        multicastLock = getSystemService(WifiManager::class.java)
            ?.createMulticastLock("nectarlink-discovery")
            ?.apply { setReferenceCounted(false) }
        setDiscoveryLock(getSystemService(PowerManager::class.java)?.isInteractive != false)
        registerScreenWake()

        lifecycleScope.launch {
            core.state.map { Status.of(it) to (it.status is app.nectarlink.android.core.CoreStatus.Ready) }
                .distinctUntilChanged()
                .collect { (status, ready) ->
                    if (ready && status.paired == 0) {
                        stopSelf()
                    } else {
                        getSystemService(NotificationManager::class.java)
                            .notify(ONGOING_ID, ongoingNotification(status))
                    }
                }
        }
        // USB only: with no PC on the cable there's nothing to keep running or to show. The
        // PC starts this again when it connects (usb.rs). The wait covers the PC's first
        // connection after it woke the app, and a cable that's briefly out.
        lifecycleScope.launch {
            core.state
                .map { it.usbOnly && it.status is app.nectarlink.android.core.CoreStatus.Ready && it.devices.none { d -> d.online } }
                .distinctUntilChanged()
                .collectLatest { idle ->
                    if (!idle) return@collectLatest
                    delay(USB_IDLE_GRACE_MS)
                    stopSelf()
                    (application as NectarlinkApplication).onUsbIdle()
                }
        }
        // USB only doesn't need Wi-Fi to stay awake for multicast.
        lifecycleScope.launch {
            core.state.map { it.usbOnly }.distinctUntilChanged().collect {
                setDiscoveryLock(getSystemService(PowerManager::class.java)?.isInteractive != false)
            }
        }
        // Updates while the app is in the background (they install by themselves when
        // allowed). AppUpdater checks at most once a day, so waking now and then is cheap.
        lifecycleScope.launch {
            val updater = (application as NectarlinkApplication).updater
            while (true) {
                runCatching { updater.updateIfDue() }.onFailure { Log.i(TAG, "update check failed", it) }
                delay(UPDATE_WAKE_MS)
            }
        }
        // Pause for banking apps: follow the app in front while it's set up.
        // Screen returns end the idle wait and start a fresh resume countdown.
        lifecycleScope.launch(kotlinx.coroutines.Dispatchers.Default) {
            val context = applicationContext
            val power = getSystemService(PowerManager::class.java)
            var decider = PauseDecider()
            var foreground: BankingPause.Foreground? = null
            while (true) {
                if (BankingPause.apps(context).isEmpty()) {
                    // Nothing chosen: wait for a choice rather than look every 15 s.
                    foreground = null
                    decider = PauseDecider()
                    BankingPause.awaitAppsChange(context)
                    continue
                }
                if (!BankingPause.isReady(context)) {
                    // Chosen, but a permission is missing: Android doesn't say when it's
                    // granted, so look again now and then.
                    foreground = null
                    decider = PauseDecider()
                    delay(BANKING_IDLE_MS)
                    continue
                }
                val follow = foreground ?: BankingPause.Foreground(context).also { foreground = it }
                val pkg = follow.poll()
                if (screenWake.tryReceive().isSuccess || pkg == null) decider = PauseDecider()
                when (decider.decide(pkg, BankingPause.apps(context), BankingPause.isPaused(context), System.currentTimeMillis())) {
                    PauseDecider.Action.PAUSE -> BankingPause.pause(context)
                    PauseDecider.Action.RESUME -> BankingPause.resume(context)
                    null -> {}
                }
                val wait = bankingPollWaitMs(power?.isInteractive != false, screenWakeRegistered, BANKING_POLL_MS, BANKING_IDLE_MS)
                if (awaitScreenWake(wait, screenWake)) decider = PauseDecider()
            }
        }
        lifecycleScope.launch {
            core.state.map { it.ringingPeer to it.ringingFrom }.distinctUntilChanged().collect { (peer, from) ->
                val manager = getSystemService(NotificationManager::class.java)
                if (peer != null) {
                    manager.notify(RINGING_ID, ringingNotification(from.orEmpty()))
                    if (!getSystemService(KeyguardManager::class.java).isKeyguardLocked) showRingScreen()
                } else {
                    manager.cancel(RINGING_ID)
                }
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        super.onStartCommand(intent, flags, startId)
        if (intent?.action == ACTION_STOP_SERVICE) {
            Preferences.setBackgroundServiceEnabled(this, false)
            stopSelf(startId)
            getSystemService(NotificationManager::class.java)?.cancel(ONGOING_ID)
            (application as? NectarlinkApplication)?.onBackgroundServiceDisabled()
            return START_NOT_STICKY
        }
        if (!foregroundStarted) {
            stopSelf(startId)
            return START_NOT_STICKY
        }
        if (intent?.action == ACTION_USB_WAKE) {
            // The PC started this through adb. Settings that want no service here say so.
            if (!Preferences.isUsbOnlyEnabled(this) && !Preferences.isBackgroundServiceEnabled(this)) {
                stopSelf(startId)
                return START_NOT_STICKY
            }
            (application as NectarlinkApplication).core.start()
        }
        if (intent?.action == ACTION_STOP_RINGING) {
            (application as NectarlinkApplication).core.stopRinging()
        }
        return START_STICKY
    }

    override fun onDestroy() {
        unregisterScreenWake()
        multicastLock?.runCatching { if (isHeld) release() }
        val interactive = getSystemService(PowerManager::class.java)?.isInteractive != false
        (application as? NectarlinkApplication)?.core?.onScreenInteractiveChanged(interactive)
        getSystemService(NotificationManager::class.java)?.let {
            it.cancel(ONGOING_ID)
            it.cancel(RINGING_ID)
        }
        super.onDestroy()
    }

    /**
     * Holds the multicast lock only while the screen is on. With it held,
     * Wi-Fi wakes the phone for every multicast packet on the network (mDNS,
     * SSDP from TVs and speakers), all night. With the screen off, the phone
     * and its PCs reach each other at the addresses they last had; a pairing
     * screen holds a lock of its own.
     */
    private fun setDiscoveryLock(on: Boolean) {
        val hold = on && (application as? NectarlinkApplication)?.core?.state?.value?.usbOnly != true
        multicastLock?.runCatching { if (hold) { if (!isHeld) acquire() } else if (isHeld) release() }
        (application as? NectarlinkApplication)?.core?.onScreenInteractiveChanged(on)
    }

    /**
     * Screen on and user present are system broadcasts. Registered only while
     * this service is alive, so a restart cannot leave a receiver behind, and
     * the receiver does not start the service.
     */
    private fun registerScreenWake() {
        if (screenReceiver != null) return
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                val action = intent.action
                if (action == Intent.ACTION_SCREEN_OFF) {
                    screenWake.tryReceive()
                    return setDiscoveryLock(false)
                }
                if (action != Intent.ACTION_SCREEN_ON && action != Intent.ACTION_USER_PRESENT) return
                setDiscoveryLock(true)
                screenWake.trySend(Unit)
                if (action == Intent.ACTION_USER_PRESENT) {
                    val core = (application as NectarlinkApplication).core
                    if (core.state.value.ringingPeer != null) showRingScreen()
                    core.reconnectIfOffline()
                }
            }
        }
        val filter = IntentFilter().apply {
            addAction(Intent.ACTION_SCREEN_ON)
            addAction(Intent.ACTION_SCREEN_OFF)
            addAction(Intent.ACTION_USER_PRESENT)
        }
        val registered = runCatching {
            ContextCompat.registerReceiver(this, receiver, filter, ContextCompat.RECEIVER_NOT_EXPORTED)
        }.onFailure {
            Log.w(TAG, "screen wake receiver failed", it)
        }.isSuccess
        if (!registered) return
        screenReceiver = receiver
        screenWakeRegistered = true
    }

    private fun unregisterScreenWake() {
        // Drop the flag first so a poll still in the loop cannot idle with the receiver already gone.
        screenWakeRegistered = false
        val receiver = screenReceiver ?: return
        screenReceiver = null
        runCatching { unregisterReceiver(receiver) }
    }

    private fun startInForeground(state: CoreState): Boolean = runCatching {
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
        } else {
            0
        }
        ServiceCompat.startForeground(this, ONGOING_ID, ongoingNotification(Status.of(state)), type)
    }.onFailure {
        android.util.Log.w("ConnectionService", "can't enter foreground", it)
    }.isSuccess

    /** What the ongoing notification says. */
    private data class Status(
        val connected: List<String>,
        val paired: Int,
        val noNetwork: Boolean,
        val pause: app.nectarlink.android.core.DialingPause?,
        val usbOnly: Boolean,
    ) {
        companion object {
            fun of(state: CoreState) = Status(
                state.devices.filter { it.online }.map { it.name },
                state.devices.size,
                // Not trying to reach PCs then (#397), so it shouldn't look like it is.
                noNetwork = !state.hasNetwork && !state.awayMode && !state.usbOnly,
                pause = state.dialingPause.takeIf { !state.awayMode },
                usbOnly = state.usbOnly,
            )
        }
    }

    private fun ongoingNotification(status: Status): Notification {
        val text = when {
            status.connected.size == 1 -> getString(R.string.notification_connected_to, status.connected[0])
            status.connected.size > 1 -> resources.getQuantityString(
                R.plurals.notification_connected_pcs,
                status.connected.size,
                status.connected.size,
            )
            status.paired == 0 -> getString(R.string.notification_not_paired)
            status.usbOnly -> getString(R.string.notification_usb_only)
            status.noNetwork -> getString(R.string.notification_waiting_network)
            status.pause == app.nectarlink.android.core.DialingPause.GaveUp ->
                getString(R.string.notification_stopped_looking)
            status.pause == app.nectarlink.android.core.DialingPause.UnknownNetwork ->
                getString(R.string.notification_unknown_network)
            else -> getString(R.string.notification_waiting)
        }
        val builder = NotificationCompat.Builder(this, CHANNEL_CONNECTION)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(text)
            .setContentIntent(openApp())
            .setOngoing(true)
            .setSilent(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
        if (status.connected.isNotEmpty()) {
            val sendClip = PendingIntent.getActivity(
                this, 2, SendActivity.sendClipboardIntent(this), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val sendFiles = PendingIntent.getActivity(
                this, 3, SendActivity.sendFilesIntent(this), PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            builder.addAction(0, getString(R.string.action_send_clipboard), sendClip)
            builder.addAction(0, getString(R.string.action_send_files), sendFiles)
        } else {
            // Waiting for a PC is when turning this off makes sense. While one is
            // connected, Settings still has it.
            val stopService = PendingIntent.getService(
                this, 4, Intent(this, ConnectionService::class.java).setAction(ACTION_STOP_SERVICE),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            builder.addAction(0, getString(R.string.action_stop_service), stopService)
        }
        return builder.build()
    }

    private fun showRingScreen() {
        // Android can restrict this background launch. The alarm notification also opens the screen.
        runCatching { startActivity(RingActivity.intent(this)) }
    }

    private fun ringingNotification(from: String): Notification {
        val open = PendingIntent.getActivity(
            this, RINGING_ID, RingActivity.intent(this), PendingIntent.FLAG_IMMUTABLE,
        )
        val stop = PendingIntent.getService(
            this, 1, Intent(this, ConnectionService::class.java).setAction(ACTION_STOP_RINGING),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val title = if (from.isEmpty()) getString(R.string.ringing_title_unknown) else getString(R.string.ringing_title, from)
        return NotificationCompat.Builder(this, CHANNEL_RINGING)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(title)
            .setContentIntent(open)
            .setFullScreenIntent(open, true)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_ALARM)
            .setOngoing(true)
            .addAction(0, getString(R.string.action_stop_ringing), stop)
            .setDeleteIntent(stop)
            .build()
    }

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
    )

    internal enum class StartStrategy {
        SKIP,
        START_SERVICE,
        START_FOREGROUND_SERVICE,
    }

    companion object {
        private const val TAG = "Nectarlink"
        private const val UPDATE_WAKE_MS = 6L * 3600 * 1000
        /** How often to look at the app in front while the screen is on and Pause for banking apps is set up. */
        private const val BANKING_POLL_MS = 800L
        /** Screen off, or Pause for banking apps is not set up. */
        private const val BANKING_IDLE_MS = 15_000L
        private const val ONGOING_ID = 1
        private const val RINGING_ID = 2
        private const val CHANNEL_CONNECTION = "connection"
        private const val CHANNEL_RINGING = "ringing"
        private const val ACTION_STOP_RINGING = "app.nectarlink.action.STOP_RINGING"
        /** Sent by a PC's adb (the shell holds the DUMP permission both components ask for), see usb.rs. */
        internal const val ACTION_USB_WAKE = "app.nectarlink.action.USB_WAKE"
        private const val USB_IDLE_GRACE_MS = 5_000L
        internal const val ACTION_STOP_SERVICE = "app.nectarlink.action.STOP_SERVICE"

        internal fun resolveStartStrategy(
            backgroundServiceEnabled: Boolean,
            usbOnlyEnabled: Boolean,
            usbConnected: Boolean,
            fromForeground: Boolean,
        ): StartStrategy {
            if (!backgroundServiceEnabled) return StartStrategy.SKIP
            if (usbOnlyEnabled && !usbConnected) return StartStrategy.SKIP
            return if (fromForeground) StartStrategy.START_SERVICE else StartStrategy.START_FOREGROUND_SERVICE
        }

        private fun isProcessInForeground(): Boolean = runCatching {
            ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
        }.getOrDefault(false)

        /**
         * Starts the service.
         *
         * From the foreground ([fromForeground] true, or when the process lifecycle is
         * already [Lifecycle.State.STARTED]), uses [Context.startService] so Android does
         * not arm the `ForegroundServiceDidNotStartInTimeException` timeout; [onCreate]
         * still promotes to foreground when allowed and exits cleanly via [stopSelf] if
         * blocked by OEM or battery-saver rules. From the background, uses
         * [ContextCompat.startForegroundService] and catches background-start restrictions.
         */
        @JvmOverloads
        fun start(context: Context, fromForeground: Boolean = isProcessInForeground()) {
            val strategy = resolveStartStrategy(
                backgroundServiceEnabled = Preferences.isBackgroundServiceEnabled(context),
                usbOnlyEnabled = Preferences.isUsbOnlyEnabled(context),
                usbConnected = usbConnected(context),
                fromForeground = fromForeground,
            )
            if (strategy == StartStrategy.SKIP) return
            val intent = Intent(context, ConnectionService::class.java)
            when (strategy) {
                StartStrategy.SKIP -> return
                StartStrategy.START_SERVICE -> runCatching {
                    context.startService(intent)
                }.onFailure {
                    Log.w("ConnectionService", "can't start in foreground", it)
                }
                StartStrategy.START_FOREGROUND_SERVICE -> runCatching {
                    ContextCompat.startForegroundService(context, intent)
                }.onFailure {
                    Log.w("ConnectionService", "can't start in background", it)
                }
            }
        }

        private fun usbConnected(context: Context): Boolean =
            (context.applicationContext as? NectarlinkApplication)?.core?.state?.value
                ?.devices?.any { it.online } == true

        fun stop(context: Context) {
            runCatching {
                context.stopService(Intent(context, ConnectionService::class.java))
            }
        }

        private fun createConnectionChannel(context: Context) {
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_CONNECTION, context.getString(R.string.channel_connection), NotificationManager.IMPORTANCE_LOW,
                ).apply { setShowBadge(false) },
            )
        }

        private fun createSecondaryChannels(context: Context) {
            val manager = context.getSystemService(NotificationManager::class.java)
            manager.createNotificationChannel(
                NotificationChannel(
                    CHANNEL_RINGING, context.getString(R.string.channel_ringing), NotificationManager.IMPORTANCE_HIGH,
                ).apply { setSound(null, null) },
            )
            app.nectarlink.android.files.TransferNotifications.createChannels(context)
            app.nectarlink.android.media.PcMedia.createChannel(context)
            app.nectarlink.android.links.LinkNotifications.createChannel(context)
            app.nectarlink.android.notifications.TaskNotifications.createChannels(context)
            app.nectarlink.android.webcam.WebcamService.createChannel(context)
            app.nectarlink.android.webcam.WebcamRequests.createChannel(context)
        }
    }
}
