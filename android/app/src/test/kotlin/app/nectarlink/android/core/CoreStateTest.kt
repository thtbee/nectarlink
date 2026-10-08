// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

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
            .reduce(Event.Battery("a", Battery(42u, true, "ac")))
        assertTrue(state.device("a")!!.online)
        assertEquals(WakeState.Idle, state.device("a")!!.wakeState)
        assertEquals(42.toUByte(), state.device("a")!!.battery!!.level)

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
}

