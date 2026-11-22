// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.services.client

import android.content.ComponentName
import android.content.Context
import android.content.Intent
import android.content.ServiceConnection
import android.content.pm.ApplicationInfo
import android.content.pm.PackageManager
import android.os.Build
import android.os.IBinder
import android.os.ParcelFileDescriptor
import android.util.Log
import java.security.MessageDigest
import app.nectarlink.services.IDisplaySession
import app.nectarlink.services.INectarlinkService
import app.nectarlink.services.INectarlinkServiceCallback
import app.nectarlink.services.PointerScreenInfo
import app.nectarlink.services.ServiceStatus
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

class NectarlinkServiceClient(
    context: Context,
    private val requireSystemApp: Boolean = true,
    private val trustedServiceCertSha256: Set<String> = NectarlinkServiceContract.TRUSTED_SERVICE_CERT_SHA256,
) {
    sealed interface ConnectionState {
        data object Unavailable : ConnectionState
        data object Disconnected : ConnectionState
        data object Connecting : ConnectionState
        data class Connected(val status: ServiceStatus) : ConnectionState
        data class Rejected(val reason: String) : ConnectionState
    }

    data class ClipboardEvent(
        val text: String,
        val html: String?,
        val isSensitive: Boolean,
    )

    private val appContext = context.applicationContext
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    private val _state = MutableStateFlow<ConnectionState>(
        if (isInstalled(appContext, requireSystemApp, trustedServiceCertSha256)) {
            ConnectionState.Disconnected
        } else {
            ConnectionState.Unavailable
        },
    )
    val state: StateFlow<ConnectionState> = _state.asStateFlow()

    private val _clipboardEvents = MutableSharedFlow<ClipboardEvent>(extraBufferCapacity = 16)
    val clipboardEvents: SharedFlow<ClipboardEvent> = _clipboardEvents.asSharedFlow()

    private val _inputBlockedEvents = MutableSharedFlow<Unit>(extraBufferCapacity = 4)
    val inputBlockedEvents: SharedFlow<Unit> = _inputBlockedEvents.asSharedFlow()

    @Volatile
    private var binder: INectarlinkService? = null

    @Volatile
    private var bound = false

    @Volatile
    private var connectWaiter: CompletableDeferred<Boolean>? = null

    val isConnected: Boolean
        get() = _state.value is ConnectionState.Connected

    private val callback = object : INectarlinkServiceCallback.Stub() {
        override fun onClipboardChanged(text: String?, html: String?, isSensitive: Boolean) {
            if (!text.isNullOrBlank()) {
                _clipboardEvents.tryEmit(ClipboardEvent(text, html?.takeIf { it.isNotBlank() }, isSensitive))
            }
        }

        override fun onInputBlocked() {
            _inputBlockedEvents.tryEmit(Unit)
        }

        override fun onStatusChanged(status: ServiceStatus?) {
            if (status != null) {
                _state.value = if (status.serviceEnabled && status.callerAuthorized) {
                    ConnectionState.Connected(status)
                } else {
                    ConnectionState.Rejected(
                        if (!status.serviceEnabled) "service disabled" else "caller not authorized",
                    )
                }
            }
        }
    }

    private val connection = object : ServiceConnection {
        override fun onServiceConnected(name: ComponentName?, service: IBinder?) {
            val targetPkg = name?.packageName
            if (targetPkg != NectarlinkServiceContract.SERVICE_PACKAGE ||
                !isInstalled(appContext, requireSystemApp, trustedServiceCertSha256)
            ) {
                Log.w(TAG, "Refusing untrusted service binding for $targetPkg")
                binder = null
                if (bound) {
                    bound = false
                    runCatching { appContext.unbindService(this) }
                }
                _state.value = ConnectionState.Rejected("untrusted service certificate")
                connectWaiter?.complete(false)
                return
            }
            val api = INectarlinkService.Stub.asInterface(service)
            binder = api
            scope.launch {
                val ok = runCatching {
                    api.registerCallback(callback)
                    val status = api.status
                    if (status != null && status.serviceEnabled && status.callerAuthorized) {
                        _state.value = ConnectionState.Connected(status)
                        true
                    } else {
                        _state.value = ConnectionState.Rejected(
                            when {
                                status == null -> "null status"
                                !status.serviceEnabled -> "service disabled"
                                else -> "caller not authorized"
                            },
                        )
                        false
                    }
                }.getOrElse { e ->
                    Log.w(TAG, "Failed to handshake with Nectarlink Services", e)
                    _state.value = ConnectionState.Rejected(e.message ?: "handshake failed")
                    false
                }
                connectWaiter?.complete(ok)
            }
        }

        override fun onServiceDisconnected(name: ComponentName?) {
            binder = null
            _state.value = if (isInstalled(appContext, requireSystemApp, trustedServiceCertSha256)) {
                ConnectionState.Disconnected
            } else {
                ConnectionState.Unavailable
            }
            connectWaiter?.complete(false)
        }

        override fun onBindingDied(name: ComponentName?) {
            onServiceDisconnected(name)
        }

        override fun onNullBinding(name: ComponentName?) {
            binder = null
            _state.value = ConnectionState.Rejected("service refused binding")
            connectWaiter?.complete(false)
        }
    }

    suspend fun connect(timeoutMs: Long = 5_000L): Boolean {
        if (isConnected) return true
        if (!isInstalled(appContext, requireSystemApp, trustedServiceCertSha256)) {
            _state.value = ConnectionState.Unavailable
            return false
        }
        val intent = Intent(NectarlinkServiceContract.ACTION_BIND).apply {
            setPackage(NectarlinkServiceContract.SERVICE_PACKAGE)
        }
        val resolveFlags = if (requireSystemApp) PackageManager.MATCH_SYSTEM_ONLY else 0
        val resolved = appContext.packageManager.resolveService(intent, resolveFlags)
        if (resolved?.serviceInfo?.packageName != NectarlinkServiceContract.SERVICE_PACKAGE) {
            _state.value = ConnectionState.Unavailable
            return false
        }
        val waiter = CompletableDeferred<Boolean>()
        connectWaiter = waiter
        _state.value = ConnectionState.Connecting
        val initiated = runCatching {
            appContext.bindService(intent, connection, Context.BIND_AUTO_CREATE)
        }.getOrElse { e ->
            Log.w(TAG, "bindService failed", e)
            false
        }
        bound = initiated
        if (!initiated) {
            _state.value = ConnectionState.Disconnected
            connectWaiter = null
            return false
        }
        return (withTimeoutOrNull(timeoutMs) { waiter.await() } ?: false).also {
            if (connectWaiter === waiter) connectWaiter = null
        }
    }

    fun disconnect() {
        val current = binder
        binder = null
        if (current != null) {
            runCatching { current.unregisterCallback(callback) }
        }
        if (bound) {
            bound = false
            runCatching { appContext.unbindService(connection) }
        }
        _state.value = if (isInstalled(appContext, requireSystemApp, trustedServiceCertSha256)) {
            ConnectionState.Disconnected
        } else {
            ConnectionState.Unavailable
        }
    }

    fun status(): ServiceStatus? = call { it.status }

    fun provisionAppPermissions(targetPackage: String = appContext.packageName): Boolean {
        if (!NectarlinkServiceContract.isAllowedProvisionPackage(targetPackage, setOf(appContext.packageName))) {
            return false
        }
        return call { it.provisionAppPermissions(targetPackage) } ?: false
    }

    fun injectTouch(displayId: Int = -1, action: Int, x: Float, y: Float): Boolean {
        if (!NectarlinkServiceContract.isAllowedTouch(displayId, action, x, y)) return false
        return call { it.injectTouch(displayId, action, x, y) } ?: false
    }

    fun injectScroll(displayId: Int = -1, x: Float, y: Float, dx: Float, dy: Float): Boolean {
        if (!NectarlinkServiceContract.isAllowedScroll(displayId, x, y, dx, dy)) return false
        return call { it.injectScroll(displayId, x, y, dx, dy) } ?: false
    }

    fun injectKey(displayId: Int = -1, keyCode: Int, metaState: Int = 0): Boolean {
        if (!NectarlinkServiceContract.isAllowedKey(displayId, keyCode)) return false
        return call { it.injectKey(displayId, keyCode, metaState) } ?: false
    }

    fun injectText(displayId: Int = -1, text: String): Boolean {
        if (!NectarlinkServiceContract.isAllowedText(displayId, text)) return false
        return call { it.injectText(displayId, text) } ?: false
    }

    fun injectGamepad(
        displayId: Int = -1,
        buttons: Int,
        lx: Int,
        ly: Int,
        rx: Int,
        ry: Int,
        lt: Int,
        rt: Int,
    ): Boolean {
        if (!NectarlinkServiceContract.isAllowedGamepad(displayId, lx, ly, rx, ry, lt, rt)) return false
        return call { it.injectGamepad(displayId, buttons, lx, ly, rx, ry, lt, rt) } ?: false
    }

    fun pointerStart(edge: String, y: Float): PointerScreenInfo? {
        if (!NectarlinkServiceContract.isAllowedPointerStart(edge, y)) return null
        return call { it.pointerStart(edge, y) }
    }

    fun pointerMove(dx: Int, dy: Int, buttons: Int, wheel: Int): Boolean =
        call { it.pointerMove(dx, dy, buttons, wheel) } ?: false

    fun pointerWhere(): Pair<Float, Float>? =
        call { it.pointerWhere() }?.takeIf { it.size >= 2 && it[0] >= 0f && it[1] >= 0f }?.let { it[0] to it[1] }

    fun pointerStop() {
        call { it.pointerStop() }
    }

    fun openScreenMirror(
        width: Int,
        height: Int,
        dpi: Int,
        bitrate: Int,
        fps: Int,
        codec: String = "h264",
    ): IDisplaySession? {
        if (!NectarlinkServiceContract.isAllowedDisplayRequest(width, height, dpi, bitrate, fps, codec)) {
            return null
        }
        return call { it.openScreenMirror(width, height, dpi, bitrate, fps, codec) }
    }

    fun openAppDisplay(
        pkg: String,
        width: Int,
        height: Int,
        dpi: Int,
        bitrate: Int,
        fps: Int,
        codec: String = "h264",
    ): IDisplaySession? {
        if (!NectarlinkServiceContract.isValidPackageName(pkg) ||
            !NectarlinkServiceContract.isAllowedDisplayRequest(width, height, dpi, bitrate, fps, codec)
        ) {
            return null
        }
        return call { it.openAppDisplay(pkg, width, height, dpi, bitrate, fps, codec) }
    }

    fun openDeviceAudio(): ParcelFileDescriptor? = call { it.openDeviceAudio() }

    fun isCallAudioSupported(): Boolean = call { it.isCallAudioSupported } ?: false

    fun openCallAudio(): ParcelFileDescriptor? = call { it.openCallAudio() }

    fun setScreenOff(off: Boolean): Boolean = call { it.setScreenOff(off) } ?: false

    fun wakeScreen(): Boolean = call { it.wakeScreen() } ?: false

    fun setClipboard(text: String, html: String? = null, sensitive: Boolean = false): Boolean =
        call { it.setClipboard(text, html, sensitive) } ?: false

    fun clearClipboard(text: String): Boolean = call { it.clearClipboard(text) } ?: false

    fun expandNotifications(): Boolean = call { it.expandNotifications() } ?: false

    fun setWifiEnabled(enabled: Boolean): Boolean = call { it.setWifiEnabled(enabled) } ?: false

    fun setBluetoothEnabled(enabled: Boolean): Boolean = call { it.setBluetoothEnabled(enabled) } ?: false

    fun setHotspotEnabled(enabled: Boolean): Boolean = call { it.setHotspotEnabled(enabled) } ?: false

    fun setScreenBrightness(brightness: Int): Boolean {
        if (!NectarlinkServiceContract.isAllowedBrightness(brightness)) return false
        return call { it.setScreenBrightness(brightness) } ?: false
    }

    fun setMusicVolume(index: Int): Boolean {
        if (index < 0) return false
        return call { it.setMusicVolume(index) } ?: false
    }

    fun uninstallPackage(pkg: String): Boolean {
        if (!NectarlinkServiceContract.isAllowedUninstallPackage(pkg, appContext.packageName)) return false
        return call { it.uninstallPackage(pkg) } ?: false
    }

    fun launchActivityFromBackground(pkg: String, activityClass: String): Boolean {
        if (!NectarlinkServiceContract.isAllowedBackgroundLaunch(
                pkg,
                activityClass,
                setOf(appContext.packageName),
            )
        ) {
            return false
        }
        return call { it.launchActivityFromBackground(pkg, activityClass) } ?: false
    }

    fun openInputServerChannel(): ParcelFileDescriptor? = call { it.openInputServerChannel() }

    private inline fun <T> call(block: (INectarlinkService) -> T): T? {
        val api = binder ?: return null
        return runCatching { block(api) }.onFailure { e ->
            Log.w(TAG, "Binder call failed", e)
        }.getOrNull()
    }

    companion object {
        private const val TAG = "NectarlinkServiceClient"

        fun isInstalled(
            context: Context,
            requireSystemApp: Boolean = true,
            trustedCertSha256: Set<String> = NectarlinkServiceContract.TRUSTED_SERVICE_CERT_SHA256,
        ): Boolean {
            val info = runCatching {
                @Suppress("DEPRECATION")
                context.packageManager.getApplicationInfo(NectarlinkServiceContract.SERVICE_PACKAGE, 0)
            }.getOrNull() ?: return false
            if (requireSystemApp) {
                val flags = info.flags
                val isSystem = (flags and ApplicationInfo.FLAG_SYSTEM) != 0 ||
                    (flags and ApplicationInfo.FLAG_UPDATED_SYSTEM_APP) != 0
                if (!isSystem) return false
            }
            return hasTrustedSigningCertificate(
                context,
                NectarlinkServiceContract.SERVICE_PACKAGE,
                trustedCertSha256,
            )
        }

        fun isAvailable(
            context: Context,
            requireSystemApp: Boolean = true,
            trustedCertSha256: Set<String> = NectarlinkServiceContract.TRUSTED_SERVICE_CERT_SHA256,
        ): Boolean {
            if (!isInstalled(context, requireSystemApp, trustedCertSha256)) return false
            val intent = Intent(NectarlinkServiceContract.ACTION_BIND).setPackage(NectarlinkServiceContract.SERVICE_PACKAGE)
            val flags = if (requireSystemApp) PackageManager.MATCH_SYSTEM_ONLY else 0
            val resolved = context.packageManager.resolveService(intent, flags)
            return resolved?.serviceInfo?.packageName == NectarlinkServiceContract.SERVICE_PACKAGE
        }

        internal fun hasTrustedSigningCertificate(
            context: Context,
            packageName: String,
            trustedCertSha256: Set<String>,
        ): Boolean {
            val digests = trustedCertSha256.mapNotNull(::decodeSha256Hex)
            if (digests.isEmpty()) return false
            val pm = context.packageManager
            return runCatching {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                    digests.any { digest ->
                        pm.hasSigningCertificate(packageName, digest, PackageManager.CERT_INPUT_SHA256)
                    }
                } else {
                    @Suppress("DEPRECATION")
                    val info = pm.getPackageInfo(packageName, PackageManager.GET_SIGNATURES)
                    @Suppress("DEPRECATION")
                    val sigs = info.signatures ?: return@runCatching false
                    if (sigs.size != 1) return@runCatching false
                    val actual = MessageDigest.getInstance("SHA-256").digest(sigs[0].toByteArray())
                    matchesAnyDigest(actual, digests)
                }
            }.getOrDefault(false)
        }

        internal fun decodeSha256Hex(hex: String): ByteArray? {
            val clean = hex.trim().lowercase().replace(":", "")
            if (clean.length != 64 || !clean.all { it in '0'..'9' || it in 'a'..'f' }) return null
            return ByteArray(32) { i ->
                clean.substring(i * 2, i * 2 + 2).toInt(16).toByte()
            }
        }

        internal fun matchesAnyDigest(actual: ByteArray, allowed: List<ByteArray>): Boolean =
            allowed.any { expected -> MessageDigest.isEqual(actual, expected) }
    }
}
