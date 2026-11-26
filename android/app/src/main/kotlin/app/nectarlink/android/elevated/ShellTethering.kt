// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.os.Binder
import android.os.IBinder
import android.os.Parcel

/**
 * The phone's Wi-Fi hotspot, switched by the shell helper through Android's
 * tethering service (`ITetheringConnector`), the way Settings does it (#615).
 * The shell holds `TETHER_PRIVILEGED`, so no entitlement check or UI is
 * asked for. There's no shell command for it: `cmd connectivity` has no
 * tethering, and `cmd wifi start-softap` makes a bare test access point.
 *
 * Android 11 and later; the call's arguments are matched by type, as they
 * changed between versions (a calling attribution tag from 12).
 */
internal object ShellTethering {
    private const val TETHERING_WIFI = 0
    private const val CONNECTIVITY_SCOPE_GLOBAL = 1
    private const val SHELL = "com.android.shell"
    private const val LISTENER = "android.net.IIntResultListener"

    /** Starts or stops the hotspot; false when this Android has no tethering service to ask. */
    @android.annotation.SuppressLint("PrivateApi", "DiscouragedPrivateApi")
    fun set(on: Boolean): Boolean = runCatching {
        val binder = Class.forName("android.os.ServiceManager")
            .getMethod("getService", String::class.java)
            .invoke(null, "tethering") as? IBinder ?: return false
        val connector = Class.forName("android.net.ITetheringConnector\$Stub")
            .getMethod("asInterface", IBinder::class.java)
            .invoke(null, binder) ?: return false
        val name = if (on) "startTethering" else "stopTethering"
        val call = connector.javaClass.methods.firstOrNull { it.name == name } ?: return false
        val listenerClass = Class.forName(LISTENER)
        val args = arguments(call.parameterTypes, listenerClass, listener(listenerClass, on)) { type ->
            request(type)
        } ?: return false
        call.invoke(connector, *args)
        true
    }.getOrElse {
        System.err.println("tethering service unavailable: $it")
        false
    }

    /**
     * The call's arguments by type: the request (made by [request]), the
     * Wi-Fi type, the shell as the caller with no attribution tag, and the
     * listener. Null when a parameter is of a type this doesn't know.
     */
    internal fun arguments(
        types: Array<Class<*>>,
        listenerClass: Class<*>,
        listener: Any,
        request: (Class<*>) -> Any,
    ): Array<Any?>? {
        var seenString = false
        return Array(types.size) { i ->
            val type = types[i]
            when {
                type == Int::class.javaPrimitiveType -> TETHERING_WIFI
                type == String::class.java -> (if (seenString) null else SHELL).also { seenString = true }
                type == listenerClass -> listener
                type.name.endsWith("TetheringRequestParcel") -> request(type)
                else -> return null
            }
        }
    }

    /** A request for Wi-Fi tethering, without the carrier's entitlement check. */
    private fun request(type: Class<*>): Any {
        val parcel = type.getConstructor().newInstance()
        fun set(field: String, value: Any) {
            runCatching { type.getField(field).set(parcel, value) }
        }
        set("tetheringType", TETHERING_WIFI)
        set("exemptFromEntitlementCheck", true)
        set("showProvisioningUi", false)
        set("connectivityScope", CONNECTIVITY_SCOPE_GLOBAL)
        return parcel
    }

    /** Logs how the request went: 0 is done, anything else a tethering error code. */
    private fun listener(listenerClass: Class<*>, on: Boolean): Any {
        val report = { code: Int ->
            if (code != 0) System.err.println("hotspot ${if (on) "on" else "off"} failed: tethering error $code")
        }
        val binder = object : Binder() {
            override fun onTransact(code: Int, data: Parcel, reply: Parcel?, flags: Int): Boolean {
                if (code != FIRST_CALL_TRANSACTION) return super.onTransact(code, data, reply, flags)
                runCatching { data.enforceInterface(LISTENER) }
                report(data.readInt())
                return true
            }
        }
        var self: Any? = null
        return java.lang.reflect.Proxy.newProxyInstance(listenerClass.classLoader, arrayOf(listenerClass)) { _, method, args ->
            when (method.name) {
                "asBinder" -> binder
                "onResult" -> report(args?.getOrNull(0) as? Int ?: 0).let { null }
                "hashCode" -> System.identityHashCode(binder)
                "equals" -> args?.getOrNull(0) === self
                "toString" -> "NectarlinkTetheringListener"
                else -> null
            }
        }.also { self = it }
    }
}
