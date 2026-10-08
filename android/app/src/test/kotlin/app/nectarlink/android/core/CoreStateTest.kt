// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.view.Surface
import app.nectarlink.core.Battery
import app.nectarlink.core.DeviceInfo
import app.nectarlink.core.DeviceKind
import app.nectarlink.core.DiscoveredDevice
import app.nectarlink.core.Event
import app.nectarlink.core.Link
import app.nectarlink.core.Notification
import app.nectarlink.core.PairedDevice
import app.nectarlink.core.PairingFailure
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class CoreStateTest {
    private fun pc(id: String, name: String, pairedAt: Long, canWake: Boolean = false) = PairedDevice(
        id = id,
        info = DeviceInfo(name, DeviceKind.DESKTOP, "windows", "10.0.26200", null, null),
        pairedAt = pairedAt,
        link = Link.Offline(null),
        canWake = canWake,
    )

    @Test
    fun devicesFollowEventsInPairingOrder() {
        var state = CoreState()
            .reduce(Event.DeviceAdded(pc("b", "Laptop", 20)))
            .reduce(Event.DeviceAdded(pc("a", "Desk", 10)))
        assertEquals(listOf("Desk", "Laptop"), state.devices.map { it.name })

        state = state.reduce(Event.WakeInfoChanged("a", true))
            .withWakeState("a", WakeState.Waking)
        assertTrue(state.device("a")!!.canWake)
        assertEquals(WakeState.Waking, state.device("a")!!.wakeState)

        state = state.reduce(Event.LinkChanged("a", Link.Online(false, 5u)))
            .reduce(Event.Battery("a", Battery(42u, true, "ac", 28u)))
        assertTrue(state.device("a")!!.online)
        assertEquals(WakeState.Idle, state.device("a")!!.wakeState)
        assertEquals(42.toUByte(), state.device("a")!!.battery!!.level)
        assertEquals(28.toUShort(), state.device("a")!!.battery!!.fullIn)

        // Unknown devices are ignored; removal drops the device.
        state = state.reduce(Event.LinkChanged("zz", Link.Connecting)).reduce(Event.DeviceRemoved("b"))
        assertEquals(listOf("a"), state.devices.map { it.id })
    }

    @Test
    fun pairingFlow() {
        var state = CoreState()
        // A failure the user isn't looking at doesn't open the pairing screen.
        state = state.reduce(Event.PairingFailed(PairingFailure.Rejected))
        assertEquals(PairingState.Idle, state.pairing)

        state = state.copy(pairing = PairingState.Connecting("a"))
            .reduce(Event.PairingCode("a", "123456"))
        assertEquals(PairingState.Comparing("a", "123456"), state.pairing)

        state = state.reduce(Event.Paired(pc("a", "Desk", 1)))
        assertEquals(PairingState.Paired("Desk"), state.pairing)

        state = state.copy(pairing = PairingState.Joining).reduce(Event.PairingFailed(PairingFailure.Expired))
        assertEquals(PairingState.Failed(PairingFailure.Expired), state.pairing)
    }

    @Test
    fun ringingAndDiscovery() {
        var state = CoreState().withDevices(listOf(pc("a", "Desk", 1)))
        state = state.reduce(Event.Ring("a", true))
        assertEquals("Desk", state.ringingFrom)
        state = state.reduce(Event.Ring("a", false))
        assertNull(state.ringingFrom)

        val found = DiscoveredDevice("n", "Nearby PC")
        state = state.reduce(Event.Discovered(found)).reduce(Event.Discovered(found))
        assertEquals(1, state.discovered.size)
        assertEquals("Nearby PC", state.nameOf("n"))
        state = state.reduce(Event.DiscoveryExpired("n"))
        assertTrue(state.discovered.isEmpty())
    }

    @Test
    fun notificationEventsLeaveThePhoneStateAlone() {
        val state = CoreState().reduce(Event.DeviceAdded(pc("a", "Desk", 10)))
        val note = Notification("k", "com.chat", "Chat", "Sam", "Hi", null, 0L, emptyList(), false, null, null)
        assertEquals(state, state.reduce(Event.NotificationPosted("a", note)))
        assertEquals(state, state.reduce(Event.NotificationRemoved("a", "k")))
        assertEquals(state, state.reduce(Event.NotificationsReset("a", listOf(note))))
        assertTrue("access is off until the listener connects", !state.notificationAccess)
    }

    @Test
    fun deckStateCarriesPcVolumeMuteAndOutputDevices() {
        val outputs = listOf(
            app.nectarlink.core.AudioOutputDevice("out-1", "Speakers (Realtek Audio)", true),
            app.nectarlink.core.AudioOutputDevice("out-2", "Headphones", false),
        )
        val deckState = app.nectarlink.core.DeckState(
            playing = true,
            volume = 72u,
            muted = false,
            micMuted = true,
            outputDevices = outputs,
        )
        val state = CoreState()
            .reduce(Event.DeviceAdded(pc("a", "Desk", 10)))
            .reduce(Event.DeckState("a", deckState))
        val dev = state.device("a")!!
        val ds = dev.deckState!!
        assertEquals(72.toUByte(), ds.volume)
        assertEquals(false, ds.muted)
        assertEquals(2, ds.outputDevices.size)
        assertEquals("Speakers (Realtek Audio)", ds.outputDevices.first { it.isDefault }.name)
    }

    @Test
    fun screenShapeUnrotatesCutoutAndCornersToUprightOrientation() {
        // Upright 1080x2400 screen with a top-left corner punch-hole at x=60..140, y=30..110.
        val r0 = normalizeCutoutRect(60, 30, 140, 110, 1080, 2400, Surface.ROTATION_0)!!
        // In ROTATION_90 (2400x1080), physical top-left (60..140, 30..110) sits near bottom-left:
        // x_rot = y_0 = 30..110, y_rot = 1080 - x_0 = 940..1020.
        val r90 = normalizeCutoutRect(30, 940, 110, 1020, 2400, 1080, Surface.ROTATION_90)!!
        // In ROTATION_270 (2400x1080), physical top-left sits near top-right:
        // x_rot = 2400 - y_0 = 2290..2370, y_rot = x_0 = 60..140.
        val r270 = normalizeCutoutRect(2290, 60, 2370, 140, 2400, 1080, Surface.ROTATION_270)!!

        assertEquals(r0.x, r90.x, 1e-4f)
        assertEquals(r0.y, r90.y, 1e-4f)
        assertEquals(r0.w, r90.w, 1e-4f)
        assertEquals(r0.h, r90.h, 1e-4f)
        assertEquals(r0.x, r270.x, 1e-4f)
        assertEquals(r0.y, r270.y, 1e-4f)

        val corners = normalizeCorners(120, 120, 100, 100, 1080, 2400, Surface.ROTATION_0)!!
        assertEquals(120f / 1080f, corners.tl, 1e-4f)
        assertEquals(100f / 1080f, corners.br, 1e-4f)
        assertNull(normalizeCorners(0, 0, 0, 0, 1080, 2400, Surface.ROTATION_0))

        val svg = buildNormalizedSvgPath(
            contours = listOf(listOf(540f to 36f, 576f to 72f, 540f to 108f, 504f to 72f)),
            rawW = 1080,
            rawH = 2400,
            rotation = Surface.ROTATION_0,
        )
        assertNotNull(svg)
        assertEquals("M 0.5 0.015 L 0.5333 0.03 L 0.5 0.045 L 0.4667 0.03 Z", svg)
    }
}

