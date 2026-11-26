// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ShellTetheringTest {
    private interface Listener
    private class TetheringRequestParcel
    private class TetheringRequest

    private val listener = object : Listener {}
    private val request = TetheringRequestParcel()
    private val int = Int::class.javaPrimitiveType!!
    private val string = String::class.java

    private fun args(vararg types: Class<*>) =
        ShellTethering.arguments(arrayOf(*types), Listener::class.java, listener) { request }

    @Test
    fun startOnAndroid12AndLater() {
        // startTethering(TetheringRequestParcel, callerPkg, callingAttributionTag, IIntResultListener)
        assertArrayEquals(
            arrayOf(request, "com.android.shell", null, listener),
            args(TetheringRequestParcel::class.java, string, string, Listener::class.java),
        )
    }

    @Test
    fun startOnAndroid11() {
        // No attribution tag yet.
        assertArrayEquals(
            arrayOf(request, "com.android.shell", listener),
            args(TetheringRequestParcel::class.java, string, Listener::class.java),
        )
    }

    @Test
    fun stopIsForWifi() {
        // stopTethering(type, callerPkg, callingAttributionTag, IIntResultListener)
        assertArrayEquals(
            arrayOf(0, "com.android.shell", null, listener),
            args(int, string, string, Listener::class.java),
        )
    }

    @Test
    fun anUnknownParameterGivesUp() {
        assertNull(args(TetheringRequest::class.java, string, string, Listener::class.java))
    }
}
