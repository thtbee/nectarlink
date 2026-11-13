// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.annotation.SuppressLint
import android.companion.CompanionDeviceManager
import android.content.Context
import android.hardware.display.VirtualDisplay
import android.hardware.display.VirtualDisplayConfig
import android.os.Build
import android.os.IBinder
import android.view.Surface
import androidx.annotation.RequiresApi
import java.util.concurrent.Executor

/**
 * A VirtualDeviceManager virtual device, made by the shell-side helper only
 * to own an app window's display. Its display is in the device's own display and power group, so
 * the app starts and keeps running while the phone's own screen is off or
 * locked. A plain virtual display can't: Android 17 (`separate_timeouts`)
 * puts it in the default group whatever its flags, and it sleeps with the
 * phone (#152).
 *
 * Needs a CDM association with the app streaming profile for the shell.
 * [create] makes it for the first window and [close] removes it with the
 * last, so none is left on the phone when Elevated is off or NectarLink is
 * gone; [forget] clears one a helper that was killed left behind.
 *
 * Adapted from the VirtualDevice wrapper its author (rus152) wrote for the
 * scrcpy server in Scrcpy Launcher (github.com/rus152/Scrcpy-Launcher).
 */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi", "BlockedPrivateApi")
internal class VirtualDevice private constructor(private val device: Any) {

    /** A display on this device; [flags] must leave out OWN_DISPLAY_GROUP, which would move it back to the default group. */
    @RequiresApi(Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
    fun createVirtualDisplay(name: String, width: Int, height: Int, dpi: Int, surface: Surface, flags: Int): VirtualDisplay {
        val config = VirtualDisplayConfig.Builder(name, width, height, dpi).setFlags(flags).setSurface(surface).build()
        return device.javaClass
            .getMethod("createVirtualDisplay", VirtualDisplayConfig::class.java, Executor::class.java, VirtualDisplay.Callback::class.java)
            .invoke(device, config, null, null) as VirtualDisplay
    }

    fun close() {
        runCatching { device.javaClass.getMethod("close").invoke(device) }
            .onFailure { System.err.println("can't close the virtual device: $it") }
        synchronized(lock) {
            open--
            if (open <= 0) {
                open = 0
                forget()
            }
        }
    }

    companion object {
        private const val SHELL = "com.android.shell"
        private const val APP_STREAMING = "android.app.role.COMPANION_DEVICE_APP_STREAMING"
        /**
         * Any address will do; CDM only needs one. Locally administered, and
         * NectarLink's own ("NL"), so removing its association never takes
         * another tool's.
         */
        private const val ADDRESS = "02:00:00:4e:4c:01"
        private val lock = Any()
        /** Virtual devices in use: the association goes with the last. */
        private var open = 0
        /** VirtualDeviceParams.LOCK_STATE_ALWAYS_UNLOCKED: apps show on it while the phone is locked. */
        private const val LOCK_STATE_ALWAYS_UNLOCKED = 1
        private const val ASSOCIATION_POLLS = 20
        private const val ASSOCIATION_POLL_MS = 100L

        /** A virtual device, or null when there's no association and none can be made, or no virtual devices. */
        @RequiresApi(Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
        fun create(): VirtualDevice? = synchronized(lock) {
            make()?.also { open++ } ?: null.also { if (open == 0) forget() }
        }

        /** Removes NectarLink's association for the shell, if there is one. */
        fun forget() {
            runCatching {
                Runtime.getRuntime().exec(arrayOf("cmd", "companiondevice", "disassociate", "0", SHELL, ADDRESS)).waitFor()
            }.onFailure { System.err.println("can't remove the app streaming association: $it") }
        }

        @RequiresApi(Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
        private fun make(): VirtualDevice? = runCatching {
            val association = association() ?: return null
            val context = ShellContext.get()
            val managerType = Class.forName("android.companion.virtual.VirtualDeviceManager")
            val serviceType = Class.forName("android.companion.virtual.IVirtualDeviceManager")
            val manager = managerType.getConstructor(serviceType, Context::class.java)
                .newInstance(service("virtualdevice", serviceType), context)
            val paramsType = Class.forName("android.companion.virtual.VirtualDeviceParams")
            val builderType = Class.forName("android.companion.virtual.VirtualDeviceParams\$Builder")
            val builder = builderType.getConstructor().newInstance()
            builderType.getMethod("setName", String::class.java).invoke(builder, "Nectarlink")
            builderType.getMethod("setLockState", Int::class.javaPrimitiveType).invoke(builder, LOCK_STATE_ALWAYS_UNLOCKED)
            val params = builderType.getMethod("build").invoke(builder)
            val device = managerType.getMethod("createVirtualDevice", Int::class.javaPrimitiveType, paramsType)
                .invoke(manager, association, params)!!
            VirtualDevice(device)
        }.onFailure { System.err.println("no virtual device: $it") }.getOrNull()

        /** The shell's app streaming association, made first if there's none. */
        @RequiresApi(Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
        private fun association(): Int? = findAssociation() ?: run {
            val process = Runtime.getRuntime().exec(
                arrayOf("cmd", "companiondevice", "associate", "0", SHELL, ADDRESS, APP_STREAMING),
            )
            val said = process.inputStream.bufferedReader().readText() + process.errorStream.bufferedReader().readText()
            process.waitFor()
            // CDM lists a new association a moment after `associate` returns.
            repeat(ASSOCIATION_POLLS) {
                findAssociation()?.let { return it }
                Thread.sleep(ASSOCIATION_POLL_MS)
            }
            System.err.println("can't associate for app streaming: ${said.trim()}")
            null
        }

        @RequiresApi(Build.VERSION_CODES.UPSIDE_DOWN_CAKE)
        private fun findAssociation(): Int? {
            val serviceType = Class.forName("android.companion.ICompanionDeviceManager")
            val cdm = CompanionDeviceManager::class.java.getConstructor(serviceType, Context::class.java)
                .newInstance(service("companiondevice", serviceType), ShellContext.get())
            return cdm.myAssociations.firstOrNull {
                it.deviceProfile == APP_STREAMING && it.deviceMacAddress?.toString().equals(ADDRESS, ignoreCase = true)
            }?.id
        }

        private fun service(name: String, type: Class<*>): Any {
            val binder = Class.forName("android.os.ServiceManager")
                .getMethod("getService", String::class.java)
                .invoke(null, name) as IBinder
            return Class.forName("${type.name}\$Stub").getMethod("asInterface", IBinder::class.java).invoke(null, binder)!!
        }
    }
}
