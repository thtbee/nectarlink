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
import org.junit.Assert.assertFalse
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
        reconnect = true,
    )

    @Test
    fun phoneAsSpeakerCountsWhileAConnectedPcHasItOn() {
        val speaker = app.nectarlink.core.AudioOutputDevice(PHONE_SPEAKER_OUTPUT, "Phone speaker (NectarLink)", false)
        val headphones = app.nectarlink.core.AudioOutputDevice("hp", "Headphones", true)
        fun deck(vararg outputs: app.nectarlink.core.AudioOutputDevice) =
            app.nectarlink.core.DeckState(playing = false, volume = 50u, muted = false, micMuted = null, outputDevices = outputs.toList())
        val online = CoreState()
            .reduce(Event.DeviceAdded(pc("a", "Desk", 10)))
            .reduce(Event.LinkChanged("a", Link.Online(relayed = false, usb = false, rttMs = 5u)))
        fun withDeck(state: CoreState, deck: app.nectarlink.core.DeckState) =
            state.copy(devices = state.devices.map { it.copy(deckState = deck) })

        assertTrue("relay on, nothing playing", withDeck(online, deck(headphones, speaker)).pcSpeakerRelayOn)
        assertFalse("relay off", withDeck(online, deck(headphones)).pcSpeakerRelayOn)
        assertFalse("no deck state yet", online.pcSpeakerRelayOn)
        val offline = online.reduce(Event.LinkChanged("a", Link.Offline(null)))
        assertFalse("a PC that is offline doesn't keep the phone busy", withDeck(offline, deck(speaker)).pcSpeakerRelayOn)
    }

    @Test
    fun stopReconnectingIsShownPerPcAndCleared() {
        var state = CoreState().reduce(Event.DeviceAdded(pc("a", "Desk", 10)))
        assertTrue("on unless turned off", state.device("a")!!.reconnect)
        state = state.reduce(Event.ReconnectChanged("a", false))
        assertFalse(state.device("a")!!.reconnect)
        state = state.reduce(Event.ReconnectChanged("a", true))
        assertTrue(state.device("a")!!.reconnect)
    }

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

        state = state.reduce(Event.LinkChanged("a", Link.Online(relayed = false, usb = false, rttMs = 5u)))
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
    fun withDevicesOfflineClearsOnlineStatusAndWakeState() {
        var state = CoreState()
            .reduce(Event.DeviceAdded(pc("a", "Desk", 10)))
            .reduce(Event.LinkChanged("a", Link.Online(relayed = false, usb = false, rttMs = 5u)))
            .withWakeState("a", WakeState.Waking)
        assertTrue(state.device("a")!!.online)
        assertEquals(WakeState.Waking, state.device("a")!!.wakeState)

        state = state.withDevicesOffline()
        assertFalse(state.device("a")!!.online)
        assertEquals(WakeState.Idle, state.device("a")!!.wakeState)
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
        assertEquals("a", state.ringingPeer)

        // Renaming the ringing PC updates ringingFrom and removing it still clears ringingFrom.
        val renamedInfo = DeviceInfo("Office Desk", DeviceKind.DESKTOP, "windows", "10.0.26200", null, null)
        state = state.reduce(Event.PeerInfoChanged("a", renamedInfo))
        assertEquals("Office Desk", state.ringingFrom)
        val removed = state.reduce(Event.DeviceRemoved("a"))
        assertNull(removed.ringingFrom)
        assertNull(removed.ringingPeer)

        state = state.reduce(Event.Ring("a", false))
        assertNull(state.ringingFrom)
        assertNull(state.ringingPeer)

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

    @Test
    fun accentAndTimelineEventsReduceCleanly() {
        assertEquals(0xFF4F7A4Eu, normalizeArgbSeed(0x004F7A4E))
        assertEquals(0xFFFF0000u, normalizeArgbSeed(0xFFFF0000.toInt()))

        val initialInfo = DeviceInfo("Pixel", DeviceKind.PHONE, "android", "16", "Pixel 9", 0xFF4F7A4Eu)
        var state = CoreState().reduce(
            Event.DeviceAdded(PairedDevice("p1", initialInfo, 100L, Link.Online(relayed = false, usb = true, rttMs = 4u), false, true)),
        )
        assertEquals(0xFF4F7A4Eu, state.device("p1")!!.accent)

        // Updating peer info with a new accent updates it; None preserves the last known seed.
        state = state.reduce(
            Event.PeerInfoChanged("p1", initialInfo.copy(accent = 0xFF3E6A8Au)),
        )
        assertEquals(0xFF3E6A8Au, state.device("p1")!!.accent)
        state = state.reduce(
            Event.PeerInfoChanged("p1", initialInfo.copy(accent = null)),
        )
        assertEquals(0xFF3E6A8Au, state.device("p1")!!.accent)

        assertEquals(state, state.reduce(Event.TimelineChanged))
    }

    @Test
    fun onlineDevicesSortAheadOfOfflineDevices() {
        var state = CoreState()
            .reduce(Event.DeviceAdded(pc("a", "Desk", 10)))
            .reduce(Event.DeviceAdded(pc("b", "Laptop", 20)))
        assertEquals(listOf("Desk", "Laptop"), state.devices.map { it.name })

        // Bringing the newer-paired Laptop online moves it ahead of the offline Desk.
        state = state.reduce(Event.LinkChanged("b", Link.Online(relayed = false, usb = false, rttMs = 4u)))
        assertEquals(listOf("Laptop", "Desk"), state.devices.map { it.name })

        // Bringing Desk online restores pairing-order tie-breaking among online PCs.
        state = state.reduce(Event.LinkChanged("a", Link.Online(relayed = false, usb = false, rttMs = 6u)))
        assertEquals(listOf("Desk", "Laptop"), state.devices.map { it.name })
    }

    @Test
    fun remoteInputRequestedReducesWhenControlAccessIsOff() {
        val offState = CoreState(inputAccess = false, elevated = false)
            .reduce(Event.RemoteInputRequested("pc-1"))
        assertTrue(offState.remoteInputRequested)

        val accessibilityOn = CoreState(inputAccess = true, elevated = false)
            .reduce(Event.RemoteInputRequested("pc-1"))
        assertTrue(!accessibilityOn.remoteInputRequested)

        val elevatedOn = CoreState(inputAccess = false, elevated = true)
            .reduce(Event.RemoteInputRequested("pc-1"))
        assertTrue(!elevatedOn.remoteInputRequested)
    }

    @Test
    fun inputServiceGestureHelpersClassifyTapsAndFilterDragPoints() {
        val jitterTap = listOf(100f to 200f, 103f to 202f, 98f to 199f)
        assertTrue(app.nectarlink.android.mirror.isStationaryTapOrHold(jitterTap, 1080f))

        val swipePoints = listOf(
            100f to 200f,
            100.4f to 200.3f, // sub-pixel duplicate filtered out
            120f to 240f,
            120.2f to 240.1f, // sub-pixel duplicate filtered out
            180f to 360f,
        )
        assertTrue(!app.nectarlink.android.mirror.isStationaryTapOrHold(swipePoints, 1080f))
        val filtered = app.nectarlink.android.mirror.filterDragPoints(swipePoints)
        assertEquals(listOf(100f to 200f, 120f to 240f, 180f to 360f), filtered)

        assertEquals(80L, app.nectarlink.android.mirror.swipeDurationMs(20L))
        assertEquals(250L, app.nectarlink.android.mirror.swipeDurationMs(250L))
        assertEquals(600L, app.nectarlink.android.mirror.swipeDurationMs(1100L))
    }

    @Test
    fun sendQueueMapsFfiItemsAndDropsOnDeviceRemoval() {
        val rawA = app.nectarlink.core.FfiQueueItem(
            id = "q-1",
            seq = 1L,
            deviceId = "a",
            deviceName = "Desk",
            kind = app.nectarlink.core.FfiQueueKind.CLIPBOARD,
            status = app.nectarlink.core.FfiQueueStatus.Waiting,
            createdAt = 1_700_000_000L,
            expiresAt = 1_700_604_800L,
            attempts = 0u,
            title = "Copied note",
            detail = "11 chars",
            bytesTotal = 11uL,
            bytesDone = 0uL,
            filesTotal = 0u,
            filesDone = 0u,
            files = emptyList(),
        )
        val rawB = app.nectarlink.core.FfiQueueItem(
            id = "q-2",
            seq = 2L,
            deviceId = "b",
            deviceName = "Laptop",
            kind = app.nectarlink.core.FfiQueueKind.FILES,
            status = app.nectarlink.core.FfiQueueStatus.Failed("File was deleted before it could be sent"),
            createdAt = 1_700_000_100L,
            expiresAt = 1_700_604_900L,
            attempts = 2u,
            title = "photo.jpg",
            detail = "1 file",
            bytesTotal = 2048uL,
            bytesDone = 0uL,
            filesTotal = 1u,
            filesDone = 0u,
            files = emptyList(),
        )

        var state = CoreState()
            .reduce(Event.DeviceAdded(pc("a", "Desk", 10)))
            .reduce(Event.DeviceAdded(pc("b", "Laptop", 20)))
            .copy(sendQueue = listOf(rawA.toUi(), rawB.toUi()))

        assertEquals(2, state.sendQueue.size)
        assertEquals("Copied note", state.sendQueue[0].title)
        assertNull(state.sendQueue[0].activeTransferId)
        assertEquals("q-2", state.sendQueue[1].activeTransferId)
        assertEquals(1, state.sendQueue[1].filesTotal)
        assertEquals(0, state.sendQueue[1].filesDone)
        assertEquals(app.nectarlink.core.FfiQueueStatus.Waiting, state.sendQueue[0].status)
        assertEquals(
            app.nectarlink.core.FfiQueueStatus.Failed("File was deleted before it could be sent"),
            state.sendQueue[1].status,
        )

        // QueueChanged and QueueDroppedOnUnpair are refreshed by Core via node.sendQueue() and
        // leave the pure reducer state intact until the snapshot is applied.
        assertEquals(state, state.reduce(Event.QueueChanged))
        assertEquals(state, state.reduce(Event.QueueDroppedOnUnpair("a", "Desk", 1u)))

        // Removing a paired device immediately drops its queued items from CoreState.
        state = state.reduce(Event.DeviceRemoved("a"))
        assertEquals(listOf("q-2"), state.sendQueue.map { it.id })
    }

    @Test
    fun formatQueueItemTitleCollapsesMultilineClipboardText() {
        val multiline = "First line\r\nSecond line\nThird\rFourth   with   spaces"
        assertEquals(
            "First line Second line Third Fourth with spaces",
            formatQueueItemTitle(multiline, "Clipboard", maxChars = 64),
        )

        val longClipboard = "Line one\nLine two with extra words that exceed the maximum character budget"
        val formatted = formatQueueItemTitle(
            longClipboard,
            "Clipboard",
            maxChars = 24,
        )
        assertTrue(!formatted.contains('\n'))
        assertTrue(!formatted.contains('\r'))
        assertTrue(formatted.endsWith("..."))
        assertEquals(24, formatted.length)

        assertEquals(
            "Clipboard",
            formatQueueItemTitle(" \r\n\t ", "Clipboard"),
        )
    }

    @Test
    fun transferEventUpdatesMatchingFileBackedQueueItemProgress() {
        val queuedFile = app.nectarlink.core.FfiQueueItem(
            id = "tx-100",
            seq = 10L,
            deviceId = "a",
            deviceName = "Desk",
            kind = app.nectarlink.core.FfiQueueKind.FILES,
            status = app.nectarlink.core.FfiQueueStatus.Sending,
            createdAt = 1_700_000_000L,
            expiresAt = 1_700_604_800L,
            attempts = 1u,
            title = "archive.zip",
            detail = "1 file",
            bytesTotal = 10_000uL,
            bytesDone = 0uL,
            filesTotal = 1u,
            filesDone = 0u,
            files = emptyList(),
        ).toUi()

        var state = CoreState()
            .reduce(Event.DeviceAdded(pc("a", "Desk", 10)))
            .copy(sendQueue = listOf(queuedFile))

        val liveTransfer = app.nectarlink.core.Transfer(
            id = "tx-100",
            deviceId = "a",
            direction = app.nectarlink.core.TransferDirection.OUTGOING,
            names = listOf("archive.zip"),
            files = 1u,
            total = 10_000uL,
            done = 4_500uL,
            status = app.nectarlink.core.TransferStatus.Running,
            recording = false,
            markers = emptyList(),
            openOnArrival = false,
        )

        state = state.reduce(Event.Transfer(liveTransfer))
        assertEquals(4_500L, state.sendQueue.single().bytesDone)
        assertEquals(10_000L, state.sendQueue.single().totalBytes)
        assertEquals(1, state.transfers.size)
    }
}


