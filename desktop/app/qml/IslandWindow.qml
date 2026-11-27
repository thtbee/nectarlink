// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Window
import QtQuick.Shapes
import app.nectarlink

// Dynamic Island-style floating pill anchored at the top-center of the primary
// monitor (340×46px compact, expanding smoothly to 340×368px on hover or incoming call).
// Stays hidden when idle and surfaces live activity:
//   1. Incoming / active phone call (caller name + Answer / Mute / On PC / Hang up)
//   2. File transfer in progress (circular progress ring + transfer title & speed)
//   3. Active media playback on Phone or PC (album art + scrolling song title + Play/Pause & Next)
Window {
    id: islandWin

    // GNOME on Wayland lets no app keep its own window on top. The "PiP on top"
    // extension does it for windows titled like this, for those who have it.
    title: onWayland ? qsTr("Nectar Island") + " - PiP" : qsTr("Nectar Island")
    // Clicks pass through while it's waiting with nothing to show (see keptForMinutes).
    flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.WindowDoesNotAcceptFocus
        | (Qt.platform.os !== "windows" && !interactive ? Qt.WindowTransparentForInput : 0)
    color: "transparent"

    readonly property var screens: Qt.application.screens
    readonly property var targetScreen: {
        const list = Qt.application.screens
        if (!list || list.length === 0)
            return null
        const want = Preferences.nectarIslandDisplay
        return list.find(s => s.name === want) || list[0]
    }
    screen: targetScreen
    readonly property bool onPrimaryScreen: !targetScreen || !screens || screens.length === 0
        || targetScreen === screens[0]
    readonly property int availW: targetScreen && targetScreen.desktopAvailableWidth > 0
        ? targetScreen.desktopAvailableWidth : (Screen.desktopAvailableWidth > 0 ? Screen.desktopAvailableWidth : 1920)
    readonly property int availH: targetScreen && targetScreen.desktopAvailableHeight > 0
        ? targetScreen.desktopAvailableHeight : (Screen.desktopAvailableHeight > 0 ? Screen.desktopAvailableHeight : 1080)

    // The work area (the screen minus the taskbar, wherever the taskbar is). Qt only knows
    // its size, so Windows reports the position; refreshed when the work area changes.
    property var workArea: AppController.workArea()
    readonly property int workAreaKey: availW * 100000 + availH
    onWorkAreaKeyChanged: {
        workArea = AppController.workArea()
        Qt.callLater(() => islandWin.clampCustomPosition())
    }
    readonly property real dpr: targetScreen && targetScreen.devicePixelRatio > 0
        ? targetScreen.devicePixelRatio : (Screen.devicePixelRatio > 0 ? Screen.devicePixelRatio : 1)
    readonly property real areaX: onPrimaryScreen && workArea && workArea.width
        ? workArea.x / dpr : (targetScreen ? targetScreen.virtualX : Screen.virtualX)
    readonly property real areaY: onPrimaryScreen && workArea && workArea.height
        ? workArea.y / dpr
        : (targetScreen
            ? targetScreen.virtualY + Math.max(0, targetScreen.height - targetScreen.desktopAvailableHeight)
            : Screen.virtualY)
    readonly property real areaW: onPrimaryScreen && workArea && workArea.width
        ? workArea.width / dpr : availW
    readonly property real areaH: onPrimaryScreen && workArea && workArea.height
        ? workArea.height / dpr : availH

    // Where it sits along the top (#472): "left", "center" or "right".
    readonly property string position: Preferences.nectarIslandPosition
    // A dragged position overrides the preset; the reset button clears it.
    readonly property bool customPosition: Preferences.nectarIslandHasPosition
    readonly property real defaultX: position === "left" ? areaX + 6
        : (position === "right" ? areaX + areaW - width - 6 : areaX + Math.round((areaW - width) / 2))
    readonly property real defaultY: areaY + (shape === "notch" ? 0 : 8)
    readonly property string skin: Preferences.nectarIslandSkin
    readonly property string shape: Preferences.nectarIslandShape
    readonly property real islandScale: Math.max(0.8, Math.min(1.4, Preferences.nectarIslandSize / 100))
    // Cream and light glass use dark ink; cloud follows the app's light/dark.
    readonly property bool light: skin === "cream" || skin === "glassLight"
        || (skin === "cloud" && !Theme.dark)
    readonly property color ink: light ? Theme.surfaceContent : "#FFFFFF"
    function wash(alpha) { return light ? Qt.rgba(0, 0, 0, alpha * 0.7) : Qt.rgba(1, 1, 1, alpha) }

    readonly property color islandTopColor: {
        if (skin === "cream") return "#FCF6EA"
        if (skin === "glassLight") return Qt.rgba(1, 1, 1, 0.74)
        if (skin === "glassDark") return Qt.rgba(0.10, 0.12, 0.17, 0.88)
        if (skin === "cloud") {
            const p = Theme.primary
            if (!Theme.dark)
                return Qt.rgba(0.97 - p.r * 0.04, 0.98 - p.g * 0.03, 1.0, 0.90)
            return Qt.rgba(p.r * 0.32 + 0.06, p.g * 0.32 + 0.06, p.b * 0.32 + 0.08, 0.92)
        }
        return Qt.rgba(0.075, 0.08, 0.105, 0.97)
    }
    readonly property color islandBottomColor: {
        if (skin === "cream") return "#F1E4CC"
        if (skin === "glassLight") return Qt.rgba(0.82, 0.89, 1.0, 0.44)
        if (skin === "glassDark") return Qt.rgba(0.03, 0.04, 0.06, 0.82)
        if (skin === "cloud") {
            const p = Theme.primary
            if (!Theme.dark)
                return Qt.rgba(0.90 - p.r * 0.02, 0.93 - p.g * 0.02, 0.99, 0.72)
            return Qt.rgba(p.r * 0.18 + 0.03, p.g * 0.18 + 0.03, p.b * 0.18 + 0.05, 0.88)
        }
        return Qt.rgba(0.035, 0.04, 0.055, 0.98)
    }
    readonly property color islandSurfaceColor: {
        if (skin === "cream") return "#F8F0DF"
        if (skin === "glassLight") return Qt.rgba(0.92, 0.95, 1.0, 0.72)
        if (skin === "glassDark") return Qt.rgba(0.06, 0.07, 0.10, 0.86)
        if (skin === "cloud") {
            const p = Theme.primary
            if (!Theme.dark)
                return Qt.rgba(0.95 - p.r * 0.03, 0.97 - p.g * 0.02, 1.0, 0.92)
            return Qt.rgba(p.r * 0.24 + 0.04, p.g * 0.24 + 0.04, p.b * 0.24 + 0.06, 0.92)
        }
        return Qt.rgba(0.05, 0.055, 0.075, 0.96)
    }
    readonly property color islandBorderColor: {
        if (skin === "cream") return Qt.rgba(0.42, 0.33, 0.18, 0.30)
        if (skin === "glassLight") return Qt.rgba(1, 1, 1, 0.58)
        if (skin === "glassDark") return Qt.rgba(1, 1, 1, 0.22)
        if (skin === "cloud") return light ? Qt.rgba(0, 0, 0, 0.16) : Qt.rgba(1, 1, 1, 0.28)
        return wash(0.18)
    }

    // ---- Track active media across all rows of MediaList (Phone + This PC) ----
    property string mediaDeviceId: ""
    property string mediaPlayerId: ""
    property string mediaApp: ""
    property string mediaTitle: ""
    property string mediaArtist: ""
    property string mediaArt: ""
    property bool mediaPlaying: false
    property bool mediaCanPlay: true
    property bool mediaCanPause: true
    property bool mediaCanNext: true
    property bool mediaCanPrev: true
    property bool mediaCanSeek: false
    property bool phoneSourcePlaying: false
    property bool pcSourcePlaying: false
    // Synced lyrics for the shown player, from a public lyric lookup.
    property string lyricLine: ""
    property string lyricNext: ""
    property string lyricKey: ""
    property string lyricTitle: ""
    property string lyricArtist: ""
    property real lyricDuration: 0
    property var lyricLines: []
    property var visibleLyrics: []
    property int lyricIndex: -1
    // The first line in the five-line window the card shows. It changes only
    // when the song moves to another line, not on every playback tick.
    property int lyricWindowStart: -1
    property bool lyricLoading: false
    // True when the lookup finished and the current media has no usable
    // lyrics, so the UI can say so instead of showing a guess.
    property bool lyricUnavailable: false
    property var lyricCache: ({})
    // How much of the current line has been sung, 0..1. It is animated across
    // the line's own time instead of stepping with the one-second clock, so the
    // colour sweeps through the words smoothly.
    property real lyricProgress: 0
    property real lyricLineStart: 0
    property real lyricLineEnd: 0
    // Word-level timings from NetEase's YRC, when the song has them. They let
    // the colour follow each word instead of guessing evenly across the line.
    property var lyricWords: []
    readonly property color lyricBaseColor: light
        ? Qt.rgba(0, 0, 0, 0.46) : Qt.rgba(1, 1, 1, 0.52)
    readonly property color lyricFillColor: Theme.primary
    NumberAnimation {
        id: lyricProgressAnim
        target: islandWin
        property: "lyricProgress"
        from: 0
        to: 1
        duration: 1000
        easing.type: Easing.Linear
    }
    Timer {
        id: lyricWordTick
        // 30 fps is enough for a colour sweep over lyrics and halves the
        // property-update and repaint load while a song plays.
        interval: 33
        repeat: true
        running: islandWin.visible && islandWin.mediaPlaying && islandWin.mode === "media"
            && islandWin.lyricWords.length > 0
        onTriggered: {
            const value = islandWin.yrcProgressAt(islandWin.livePosition())
            if (value !== null)
                islandWin.lyricProgress = value
        }
    }
    property real mediaDuration: 0
    property real mediaPosition: 0
    // When mediaPosition was current, as JavaScript time.
    property real mediaPositionAt: 0
    // Phones only report the position when it jumps, so it's counted forward here.
    property real mediaNow: Date.now()
    readonly property real mediaPositionNow: mediaPosition
        + (mediaPlaying ? Math.max(0, mediaNow - mediaPositionAt) : 0)

    Timer {
        interval: islandWin.mediaPlaying && islandWin.lyricLines.length > 0
            ? (islandWin.lyricWords.length > 0 ? 80 : 250) : 1000
        repeat: true
        running: islandWin.visible
            && (islandWin.mediaPlaying && islandWin.mediaDuration > 0 && islandWin.mode === "media"
                || (islandWin.mode === "media" && islandWin.lyricLines.length > 0))
        onRunningChanged: if (running) islandWin.mediaNow = Date.now()
        onTriggered: islandWin.mediaNow = Date.now()
    }
    onMediaNowChanged: if (mode === "media" && lyricLines.length > 0) updateLyric()
    onMediaPositionChanged: if (mode === "media" && lyricLines.length > 0) updateLyric()
    onMediaPlayingChanged: if (mode === "media" && lyricLines.length > 0) syncLyricProgress(true)

    // ---- Track active file transfer ----
    property string activeTransferId: ""
    property string activeTransferTitle: ""
    // The last one's title, kept for "Transfer complete": its row can go
    // before the count of active transfers drops.
    property string finishedTransferTitle: ""
    property real activeTransferProgress: 0
    property real activeTransferRate: 0
    property bool activeTransferIncoming: false
    property bool transferJustFinished: false
    property bool transferFailed: false
    readonly property color transferEndColor: transferFailed ? "#F87171" : "#4ADE80"

    // ---- Transient notification toast (only when a new phone notification arrives) ----
    property bool notificationToastActive: false
    // Content dips and comes back when the Island changes what it shows, so a
    // new song or alert doesn't snap into the old layout.
    property real modeFade: 1
    onModeChanged: {
        if (!Theme.reduceMotion) {
            modeFade = 0
            modeFadeAnim.restart()
            // The basic render loop only advances animations while the window
            // renders; keep it drawing through the fade.
            transitionSettle.restart()
        }
    }
    NumberAnimation {
        id: modeFadeAnim
        target: islandWin
        property: "modeFade"
        from: 0
        to: 1
        duration: 170
        easing.type: Easing.OutCubic
    }
    // Taken once, not bound: a binding could catch up before onCountChanged
    // runs, and the first notification after starting wouldn't show.
    property int lastNotificationCount: 0
    Timer {
        id: notificationToastTimer
        interval: 4200
        repeat: false
        onTriggered: islandWin.notificationToastActive = false
    }
    Connections {
        target: NotificationList
        function onCountChanged() {
            const cur = NotificationList.count
            if (cur > islandWin.lastNotificationCount && Preferences.nectarIslandNotifications
                && !AppController.notificationsPaused) {
                islandWin.notificationToastActive = true
                notificationToastTimer.restart()
            } else if (cur === 0) {
                islandWin.notificationToastActive = false
            }
            islandWin.lastNotificationCount = cur
        }
    }

    // Mouse click-through: hovering still opens the Island, but clicks reach
    // the window underneath. Windows only, where the cursor can be polled.
    readonly property bool clickThroughActive: Preferences.nectarIslandClickThrough
        && Qt.platform.os === "windows"
    function syncClickThrough() {
        if (Qt.platform.os === "windows")
            AppController.setIslandClickThrough(!interactive || clickThroughActive)
    }
    onInteractiveChanged: syncClickThrough()
    onClickThroughActiveChanged: {
        // The native transparent style stops Qt's own hover events; the cursor
        // watchdog below takes over until it is turned off again.
        windowHovered = false
        clickThroughHovered = false
        syncClickThrough()
    }
    readonly property bool unlockButtonVisible: unlockWindow.visible
    function turnOffClickThrough() {
        hideTip(null)
        Preferences.nectarIslandClickThrough = false
    }

    // QSG_RENDER_LOOP=basic advances QML's animation clock only while a window
    // renders; with the main window closed (App.qml) the Island is alone, so
    // its clock freezes. A wall-clock timer repaints each frame to re-advance
    // it, but only while something is really moving: the looping equalizer or
    // scrolling title, or a transition. A still pill is left alone.
    readonly property bool animating:
        (mode === "media" && mediaPlaying && !expanded && !Theme.reduceMotion)
        || compactTitleClip.shouldMarquee
        || transitionSettle.running

    // One duration for every expansion and collapse. The window itself stays
    // at its large size until the pill has finished shrinking, so the native
    // resize can never interrupt the animation halfway through.
    readonly property int morphDuration: 240

    // Holds `animating` true through the drop-in and expand/collapse Behaviors,
    // which would otherwise snap while the clock is frozen.
    Timer {
        id: transitionSettle
        interval: islandWin.morphDuration + 200
        repeat: false
    }
    onExpandedChanged: {
        if (!expanded)
            hideTip(null)
        if (!Theme.reduceMotion)
            transitionSettle.restart()
        if (expanded) {
            roomyHold.stop()
            windowRoomy = true
        } else if (Theme.reduceMotion) {
            windowRoomy = false
        } else {
            // Shrink the pill first, then release the extra window height.
            // Resizing in the middle of the tween was the visible hitch at
            // the end of a collapse.
            roomyHold.restart()
        }
    }

    Timer {
        running: islandWin.visible && islandWin.animating
        interval: 16
        repeat: true
        onTriggered: islandWin.update()
    }

    Timer {
        id: transferDoneTimer
        interval: 3200
        repeat: false
        onTriggered: {
            islandWin.transferJustFinished = false
            islandWin.transferFailed = false
            islandWin.finishedTransferTitle = ""
        }
    }

    // Rows change before `active` does, so by the time this runs every transfer
    // that just ended has been through noteTransferEnded(). Nothing is shown when
    // they were all cancelled.
    property int lastActiveTransfers: TransferList.active
    onLastActiveTransfersChanged: {
        if (lastActiveTransfers > 0) {
            // A new transfer replaces a banner still showing, and starts afresh.
            if (transferJustFinished) {
                transferDoneTimer.stop()
                transferJustFinished = false
                transferFailed = false
                finishedTransferTitle = ""
            }
            return
        }
        if (finishedTransferTitle.length > 0) {
            transferJustFinished = true
            transferDoneTimer.restart()
        }
    }

    function noteTransferEnded(status, title) {
        if (status === "failed") {
            transferFailed = true
            finishedTransferTitle = title
        } else if (status === "done" && !transferFailed) {
            finishedTransferTitle = title
        }
    }

    // The source picked with the switch button (#472), as "device|player".
    property string mediaPick: ""
    // How many sources are playing, to offer the switch.
    property int mediaSources: 0

    // Playing players: phones', and this PC's own. A PC player still shows
    // without a phone, or while only the PC plays, even if "This PC's players
    // too" is off: the Island is useful on its own.
    function playingMedia() {
        const all = []
        for (let i = 0; i < mediaRepeater.count; i++) {
            const item = mediaRepeater.itemAt(i)
            if (item && item.playing) all.push(item)
        }
        const phonePlaying = all.some(item => item.deviceId !== "local")
        return all.filter(item => item.deviceId !== "local"
            || Preferences.nectarIslandPcMedia || !phonePlaying)
    }

    function mediaKey(item) { return item.deviceId + "|" + item.playerId }

    // A song paused (here, on headphones or on the phone) stays on the Island
    // with Play for a while, rather than taking the button away (#516).
    Timer {
        id: pausedKeep
        interval: 5 * 60 * 1000
        onTriggered: islandWin.recomputeMedia()
    }
    function pausedMedia(key) {
        for (let i = 0; i < mediaRepeater.count; i++) {
            const item = mediaRepeater.itemAt(i)
            if (item && !item.playing && mediaKey(item) === key && item.title.length > 0) return item
        }
        return null
    }

    // Shows the next playing source.
    function nextMediaSource() {
        const playing = playingMedia()
        if (playing.length < 2) return
        const at = playing.findIndex(item => mediaKey(item) === mediaDeviceId + "|" + mediaPlayerId)
        mediaPick = mediaKey(playing[(at + 1) % playing.length])
        recomputeMedia()
    }

    function recomputeMedia() {
        const playing = playingMedia()
        mediaSources = playing.length
        phoneSourcePlaying = playing.some(item => item.deviceId !== "local")
        pcSourcePlaying = playing.some(item => item.deviceId === "local")
        // A pick only stands while that source is still playing.
        if (mediaPick.length > 0 && !playing.some(item => mediaKey(item) === mediaPick))
            mediaPick = ""
        // The one picked while it plays, else the side the user prefers, else
        // the first playing source.
        const preferred = Preferences.nectarIslandMediaSource === "pc"
            ? playing.find(item => item.deviceId === "local")
            : playing.find(item => item.deviceId !== "local")
        let best = playing.find(item => mediaKey(item) === mediaPick)
            || preferred
            || playing[0]
            || null
        if (best) {
            pausedKeep.stop()
        } else if (mediaTitle.length > 0) {
            // Nothing plays: the song that was showing, paused, if it's still there.
            const paused = pausedMedia(mediaDeviceId + "|" + mediaPlayerId)
            if (paused && (mediaPlaying || pausedKeep.running)) {
                if (mediaPlaying)
                    pausedKeep.restart()
                best = paused
            } else {
                pausedKeep.stop()
            }
        }
        if (best) {
            mediaDeviceId = best.deviceId
            mediaPlayerId = best.playerId
            mediaApp = best.app
            mediaTitle = best.title
            mediaArtist = best.artist
            mediaArt = best.art
            mediaPlaying = best.playing
            mediaCanPlay = best.canPlay
            mediaCanPause = best.canPause
            mediaCanNext = best.canNext
            mediaCanPrev = best.canPrevious
            mediaCanSeek = best.canSeek
            mediaDuration = best.duration
            mediaPosition = best.position
            mediaPositionAt = best.positionAt
            mediaNow = Date.now()
            if (best.title !== lyricTitle || best.artist !== lyricArtist
                    || Math.abs(best.duration - lyricDuration) > 1000) {
                lyricTitle = best.title
                lyricArtist = best.artist
                lyricDuration = best.duration
                requestLyrics(best.title, best.artist, best.duration)
            }
        } else {
            mediaDeviceId = ""
            mediaPlayerId = ""
            mediaApp = ""
            mediaTitle = ""
            mediaArtist = ""
            mediaArt = ""
            mediaPlaying = false
            mediaCanSeek = false
            clearLyrics()
        }
    }

    /// Follow one side for the current song, from the card's source switch.
    function pickMediaSource(source) {
        const playing = playingMedia()
        const hit = source === "pc"
            ? playing.find(item => item.deviceId === "local")
            : playing.find(item => item.deviceId !== "local")
        if (!hit)
            return
        mediaPick = mediaKey(hit)
        recomputeMedia()
    }

    // ---- Lyrics ----
    // A public, keyless lookup (NetEase Cloud Music). The search returns the
    // song, then its synced LRC. Everything is cached per title/artist, so a
    // song is looked up once; failures just leave the lyric area hidden.
    function finishLyrics(key, lines) {
        lyricCache[key] = lines
        if (lyricKey !== key)
            return
        lyricLines = lines
        lyricWindowStart = -1
        lyricLoading = false
        lyricUnavailable = !lines || lines.length === 0
        updateLyric()
    }

    function lyricKeyFor(title, artist, duration) {
        const d = Number(duration) > 0 ? Math.round(Number(duration) / 1000) : 0
        return String(title || "").trim() + "\u0001" + String(artist || "").trim() + "\u0001" + d
    }

    function lyricTextKey(text) {
        return String(text || "").toLowerCase()
            .replace(/[\s·・\-—_()（）\[\]【】]/g, "")
    }

    function songArtists(song) {
        return (song && song.artists ? song.artists : [])
            .map(a => lyricTextKey(a && a.name ? a.name : ""))
            .filter(a => a.length > 0)
    }

    // Only a confident match may supply lyrics. A search result that merely
    // shares a word with a video's title is not the song being played, and
    // showing its lyrics would be worse than showing none.
    function pickLyricSong(songs, title, artist, duration) {
        const wantTitle = lyricTextKey(title)
        const wantArtist = lyricTextKey(artist)
        if (!songs || songs.length === 0 || wantTitle.length < 2)
            return null
        let best = null
        let bestScore = -Infinity
        for (const song of songs) {
            const gotTitle = lyricTextKey(song.name)
            const exact = gotTitle === wantTitle
            const contains = wantTitle.length >= 2
                && (gotTitle.includes(wantTitle) || wantTitle.includes(gotTitle))
            const sameArtist = wantArtist.length > 0 && songArtists(song).some(name =>
                name === wantArtist || name.includes(wantArtist) || wantArtist.includes(name))
            const gotDuration = Number(song.duration || 0)
            const closeDuration = Number(duration) > 0 && gotDuration > 0
                && Math.abs(gotDuration - Number(duration)) <= 5000
            // Exact title, or the same artist with the title in the result.
            if (!exact && !(sameArtist && contains))
                continue
            // An exact title with a known, different length is another version;
            // do not use its lyrics for this one.
            if (exact && Number(duration) > 0 && gotDuration > 0 && !closeDuration)
                continue
            const drift = Number(duration) > 0 && gotDuration > 0
                ? Math.min(120000, Math.abs(gotDuration - Number(duration))) : 60000
            const score = (exact ? 100000 : 10000) + (sameArtist ? 5000 : 0) - drift / 100
            if (score > bestScore) {
                bestScore = score
                best = song
            }
        }
        return best
    }

    function requestLyrics(title, artist, duration) {
        const cleanTitle = String(title || "").trim()
        const cleanArtist = String(artist || "").trim()
        if (cleanTitle.length === 0) {
            clearLyrics()
            return
        }
        const key = lyricKeyFor(cleanTitle, cleanArtist, duration)
        if (key === lyricKey && (lyricLines.length > 0 || lyricLoading || lyricCache[key] !== undefined)) {
            if (lyricCache[key] !== undefined)
                lyricUnavailable = lyricCache[key].length === 0
            return
        }
        lyricKey = key
        lyricLine = ""
        lyricNext = ""
        lyricLines = []
        visibleLyrics = []
        lyricIndex = -1
        lyricLoading = true
        lyricUnavailable = false
        if (lyricCache[key] !== undefined) {
            lyricLines = lyricCache[key] || []
            lyricLoading = false
            lyricUnavailable = lyricLines.length === 0
            updateLyric()
            return
        }
        const search = new XMLHttpRequest()
        search.timeout = 8000
        search.onreadystatechange = function() {
            if (search.readyState !== XMLHttpRequest.DONE)
                return
            if (lyricKey !== key)
                return
            if (search.status !== 200) {
                finishLyrics(key, [])
                return
            }
            try {
                const found = JSON.parse(search.responseText)
                const songs = found && found.result && found.result.songs
                if (!songs || songs.length === 0) {
                    finishLyrics(key, [])
                    return
                }
                const pick = pickLyricSong(songs, cleanTitle, cleanArtist, Number(duration) || 0)
                if (!pick) {
                    finishLyrics(key, [])
                    return
                }
                fetchLyricText(key, pick.id)
            } catch (e) {
                finishLyrics(key, [])
            }
        }
        search.onerror = function() { finishLyrics(key, []) }
        search.ontimeout = function() { finishLyrics(key, []) }
        search.open("GET", "https://music.163.com/api/search/get/web?s="
            + encodeURIComponent(cleanTitle + " " + cleanArtist) + "&type=1&limit=5")
        search.send()
    }

    function fetchLyricText(key, id) {
        const lyric = new XMLHttpRequest()
        lyric.timeout = 8000
        lyric.onreadystatechange = function() {
            if (lyric.readyState !== XMLHttpRequest.DONE)
                return
            if (lyricKey !== key)
                return
            if (lyric.status !== 200) {
                finishLyrics(key, [])
                return
            }
            try {
                const data = JSON.parse(lyric.responseText)
                const yrc = data && data.yrc && data.yrc.lyric
                let lines = []
                if (yrc) {
                    try {
                        lines = parseYrc(yrc)
                    } catch (e) {
                        // A malformed word-timed line must not hide the LRC.
                        lines = []
                    }
                }
                if (lines.length === 0) {
                    const text = data && data.lrc && data.lrc.lyric
                    lines = text ? parseLrc(text) : []
                }
                finishLyrics(key, lines)
            } catch (e) {
                finishLyrics(key, [])
            }
        }
        lyric.onerror = function() { finishLyrics(key, []) }
        lyric.ontimeout = function() { finishLyrics(key, []) }
        lyric.open("GET", "https://music.163.com/api/song/lyric?id=" + id
            + "&lv=-1&kv=-1&tv=-1&yv=-1")
        lyric.send()
    }

    function isCreditLine(line) {
        return /^(作词|作曲|编曲|制作人|监制|出品|发行|策划|统筹|录音|混音|母带|吉他|贝斯|鼓|键盘|和声|弦乐|人声|lyricist|composer|producer|arranger|mixing|mastering|recording|vocals?|guitar|bass|drum|piano|synthesizer)\s*[:：]/i.test(line)
    }

    function parseLrc(text) {
        const lines = []
        const rows = String(text).split(/\r?\n/)
        for (const row of rows) {
            const match = row.match(/^\[(\d+):(\d+(?:\.\d+)?)\](.*)$/)
            if (!match)
                continue
            const at = (parseInt(match[1]) * 60 + parseFloat(match[2])) * 1000
            const line = match[3].trim()
            if (line.length === 0)
                continue
            // NetEase puts the writing/production credits at the top of many
            // LRC files with timestamps of their own. They are not lyrics and
            // made the Island show "作曲 : …" while the intro was playing.
            if (isCreditLine(line))
                continue
            lines.push({ at: at, text: line })
        }
        lines.sort((a, b) => a.at - b.at)
        return lines
    }

    // NetEase's word-timed lyrics:
    //   [lineStart,lineDuration](wordStart,wordDuration,0)word ...
    // Each word keeps its character range in the line, so colouring follows
    // the singing instead of assuming every line is read at one speed.
    function parseYrc(text) {
        const lines = []
        for (const row of String(text).split(/\r?\n/)) {
            const head = row.match(/^\[(\d+),(\d+)\](.*)$/)
            if (!head)
                continue
            const lineStart = parseInt(head[1])
            const lineDuration = parseInt(head[2])
            const body = head[3]
            const rawWords = []
            const re = /\((\d+),(\d+),\d+\)([^()]*)/g
            let match
            while ((match = re.exec(body)) !== null) {
                const at = parseInt(match[1])
                const end = at + parseInt(match[2])
                const word = match[3]
                if (word.length > 0)
                    rawWords.push({ at: at, end: end, text: word })
            }
            if (rawWords.length === 0)
                continue
            let full = ""
            const ranges = []
            for (const word of rawWords) {
                const start = full.length
                full += word.text
                ranges.push({ at: word.at, end: word.end, startChar: start, endChar: full.length })
            }
            const line = full.trim()
            if (line.length === 0 || isCreditLine(line))
                continue
            const lead = full.length - full.replace(/^\s+/, "").length
            const words = []
            for (const range of ranges) {
                const startChar = Math.max(0, range.startChar - lead)
                const endChar = Math.min(line.length, range.endChar - lead)
                if (endChar > startChar)
                    words.push({
                        at: range.at,
                        end: range.end,
                        startChar: startChar,
                        endChar: endChar,
                    })
            }
            lines.push({
                at: lineStart,
                end: lineStart + lineDuration,
                text: line,
                words: words,
            })
        }
        lines.sort((a, b) => a.at - b.at)
        return lines
    }

    function livePosition() {
        return mediaPosition + (mediaPlaying ? Math.max(0, Date.now() - mediaPositionAt) : 0)
    }

    // Where the colour sweep should be for word-timed lyrics at `pos`, or null
    // when this line has only line-level LRC timing.
    function yrcProgressAt(pos) {
        if (!lyricWords || lyricWords.length === 0 || lyricLine.length === 0)
            return null
        let revealed = 0
        for (let i = 0; i < lyricWords.length; i++) {
            const word = lyricWords[i]
            if (pos >= word.end) {
                revealed = word.endChar
                continue
            }
            if (pos > word.at) {
                const fraction = (pos - word.at) / Math.max(1, word.end - word.at)
                revealed = word.startChar + fraction * (word.endChar - word.startChar)
            }
            break
        }
        return Math.max(0, Math.min(1, revealed / lyricLine.length))
    }

    // The line ends when the next one starts, unless there is an instrumental
    // gap: then the words finish at a natural reading speed and stay filled.
    function lyricSpan(index) {
        const start = lyricLines[index].at
        const fixedEnd = Number(lyricLines[index].end || 0)
        if (fixedEnd > start)
            return { start: start, end: fixedEnd }
        let end = index + 1 < lyricLines.length ? lyricLines[index + 1].at : start + 5000
        const natural = Math.max(1800, Math.min(7000, lyricLines[index].text.length * 190))
        if (end - start > natural)
            end = start + natural
        if (end - start < 400)
            end = start + 400
        return { start: start, end: end }
    }

    // Matches the colour sweep to the player's clock. The sweep itself runs as
    // one render-thread animation, so it stays smooth between position updates;
    // it only restarts here when a seek or a new line makes it drift.
    function syncLyricProgress(force) {
        if (lyricIndex < 0 || lyricLineEnd <= lyricLineStart) {
            lyricProgressAnim.stop()
            lyricProgress = 0
            return
        }
        const pos = livePosition()
        const wordProgress = yrcProgressAt(pos)
        if (wordProgress !== null) {
            lyricProgressAnim.stop()
            lyricProgress = wordProgress
            return
        }
        const target = Math.max(0, Math.min(1,
            (pos - lyricLineStart) / (lyricLineEnd - lyricLineStart)))
        if (!mediaPlaying) {
            lyricProgressAnim.stop()
            lyricProgress = target
            return
        }
        if (!force && lyricProgressAnim.running && Math.abs(lyricProgress - target) < 0.12)
            return
        lyricProgressAnim.stop()
        lyricProgress = target
        if (target >= 1)
            return
        lyricProgressAnim.from = target
        lyricProgressAnim.to = 1
        lyricProgressAnim.duration = Math.max(120, lyricLineEnd - pos)
        lyricProgressAnim.start()
    }

    function updateLyric() {
        if (!lyricLines || lyricLines.length === 0) {
            lyricLine = ""
            lyricNext = ""
            lyricIndex = -1
            lyricWindowStart = -1
            visibleLyrics = []
            lyricWords = []
            lyricProgressAnim.stop()
            lyricProgress = 0
            return
        }
        const pos = mediaPositionNow
        // Move from the old line instead of scanning the whole song on every
        // tick. A seek still walks to the right line; ordinary playback only
        // steps over the line boundaries it crosses.
        let index = Math.min(lyricIndex, lyricLines.length - 1)
        while (index >= 0 && lyricLines[index].at > pos + 50)
            index--
        while (index + 1 < lyricLines.length && lyricLines[index + 1].at <= pos + 50)
            index++
        const previous = lyricIndex
        lyricIndex = index
        if (index < 0) {
            lyricLine = ""
            lyricNext = lyricLines[0].text
            lyricWords = []
            lyricWindowStart = -1
            if (visibleLyrics.length > 0)
                visibleLyrics = []
            lyricProgressAnim.stop()
            lyricProgress = 0
        } else {
            lyricLine = lyricLines[index].text
            lyricNext = index + 1 < lyricLines.length ? lyricLines[index + 1].text : ""
            lyricWords = lyricLines[index].words || []
            const span = lyricSpan(index)
            const lineChanged = index !== previous
                || span.start !== lyricLineStart || span.end !== lyricLineEnd
            lyricLineStart = span.start
            lyricLineEnd = span.end
            if (lineChanged) {
                lyricProgressAnim.stop()
                lyricProgress = 0
                flashLyrics()
            }
            syncLyricProgress(lineChanged)
        }
        const center = Math.max(0, index)
        const start = Math.max(0, Math.min(center - 2, lyricLines.length - 5))
        // Rebuilding this list on every position tick made the card's lyric
        // repeater re-lay out 10+ times a second. It only changes by line.
        if (start !== lyricWindowStart || index !== previous) {
            lyricWindowStart = start
            const end = Math.min(lyricLines.length, start + 5)
            const windowLines = []
            for (let i = start; i < end; i++)
                windowLines.push({ text: lyricLines[i].text, active: i === index })
            visibleLyrics = windowLines
        }
    }

    function flashLyrics() {
        if (Theme.reduceMotion)
            return
        compactLyricFlash.restart()
    }

    function clearLyrics() {
        lyricKey = ""
        lyricTitle = ""
        lyricArtist = ""
        lyricDuration = 0
        lyricLine = ""
        lyricNext = ""
        lyricLines = []
        visibleLyrics = []
        lyricIndex = -1
        lyricWindowStart = -1
        lyricLoading = false
        lyricUnavailable = false
        lyricWords = []
        lyricProgressAnim.stop()
        lyricProgress = 0
        lyricLineStart = 0
        lyricLineEnd = 0
    }

    function recomputeTransfer() {
        let found = null
        const count = transferRepeater.count
        for (let i = 0; i < count; i++) {
            const item = transferRepeater.itemAt(i)
            if (!item) continue
            if (item.status === "running" || item.status === "waiting" || item.status === "requested" || item.status === "paused") {
                found = item
                break
            }
        }
        if (found) {
            activeTransferId = found.transferId
            activeTransferTitle = found.title
            activeTransferProgress = Math.max(0, Math.min(1, found.progress || 0))
            activeTransferRate = found.rate || 0
            activeTransferIncoming = found.incoming
        } else if (!transferJustFinished) {
            activeTransferId = ""
            activeTransferTitle = ""
            activeTransferProgress = 0
            activeTransferRate = 0
        }
    }

    function formatRate(bytesPerSec) {
        const b = Number(bytesPerSec) || 0
        if (b <= 0) return ""
        if (b < 1024 * 1024)
            return Math.round(b / 1024) + " KB/s"
        return Qt.locale().toString(b / (1024 * 1024), "f", 1) + " MB/s"
    }

    readonly property bool hasMediaCandidate: mediaTitle.length > 0 && (mediaPlaying || pausedKeep.running)
    readonly property string mode: {
        if (Preferences.nectarIslandCalls && PhoneCall.active) return "call"
        if (Preferences.nectarIslandTransfers && (TransferList.active > 0 || transferJustFinished)) return "transfer"
        if (Preferences.nectarIslandMedia && hasMediaCandidate) return "media"
        // Kept on screen while the phone has notifications to read, not only
        // for the first few seconds after one arrives. Paused pop-ups stay off
        // the Island too.
        if (Preferences.nectarIslandNotifications && !AppController.notificationsPaused
                && NotificationList.count > 0) return "notification"
        if (Preferences.nectarIslandTransfers && TransferList.waitingCount > 0) return "waiting"
        return "idle"
    }

    readonly property bool hasActivity: mode !== "idle"
    /// What the pill's count shows; 0 when the Island doesn't take phone alerts.
    readonly property int unreadCount: Preferences.nectarIslandNotifications ? NotificationList.count : 0
    // Input stays on while the panel is open, even if the last notification
    // just went away, so the pill can still be clicked and dragged.
    readonly property bool interactive: hasActivity || panelShown
        || Preferences.nectarIslandAlwaysVisible
    readonly property bool onWayland: Qt.platform.pluginName.startsWith("wayland")
    // Wayland lets the Island be dragged, but a window that hides comes back
    // wherever the compositor puts it. So after activity it stays, empty and
    // letting clicks through, for a while: a song paused and played again keeps
    // its spot. Then it hides, out of the window switcher.
    property bool keptForMinutes: false
    onHasActivityChanged: {
        if (hasActivity) {
            keepTimer.stop()
            // Coming back while kept: the window is already up, so drop in here.
            if (keptForMinutes && visible && !Theme.reduceMotion)
                dropInAnim.restart()
            // Not on Niri: there an empty window still catches clicks (D511).
            keptForMinutes = onWayland && !AppController.onNiri
        } else if (keptForMinutes) {
            keepTimer.restart()
        }
    }
    Timer {
        id: keepTimer
        interval: 10 * 60 * 1000
        onTriggered: islandWin.keptForMinutes = false
    }
    // The panel keeps the pill around while it is open, even if the last
    // notification or song just went away.
    readonly property bool shouldBeVisible: Preferences.nectarIsland
        && (hasActivity || keptForMinutes || panelShown || Preferences.nectarIslandAlwaysVisible)
    visible: false
    // Own the window's visibility instead of relying on a binding: Windows can
    // hide a tool window itself (a full-screen app, a display change, a session
    // switch), and a binding that saw that once would stay broken. Re-asserting
    // it also brings the Island back when the platform hid it.
    function syncVisible() {
        if (visible !== shouldBeVisible)
            visible = shouldBeVisible
        if (shouldBeVisible) {
            show()
            AppController.keepIslandVisible(true, title)
        }
    }
    onShouldBeVisibleChanged: syncVisible()
    onVisibleChanged: {
        if (!visible) {
            keepTimer.stop()
            keptForMinutes = false
            hideTip(null)
            closePanel()
        }
        if (visible) {
            panelSuppressed = false
            syncClickThrough()
            Qt.callLater(() => AppController.styleShelfWindow())
            if (!Theme.reduceMotion) {
                dropInAnim.restart()
                transitionSettle.restart()
            }
        }
    }
    Timer {
        interval: 2000
        repeat: true
        running: true
        onTriggered: islandWin.syncVisible()
    }

    // ---- Hover panel below the pill ----
    // Hovering shows it; clicking the pill pins it so its buttons and the
    // phone's notifications stay reachable while the pointer moves away.
    property bool panelPinned: false
    property bool panelHovered: false
    property bool panelShown: false
    // The Island's own settings page, inside the expanded card. It never pins
    // the card: leaving the pointer collapses everything as usual.
    property bool islandSettingsOpen: false
    property real islandSettingsPanelHeight: 0
    property int cursorFailures: 0
    function openIslandSettings() {
        // Keep the size the card already has: opening settings must not grow
        // or shrink the Island. Its contents scroll inside that height.
        const current = card && card.height > 0 ? card.height - 32 : 0
        islandSettingsPanelHeight = Math.max(80, Math.round(current))
        panelSuppressed = false
        panelPinned = false
        islandSettingsOpen = true
        panelShown = true
    }
    function closeIslandSettings() {
        if (!islandSettingsOpen)
            return
        islandSettingsOpen = false
    }

    // A double-click's second tap must not also pin the panel.
    property bool suppressTap: false
    // After the close button, don't immediately reopen while the pointer is
    // still over the Island; wait until it leaves.
    property bool panelSuppressed: false
    readonly property bool panelWanted: !panelSuppressed && (pointerOverIsland || panelHovered || panelPinned)
    onPanelWantedChanged: {
        if (panelWanted) {
            panelCloseTimer.stop()
            panelShown = true
        } else {
            panelCloseTimer.restart()
        }
    }
    Timer {
        id: panelCloseTimer
        interval: 140
        onTriggered: if (!islandWin.panelWanted) islandWin.panelShown = false
    }
    // Hover can go stale when the card disappears under the pointer or the
    // window moves. Check the real cursor now and then and close an unpinned
    // panel once it is outside the Island.
    onPanelShownChanged: {
        if (!panelShown) {
            panelHovered = false
            islandSettingsOpen = false
            cursorFailures = 0
            hideTip(null)
        }
    }
    Timer {
        id: cursorWatch
        // Click-through windows get no Qt hover events, so this poll is their
        // hover. Nothing else polls the cursor: the old occlusion avoidance
        // and its second window scan every 600 ms are gone.
        interval: 120
        repeat: true
        running: islandWin.visible && Qt.platform.os === "windows"
            && islandWin.clickThroughActive
        onTriggered: {
            const cx = AppController.cursorX()
            const cy = AppController.cursorY()
            if (cx < 0 || cy < 0) {
                islandWin.clickThroughHovered = false
                return
            }
            const scale = islandWin.dpr > 0 ? islandWin.dpr : 1
            const px = cx / scale
            const py = cy / scale
            const inside = px >= islandWin.x && px <= islandWin.x + islandWin.width
                && py >= islandWin.y && py <= islandWin.y + islandWin.height
            islandWin.clickThroughHovered = inside
        }
    }
    Timer {
        id: pointerLeaveWatch
        interval: 220
        repeat: true
        running: islandWin.panelShown && !islandWin.panelPinned
            && Qt.platform.os === "windows"
        onTriggered: {
            const cx = AppController.cursorX()
            const cy = AppController.cursorY()
            if (cx < 0 || cy < 0) {
                // If the cursor can't be read (session switch, locked desktop),
                // don't leave an unpinned card open over everything.
                islandWin.cursorFailures++
                if (islandWin.cursorFailures >= 5) {
                    islandWin.cursorFailures = 0
                    islandWin.panelHovered = false
                    islandWin.panelSuppressed = false
                    islandWin.panelShown = false
                    panelCloseTimer.stop()
                }
                return
            }
            islandWin.cursorFailures = 0
            const scale = islandWin.dpr > 0 ? islandWin.dpr : 1
            const px = cx / scale
            const py = cy / scale
            const inside = px >= islandWin.x - 4 && px <= islandWin.x + islandWin.width + 4
                && py >= islandWin.y - 4 && py <= islandWin.y + islandWin.height + 8
            if (!inside) {
                islandWin.panelHovered = false
                islandWin.panelSuppressed = false
                islandWin.panelShown = false
                panelCloseTimer.stop()
            }
        }
    }
    function closePanel() {
        panelCloseTimer.stop()
        panelPinned = false
        panelHovered = false
        panelShown = false
        islandSettingsOpen = false
        panelSuppressed = true
    }
    function togglePanelPin() {
        panelSuppressed = false
        if (panelShown && panelPinned)
            panelPinned = false
        else
            panelPinned = true
        panelShown = true
    }
    onPanelHoveredChanged: if (!panelHovered && !pointerOverIsland) panelSuppressed = false
    // Whether a point on the pill is one of its own buttons, so a click or
    // double-click there is not also the pill's.
    function isPillControl(point) {
        if (compactMediaControls.visible) {
            const p1 = compactMediaControls.mapFromItem(pillBg, point)
            if (p1.x >= 0 && p1.y >= 0 && p1.x <= compactMediaControls.width && p1.y <= compactMediaControls.height)
                return true
        }
        if (expandedControlsRow.visible) {
            const p2 = expandedControlsRow.mapFromItem(pillBg, point)
            if (p2.x >= 0 && p2.y >= 0 && p2.x <= expandedControlsRow.width && p2.y <= expandedControlsRow.height)
                return true
        }
        return false
    }
    function openMain() {
        closePanel()
        AppController.currentPage = "home"
        AppController.activateRequested()
    }
    function resetIslandPosition() {
        Preferences.saveIslandPosition(false, 0, 0)
    }
    // The window system moved the pill; keep it on screen and remember where.
    function finishIslandMove() {
        if (onWayland)
            return
        const maxX = Math.max(0, Math.round(areaW - width))
        const maxY = Math.max(0, Math.round(areaH - height))
        const nx = Math.max(0, Math.min(Math.round(x - areaX), maxX))
        const ny = Math.max(0, Math.min(Math.round(y - areaY), maxY))
        Preferences.saveIslandPosition(true, nx, ny)
    }
    // A monitor change can leave a remembered position off screen.
    function clampCustomPosition() {
        if (!customPosition)
            return
        const maxX = Math.max(0, Math.round(areaW - width))
        const maxY = Math.max(0, Math.round(areaH - height))
        const nx = Math.max(0, Math.min(Preferences.nectarIslandX, maxX))
        const ny = Math.max(0, Math.min(Preferences.nectarIslandY, maxY))
        if (nx !== Preferences.nectarIslandX || ny !== Preferences.nectarIslandY)
            Preferences.saveIslandPosition(true, nx, ny)
    }

    Component.onCompleted: {
        lastNotificationCount = NotificationList.count
        if (expanded)
            windowRoomy = true
        syncVisible()
        Qt.callLater(() => {
            recomputeMedia()
            recomputeTransfer()
            clampCustomPosition()
            syncClickThrough()
            AppController.styleShelfWindow()
        })
    }

    Connections {
        target: Preferences
        function onNectarIslandChanged() {
            if (Preferences.nectarIsland) {
                Qt.callLater(() => AppController.styleShelfWindow())
            }
        }
        function onNectarIslandPcMediaChanged() { islandWin.recomputeMedia() }
        function onNectarIslandMediaSourceChanged() { islandWin.recomputeMedia() }
    }

    readonly property bool expanded: pointerOverIsland || panelShown
        || (Preferences.nectarIslandCalls && PhoneCall.active && PhoneCall.ringing)
    readonly property bool sideDock: Preferences.nectarIslandEdge !== "top"
    readonly property bool sideRight: Preferences.nectarIslandEdge === "right"
    readonly property real sideRailW: 52
    readonly property real sideRailH: 200
    property bool windowHovered: false
    // Click-through windows get no Qt hover events, so the cursor watchdog
    // below fills this in; otherwise the normal hover handlers drive it.
    property bool clickThroughHovered: false
    readonly property bool pointerOverIsland: clickThroughActive
        ? clickThroughHovered
        : (windowHovered || pillHover.hovered || sideHover.hovered)
    // One collapsed size for every mode and state: a 340x46 capsule. There is
    // no thin line and no occlusion shrinking; the width never changes on
    // hover, and opening only adds height.
    readonly property real compactPillHeight: 46
    readonly property real pillHeight: expanded ? 88 : compactPillHeight
    readonly property real compactWindowHeight: compactPillHeight
    // Never fully invisible: the Island stays findable and clickable.
    readonly property real contentOpacity:
        Math.max(0.3, Math.min(1, Preferences.nectarIslandOpacity / 100))
    // Open, the Island is exactly eight times its collapsed height. The card
    // scrolls inside it; the native window is only resized per open/close.
    readonly property real expandedHeight: Math.round(compactPillHeight * 8)
    readonly property real expandedCardHeight: sideDock
        ? expandedHeight : Math.max(0, expandedHeight - (88 - 6))
    property bool windowRoomy: false
    Timer {
        id: roomyHold
        // Longer than the pill tween above: the native window gives up its
        // extra height only after the visible collapse has finished.
        interval: islandWin.morphDuration + 80
        repeat: false
        onTriggered: islandWin.windowRoomy = false
    }
    // The card sits on the pill's bottom edge and travels with it while the
    // pill stretches, so the two never separate mid-animation.
    // The pill keeps its width when it opens: the Island grows down only.
    // Side docks grow inward from the chosen edge.
    width: sideDock
        ? (windowRoomy ? 340 : sideRailW) * islandScale
        : 340 * islandScale
    height: sideDock
        ? (windowRoomy ? expandedHeight : sideRailH) * islandScale
        : (windowRoomy ? expandedHeight : compactWindowHeight) * islandScale
    x: sideDock
        ? (sideRight ? areaX + areaW - width : areaX)
        : (customPosition ? areaX + Preferences.nectarIslandX : defaultX)
    y: sideDock
        ? areaY + Math.round((areaH - height) / 2)
        : (customPosition ? areaY + Preferences.nectarIslandY : defaultY)

    readonly property string primaryLabel: {
        if (mode === "call")
            return PhoneCall.caller.length > 0 ? PhoneCall.caller : qsTr("Phone call")
        if (mode === "transfer") {
            if (transferJustFinished && TransferList.active === 0)
                return transferFailed ? qsTr("Transfer failed") : qsTr("Transfer complete")
            return activeTransferTitle.length > 0
                ? activeTransferTitle
                : qsTr("Transferring %1 file(s)…").arg(TransferList.active)
        }
        if (mode === "media")
            return mediaTitle.length > 0 ? mediaTitle : (mediaApp.length > 0 ? mediaApp : qsTr("Now playing"))
        if (mode === "notification")
            return NotificationList.latestTitle.length > 0
                ? NotificationList.latestTitle
                : (NotificationList.latestApp.length > 0
                    ? NotificationList.latestApp
                    : qsTr("%1 phone notification(s)").arg(NotificationList.count))
        if (mode === "waiting")
            return qsTr("%1 waiting").arg(TransferList.waitingCount)
        return DeviceList.count > 0
            ? qsTr("Nectar Island · %1").arg(DeviceList.deviceNameAt(0))
            : qsTr("Nectar Island · Ready")
    }

    // The call's clock, ticking once a second while a call is shown.
    property real callNow: Date.now()
    property string clockText: ""
    Timer {
        interval: 1000
        repeat: true
        running: islandWin.visible && Preferences.nectarIslandClock
        triggeredOnStart: true
        onTriggered: {
            const now = new Date()
            islandWin.clockText = Qt.formatTime(now,
                Preferences.nectarIslandClock12h ? "h:mm AP" : "HH:mm")
        }
    }

    // ---- Hover hints ----
    // A small bubble that follows the pointer over the Island's icon buttons.
    property string tipText: ""
    property string tipPending: ""
    property Item tipOwner: null
    property real tipX: 0
    property real tipY: 0
    Timer {
        id: tipWait
        interval: 450
        repeat: false
        onTriggered: if (islandWin.tipOwner) islandWin.tipText = islandWin.tipPending
    }
    function showTip(text, owner, point) {
        if (!owner || !text || text.length === 0) {
            hideTip(owner)
            return
        }
        if (tipOwner !== owner) {
            tipText = ""
            tipOwner = owner
        }
        tipPending = text
        moveTip(owner, point)
        if (tipText.length === 0)
            tipWait.restart()
        else
            tipText = text
    }
    function moveTip(owner, point) {
        if (!owner || tipOwner !== owner)
            return
        const p = owner.mapToItem(root, point.x, point.y)
        tipX = p.x
        tipY = p.y
    }
    function hideTip(owner) {
        if (owner && tipOwner !== owner)
            return
        tipWait.stop()
        tipOwner = null
        tipPending = ""
        tipText = ""
    }
    Timer {
        interval: 1000
        repeat: true
        running: islandWin.visible && islandWin.mode === "call" && PhoneCall.since > 0
        triggeredOnStart: true
        onTriggered: islandWin.callNow = Date.now()
    }
    readonly property string callClock: {
        if (PhoneCall.since <= 0) return ""
        const s = Math.max(0, Math.floor((callNow - PhoneCall.since) / 1000))
        const h = Math.floor(s / 3600)
        const m = Math.floor((s % 3600) / 60)
        const ss = String(s % 60).padStart(2, "0")
        return h > 0 ? h + ":" + String(m).padStart(2, "0") + ":" + ss : m + ":" + ss
    }

    readonly property string secondaryLabel: {
        if (mode === "call") {
            if (PhoneCall.ringing) return qsTr("Incoming call")
            // How long, where the sound is, and whether the other side hears this end.
            const parts = []
            if (callClock.length > 0) parts.push(callClock)
            if (PhoneCall.held) parts.push(qsTr("On hold"))
            if (PhoneCall.app.length > 0) parts.push(PhoneCall.app)
            parts.push(PhoneCall.callAudio ? qsTr("On PC") : qsTr("On phone"))
            if (PhoneCall.controls) parts.push(PhoneCall.muted ? qsTr("Mic off") : qsTr("Mic on"))
            return parts.join(" · ")
        }
        if (mode === "transfer") {
            if (transferJustFinished && TransferList.active === 0)
                return finishedTransferTitle
            const pct = Math.round(activeTransferProgress * 100) + "%"
            const spd = formatRate(activeTransferRate)
            const base = spd.length > 0 ? (pct + " · " + spd) : pct
            return TransferList.waitingCount > 0
                ? (base + " · " + qsTr("%1 waiting").arg(TransferList.waitingCount))
                : base
        }
        if (mode === "media") {
            const src = mediaDeviceId === "local" ? qsTr("This PC") : qsTr("Phone")
            const base = mediaArtist.length > 0
                ? (mediaArtist + " · " + src)
                : (mediaApp.length > 0 ? (mediaApp + " · " + src) : src)
            return TransferList.waitingCount > 0
                ? (base + " · " + qsTr("%1 waiting").arg(TransferList.waitingCount))
                : base
        }
        if (mode === "notification") {
            if (NotificationList.latestApp.length === 0)
                return qsTr("Click to view in NectarLink")
            return NotificationList.count > 1
                ? qsTr("%1 · %n notification(s)", "", NotificationList.count).arg(NotificationList.latestApp)
                : NotificationList.latestApp
        }
        if (mode === "waiting") {
            return DeviceList.count > 0
                ? qsTr("Will send when %1 is back").arg(DeviceList.deviceNameAt(0))
                : qsTr("Will send when connected")
        }
        return qsTr("Live calls, transfers & media")
    }

    Item {
        id: root
        width: islandWin.width / islandWin.islandScale
        height: islandWin.height / islandWin.islandScale
        scale: islandWin.islandScale
        transformOrigin: Item.TopLeft
        visible: islandWin.interactive
        // The whole Island, including its content, can be made translucent
        // from Settings. Input and the window bounds stay exact at any level.
        opacity: islandWin.contentOpacity
        Behavior on opacity {
            enabled: !Theme.reduceMotion
            NumberAnimation { duration: 150; easing.type: Easing.OutCubic }
        }

        // A window-wide hover area: the transparent margins are still part of
        // the Island, so approaching it opens the pill.
        HoverHandler {
            id: rootHover
            onHoveredChanged: islandWin.windowHovered = hovered
        }

        // Hidden repeaters inside Item scope so QtQuick instantiates delegates cleanly
        Repeater {
            id: mediaRepeater
            model: Preferences.nectarIsland ? MediaList : null
            onItemAdded: islandWin.recomputeMedia()
            onItemRemoved: islandWin.recomputeMedia()
            delegate: Item {
                required property int index
                required property string deviceId
                required property string playerId
                required property string app
                required property string title
                required property string artist
                required property string art
                required property bool playing
                required property bool canPlay
                required property bool canPause
                required property bool canNext
                required property bool canPrevious
                required property bool canSeek
                required property real duration
                required property real position
                required property real positionAt
                visible: false
                onPlayingChanged: islandWin.recomputeMedia()
                onTitleChanged: islandWin.recomputeMedia()
                onArtistChanged: islandWin.recomputeMedia()
                onArtChanged: islandWin.recomputeMedia()
                onDeviceIdChanged: islandWin.recomputeMedia()
                onPositionChanged: islandWin.recomputeMedia()
                onDurationChanged: islandWin.recomputeMedia()
                onCanPlayChanged: islandWin.recomputeMedia()
                onCanPauseChanged: islandWin.recomputeMedia()
                onCanNextChanged: islandWin.recomputeMedia()
                onCanSeekChanged: islandWin.recomputeMedia()
                onCanPreviousChanged: islandWin.recomputeMedia()
            }
        }

        Repeater {
            id: transferRepeater
            model: Preferences.nectarIsland ? TransferList : null
            onItemAdded: islandWin.recomputeTransfer()
            onItemRemoved: islandWin.recomputeTransfer()
            delegate: Item {
                required property int index
                required property string transferId
                required property string title
                required property real progress
                required property real rate
                required property bool incoming
                required property string status
                visible: false
                // Creating a row sets its status too, and an old row says nothing new.
                property bool created: false
                Component.onCompleted: created = true
                onStatusChanged: {
                    if (created)
                        islandWin.noteTransferEnded(status, title)
                    islandWin.recomputeTransfer()
                }
                onProgressChanged: islandWin.recomputeTransfer()
                onRateChanged: islandWin.recomputeTransfer()
                onTitleChanged: islandWin.recomputeTransfer()
            }
        }

        transform: Translate {
            id: dropSlide
            y: 0
        }

        NumberAnimation {
            id: dropInAnim
            target: dropSlide
            property: "y"
            from: -18
            to: 0
            duration: 240
            easing.type: Easing.OutBack
        }

        // While the card is open the pill and the card share one surface, so
        // the Island itself stretches down instead of looking like a separate
        // window appearing underneath it.
        Rectangle {
            id: stretchedSurface
            visible: islandWin.panelShown && !islandWin.sideDock
            x: 0
            y: 0
            width: parent.width
            height: Math.max(0, card.y + card.height)
            radius: islandWin.shape === "notch" ? 20 : 28
            color: islandWin.islandSurfaceColor
            border.width: 1
            border.color: islandWin.mode === "call"
                ? Qt.rgba(0.29, 0.87, 0.50, 0.55)
                : islandWin.islandBorderColor
        }

        // Subtle outer ambient halo around the Dynamic Island capsule
        Rectangle {
            anchors.centerIn: pillBg
            visible: islandWin.expanded && !islandWin.panelShown
            width: pillBg.width + 6
            height: pillBg.height + 6
            radius: height / 2
            color: "transparent"
            border.width: 1.5
            border.color: islandWin.mode === "call"
                ? Qt.rgba(0.29, 0.87, 0.50, 0.32)
                : Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.24)
        }

        Rectangle {
            id: pillBg
            visible: !islandWin.sideDock
            x: 0
            anchors.top: parent.top
            anchors.topMargin: 0
            width: parent.width
            height: islandWin.pillHeight
            radius: islandWin.shape === "notch" ? 14 : height / 2
            // While the card is open this is the top of the one stretched
            // surface behind it (stretchedSurface), not a second pill.
            color: "transparent"
            border.width: islandWin.panelShown ? 0 : 1
            border.color: islandWin.mode === "call"
                ? Qt.rgba(0.29, 0.87, 0.50, 0.55)
                : (islandWin.notificationToastActive
                    ? Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.62)
                    : islandWin.islandBorderColor)
            Behavior on border.color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 260 } }
            clip: true

            Rectangle {
                anchors.fill: parent
                radius: parent.radius
                visible: !islandWin.panelShown
                gradient: Gradient {
                    GradientStop { position: 0; color: islandWin.islandTopColor }
                    GradientStop { position: 1; color: islandWin.islandBottomColor }
                }
            }

            Behavior on width {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: islandWin.morphDuration; easing.type: Easing.OutCubic }
            }
            Behavior on height {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: islandWin.morphDuration; easing.type: Easing.OutCubic }
            }

            HoverHandler {
                id: pillHover
                cursorShape: Qt.PointingHandCursor
                onHoveredChanged: {
                    if (!hovered && !islandWin.panelHovered)
                        islandWin.panelSuppressed = false
                }
            }

            // Drag the pill anywhere; the window system does the moving, so
            // Wayland's compositor keeps control there.
            DragHandler {
                target: null
                onActiveChanged: {
                    if (active) {
                        islandMoveSettle.stop()
                        islandTapTimer.stop()
                        islandWin.panelPinned = false
                        islandWin.panelShown = false
                        panelCloseTimer.stop()
                        islandWin.startSystemMove()
                    } else {
                        // Read the position once the system move has settled.
                        islandMoveSettle.restart()
                    }
                }
            }
            Timer {
                id: islandMoveSettle
                interval: 160
                onTriggered: islandWin.finishIslandMove()
            }

            // A click pins the panel; a double-click puts the pill back where
            // it started. The short timer keeps a double-click from also
            // pinning the panel first.
            TapHandler {
                onTapped: (eventPoint) => {
                    if (islandWin.suppressTap) {
                        islandWin.suppressTap = false
                        return
                    }
                    if (!islandWin.isPillControl(eventPoint.position))
                        islandTapTimer.restart()
                }
                onDoubleTapped: (eventPoint) => {
                    if (islandWin.isPillControl(eventPoint.position))
                        return
                    islandWin.suppressTap = true
                    suppressTapTimer.restart()
                    islandTapTimer.stop()
                    islandWin.resetIslandPosition()
                }
            }
            Timer {
                id: islandTapTimer
                interval: 190
                onTriggered: islandWin.togglePanelPin()
            }
            Timer {
                id: suppressTapTimer
                interval: 400
                onTriggered: islandWin.suppressTap = false
            }

            // ================================================================
            // COMPACT PILL VIEW (340×46px)
            // ================================================================
            Item {
                id: compactView
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 10
                visible: !islandWin.expanded
                opacity: (islandWin.expanded ? 0.0 : 1.0) * islandWin.modeFade
                Behavior on opacity { NumberAnimation { duration: 140 } }

                // Left slot: Album Art / Progress Ring / Call Pulse / Brand Badge
                Item {
                    id: compactLeftSlot
                    width: 26
                    height: 26
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter

                    // 1. Media album art squircle
                    Rectangle {
                        anchors.fill: parent
                        radius: 8
                        visible: islandWin.mode === "media"
                        color: Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.24)
                        Icon {
                            anchors.centerIn: parent
                            width: 14; height: 14
                            visible: !(islandWin.mediaArt.length > 0 && compactArtImg.status === Image.Ready)
                            path: Icons.music
                            color: Theme.primary
                        }
                        RoundedImage {
                            id: compactArtImg
                            anchors.fill: parent
                            radius: 8
                            source: islandWin.mediaArt
                            sourceSize: Qt.size(64, 64)
                        }
                    }

                    // 2. Live file transfer circular progress ring
                    Item {
                        anchors.fill: parent
                        visible: islandWin.mode === "transfer"

                        Shape {
                            anchors.fill: parent
                            preferredRendererType: Shape.CurveRenderer

                            ShapePath {
                                strokeColor: islandWin.wash(0.16)
                                fillColor: "transparent"
                                strokeWidth: 2.6
                                capStyle: ShapePath.RoundCap
                                PathAngleArc {
                                    centerX: 13; centerY: 13
                                    radiusX: 10.5; radiusY: 10.5
                                    startAngle: 0; sweepAngle: 360
                                }
                            }
                            ShapePath {
                                strokeColor: islandWin.transferJustFinished ? islandWin.transferEndColor : Theme.primary
                                fillColor: "transparent"
                                strokeWidth: 2.6
                                capStyle: ShapePath.RoundCap
                                PathAngleArc {
                                    centerX: 13; centerY: 13
                                    radiusX: 10.5; radiusY: 10.5
                                    startAngle: -90
                                    sweepAngle: 360 * (islandWin.transferJustFinished ? 1.0 : Math.max(0.06, islandWin.activeTransferProgress))
                                }
                            }
                        }

                        Icon {
                            anchors.centerIn: parent
                            width: 11; height: 11
                            path: !islandWin.transferJustFinished ? Icons.send
                                : (islandWin.transferFailed ? Icons.close : Icons.check)
                            color: islandWin.transferJustFinished ? islandWin.transferEndColor : islandWin.ink
                        }
                    }

                    // 3. Call / Notification / Waiting / Ready badge
                    Rectangle {
                        anchors.fill: parent
                        radius: 13
                        visible: islandWin.mode === "call" || islandWin.mode === "notification" || islandWin.mode === "waiting" || islandWin.mode === "ready" || islandWin.mode === "idle"
                        color: islandWin.mode === "call"
                            ? Qt.rgba(0.29, 0.87, 0.50, 0.22)
                            : Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.22)
                        Icon {
                            anchors.centerIn: parent
                            visible: !(islandWin.mode === "notification" && appIcon14.status === Image.Ready)
                            width: 14; height: 14
                            path: islandWin.mode === "call"
                                ? Icons.call
                                : (islandWin.mode === "notification"
                                    ? Icons.bell
                                    : (islandWin.mode === "waiting"
                                        ? Icons.send
                                        : (islandWin.mode === "idle" ? Icons.sparkle : Icons.phone)))
                            color: islandWin.mode === "call" ? "#4ADE80" : Theme.primary
                        }
                        RoundedImage {
                            id: appIcon14
                            anchors.fill: parent
                            radius: 13
                            visible: islandWin.mode === "notification" && status === Image.Ready
                            source: islandWin.mode === "notification" ? NotificationList.latestIcon : ""
                            sourceSize: Qt.size(96, 96)
                        }
                    }
                }

                // Right slot: Equalizer + inline quick transport controls (or % badge)
                Row {
                    id: compactMediaControls
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 4

                    // A call in progress: where its sound is, the microphone, and how long.
                    Row {
                        id: compactCallStatus
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 6
                        visible: islandWin.mode === "call" && !PhoneCall.ringing
                        Accessible.role: Accessible.StaticText
                        Accessible.name: islandWin.secondaryLabel
                        Icon {
                            anchors.verticalCenter: parent.verticalCenter
                            width: 12; height: 12
                            path: PhoneCall.callAudio ? Icons.desktop : Icons.phone
                            color: PhoneCall.callAudio ? "#4ADE80" : islandWin.wash(0.7)
                        }
                        Icon {
                            visible: PhoneCall.controls
                            anchors.verticalCenter: parent.verticalCenter
                            width: 12; height: 12
                            path: PhoneCall.muted ? Icons.micOff : Icons.mic
                            color: PhoneCall.muted ? "#EF4444" : islandWin.wash(0.7)
                        }
                        Txt {
                            visible: text.length > 0
                            anchors.verticalCenter: parent.verticalCenter
                            role: "label"
                            size: 12
                            color: PhoneCall.held ? "#FBBF24" : islandWin.wash(0.85)
                            text: islandWin.callClock
                        }
                    }

                    // Animated 3-bar equalizer in media mode
                    Row {
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 1.8
                        height: 12
                        visible: islandWin.mode === "media"
                        Repeater {
                            model: 3
                            delegate: Rectangle {
                                id: cEqBar
                                required property int index
                                width: 2.2
                                radius: 1.1
                                anchors.bottom: parent.bottom
                                color: Theme.primary
                                height: islandWin.mediaPlaying ? 8 : 3
                                SequentialAnimation on height {
                                    running: islandWin.visible && !islandWin.expanded && islandWin.mediaPlaying && !Theme.reduceMotion
                                    loops: Animation.Infinite
                                    NumberAnimation {
                                        from: 3
                                        to: cEqBar.index === 1 ? 12 : (cEqBar.index === 0 ? 9 : 10)
                                        duration: 240 + cEqBar.index * 80
                                        easing.type: Easing.InOutSine
                                    }
                                    NumberAnimation {
                                        to: 3
                                        duration: 220 + cEqBar.index * 70
                                        easing.type: Easing.InOutSine
                                    }
                                }
                            }
                        }
                    }

                    // Quick Play/Pause button right in the compact pill
                    Rectangle {
                        id: compactPlayButton
                        visible: islandWin.mode === "media"
                        width: 24; height: 24
                        radius: 12
                        anchors.verticalCenter: parent.verticalCenter
                        color: cPlayHover.hovered ? islandWin.wash(0.18) : islandWin.wash(0.08)
                        Icon {
                            anchors.centerIn: parent
                            width: 12; height: 12
                            path: islandWin.mediaPlaying ? Icons.pause : Icons.play
                            color: islandWin.ink
                        }
                        HoverHandler { id: cPlayHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: MediaList.command(
                                islandWin.mediaDeviceId,
                                islandWin.mediaPlayerId,
                                islandWin.mediaPlaying ? "pause" : "play",
                                0
                            )
                        }
                        IslandTipHover {
                            island: islandWin
                            tip: islandWin.mediaPlaying ? qsTr("Pause") : qsTr("Play")
                            owner: compactPlayButton
                        }
                    }

                    // Quick Next Track button right in the compact pill
                    Rectangle {
                        id: compactNextButton
                        visible: islandWin.mode === "media" && islandWin.mediaCanNext
                        width: 24; height: 24
                        radius: 12
                        anchors.verticalCenter: parent.verticalCenter
                        color: cNextHover.hovered ? islandWin.wash(0.18) : "transparent"
                        Icon {
                            anchors.centerIn: parent
                            width: 12; height: 12
                            path: Icons.skipNext
                            color: islandWin.wash(0.85)
                        }
                        HoverHandler { id: cNextHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: MediaList.command(islandWin.mediaDeviceId, islandWin.mediaPlayerId, "next", 0)
                        }
                        IslandTipHover { island: islandWin; tip: qsTr("Next track"); owner: compactNextButton }
                    }

                    // Transfer percentage pill
                    Txt {
                        visible: islandWin.mode === "transfer" && !islandWin.transferJustFinished
                        anchors.verticalCenter: parent.verticalCenter
                        role: "caption"
                        size: 11
                        weight: Font.Bold
                        color: Theme.primary
                        text: Math.round(islandWin.activeTransferProgress * 100) + "%"
                    }

                    // Separates the player controls from the unread count.
                    Rectangle {
                        visible: islandWin.mode === "media"
                        width: 1
                        height: 16
                        radius: 0.5
                        anchors.verticalCenter: parent.verticalCenter
                        color: islandWin.wash(0.18)
                    }

                    // Unread phone notifications, to the right of the player.
                    // Grey with 0 when there are none, red with the count otherwise.
                    Rectangle {
                        id: compactUnreadBadge
                        readonly property int count: islandWin.unreadCount
                        property int previousCount: 0
                        onCountChanged: {
                            if (count > previousCount && !Theme.reduceMotion)
                                countBump.restart()
                            previousCount = count
                        }
                        width: 24
                        height: 24
                        radius: 12
                        anchors.verticalCenter: parent.verticalCenter
                        color: count > 0 ? "#FF3B30" : islandWin.wash(0.10)
                        border.width: count > 0 ? 1 : 0
                        border.color: "#FFFFFF"
                        Behavior on color { enabled: !Theme.reduceMotion; ColorAnimation { duration: 140 } }
                        SequentialAnimation {
                            id: countBump
                            NumberAnimation {
                                target: compactUnreadBadge
                                property: "scale"
                                to: 1.18
                                duration: 90
                                easing.type: Easing.OutCubic
                            }
                            NumberAnimation {
                                target: compactUnreadBadge
                                property: "scale"
                                to: 1
                                duration: 170
                                easing.type: Easing.OutBack
                            }
                        }
                        Txt {
                            anchors.centerIn: parent
                            text: compactUnreadBadge.count > 99 ? "99+" : String(compactUnreadBadge.count)
                            role: "mono"
                            size: 10
                            weight: Font.Bold
                            color: compactUnreadBadge.count > 0 ? "#FFFFFF" : islandWin.wash(0.55)
                        }
                        Accessible.role: Accessible.StaticText
                        Accessible.name: qsTr("%1 unread notifications").arg(compactUnreadBadge.count)
                    }
                }

                // Center slot: title, the line being sung, and progress. The
                // lyric is always here, not only after hovering.
                Item {
                    id: compactTitleClip
                    anchors.left: compactLeftSlot.right
                    anchors.leftMargin: 9
                    anchors.right: compactMediaControls.left
                    anchors.rightMargin: 8
                    anchors.verticalCenter: parent.verticalCenter
                    height: islandWin.mode === "media" ? 33 : 24
                    clip: true

                    // The time keeps its own width at the end of the media-name
                    // line; the name gets the rest and scrolls when it needs to.
                    readonly property real clockWidth: compactClock.visible
                        ? Math.max(30, Math.min(Math.max(compactClock.implicitWidth + 4, width * 0.20),
                            width * 0.30))
                        : 0
                    readonly property real titleWidth: Math.max(24,
                        width - clockWidth - (clockWidth > 0 ? 6 : 0))
                    onTitleWidthChanged: compactTitleTxt.x = 0

                    readonly property bool shouldMarquee: compactTitleMetrics.width > titleWidth
                        && islandWin.visible && !islandWin.expanded && !Theme.reduceMotion

                    Txt {
                        id: compactClock
                        visible: Preferences.nectarIslandClock && islandWin.clockText.length > 0
                        anchors.right: parent.right
                        width: parent.clockWidth
                        height: islandWin.mode === "media" ? 15 : parent.height
                        text: islandWin.clockText
                        role: "mono"
                        size: 10.5
                        weight: Font.DemiBold
                        color: islandWin.wash(0.72)
                        horizontalAlignment: Text.AlignRight
                        verticalAlignment: Text.AlignVCenter
                    }

                    TextMetrics {
                        id: compactTitleMetrics
                        font: compactTitleTxt.font
                        text: islandWin.primaryLabel + (islandWin.mediaArtist.length > 0 && islandWin.mode === "media" ? "  ·  " + islandWin.mediaArtist : "")
                    }

                    Item {
                        id: compactTitleViewport
                        width: compactTitleClip.titleWidth
                        height: islandWin.mode === "media" ? 15 : parent.height
                        clip: true

                        Txt {
                            id: compactTitleTxt
                            width: compactTitleClip.shouldMarquee
                                ? compactTitleMetrics.width : parent.width
                            height: parent.height
                            role: "label"
                            size: 11.5
                            weight: Font.DemiBold
                            color: islandWin.ink
                            verticalAlignment: Text.AlignVCenter
                            elide: compactTitleClip.shouldMarquee ? Text.ElideNone : Text.ElideRight
                            text: compactTitleMetrics.text

                            SequentialAnimation on x {
                                running: compactTitleClip.shouldMarquee && islandWin.visible && !islandWin.expanded
                                         && !Theme.reduceMotion
                                onRunningChanged: if (!running) compactTitleTxt.x = 0
                                loops: Animation.Infinite
                                PauseAnimation { duration: 1200 }
                                NumberAnimation {
                                    from: 0
                                    to: -Math.max(0, compactTitleMetrics.width - compactTitleClip.titleWidth + 12)
                                    duration: Math.max(1800, (compactTitleMetrics.width - compactTitleClip.titleWidth) * 28)
                                    easing.type: Easing.InOutSine
                                }
                                PauseAnimation { duration: 900 }
                                NumberAnimation {
                                    to: 0
                                    duration: 900
                                    easing.type: Easing.OutCubic
                                }
                            }
                            onTextChanged: x = 0
                        }
                    }

                    Item {
                        id: compactLyricBox
                        y: 15
                        width: parent.width
                        height: 15
                        visible: islandWin.mode === "media"
                            && (islandWin.lyricLine.length > 0
                                || (islandWin.lyricUnavailable && !islandWin.lyricLoading))

                        KaraokeTxt {
                            anchors.fill: parent
                            visible: islandWin.lyricLine.length > 0
                            text: islandWin.lyricLine
                            progress: islandWin.lyricProgress
                            role: "body"
                            size: 10.5
                            weight: Font.DemiBold
                            baseColor: islandWin.lyricBaseColor
                            fillColor: islandWin.lyricFillColor
                        }

                        Txt {
                            anchors.fill: parent
                            visible: islandWin.lyricLine.length === 0
                                && islandWin.lyricUnavailable && !islandWin.lyricLoading
                            text: qsTr("No text available for this media.")
                            role: "body"
                            size: 10.5
                            color: islandWin.wash(0.52)
                            horizontalAlignment: Text.AlignHCenter
                            verticalAlignment: Text.AlignVCenter
                        }

                        SequentialAnimation {
                            id: compactLyricFlash
                            NumberAnimation {
                                target: compactLyricBox
                                property: "opacity"
                                from: 0.35
                                to: 1
                                duration: 220
                                easing.type: Easing.OutCubic
                            }
                        }
                    }

                    Rectangle {
                        id: compactProgress
                        y: 30.5
                        width: parent.width
                        height: 2.5
                        radius: 1
                        visible: islandWin.mode === "media" && islandWin.mediaDuration > 0
                        color: islandWin.wash(0.14)
                        Rectangle {
                            height: parent.height
                            radius: parent.radius
                            color: Theme.primary
                            width: parent.width * Math.max(0, Math.min(1,
                                islandWin.mediaPositionNow / islandWin.mediaDuration))
                            Behavior on width { NumberAnimation { duration: 400; easing.type: Easing.Linear } }
                        }
                    }
                }
            }

            // ================================================================
            // EXPANDED PILL VIEW (340×88px on hover or incoming call)
            // ================================================================
            Item {
                id: expandedView
                anchors.fill: parent
                anchors.leftMargin: 14
                anchors.rightMargin: 14
                anchors.topMargin: 12
                anchors.bottomMargin: 12
                visible: islandWin.expanded
                opacity: (islandWin.expanded ? 1.0 : 0.0) * islandWin.modeFade
                Behavior on opacity { NumberAnimation { duration: 160 } }

                // Left cover / artwork / progress dial (48×48px)
                Item {
                    id: expLeftSlot
                    width: 48
                    height: 48
                    anchors.left: parent.left
                    anchors.verticalCenter: parent.verticalCenter

                    Rectangle {
                        anchors.fill: parent
                        radius: 12
                        visible: islandWin.mode === "media"
                        color: Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.24)
                        Icon {
                            anchors.centerIn: parent
                            width: 22; height: 22
                            visible: !(islandWin.mediaArt.length > 0 && expArtImg.status === Image.Ready)
                            path: Icons.music
                            color: Theme.primary
                        }
                        RoundedImage {
                            id: expArtImg
                            anchors.fill: parent
                            radius: 12
                            source: islandWin.mediaArt
                            sourceSize: Qt.size(112, 112)
                        }
                    }

                    Item {
                        anchors.fill: parent
                        visible: islandWin.mode === "transfer"

                        Shape {
                            anchors.fill: parent
                            preferredRendererType: Shape.CurveRenderer
                            ShapePath {
                                strokeColor: islandWin.wash(0.16)
                                fillColor: "transparent"
                                strokeWidth: 3.4
                                capStyle: ShapePath.RoundCap
                                PathAngleArc {
                                    centerX: 24; centerY: 24
                                    radiusX: 19; radiusY: 19
                                    startAngle: 0; sweepAngle: 360
                                }
                            }
                            ShapePath {
                                strokeColor: islandWin.transferJustFinished ? islandWin.transferEndColor : Theme.primary
                                fillColor: "transparent"
                                strokeWidth: 3.4
                                capStyle: ShapePath.RoundCap
                                PathAngleArc {
                                    centerX: 24; centerY: 24
                                    radiusX: 19; radiusY: 19
                                    startAngle: -90
                                    sweepAngle: 360 * (islandWin.transferJustFinished ? 1.0 : Math.max(0.06, islandWin.activeTransferProgress))
                                }
                            }
                        }

                        Txt {
                            anchors.centerIn: parent
                            size: 10
                            weight: Font.Bold
                            color: islandWin.ink
                            text: islandWin.transferJustFinished
                                ? (islandWin.transferFailed ? "!" : "100%")
                                : (Math.round(islandWin.activeTransferProgress * 100) + "%")
                        }
                    }

                    Rectangle {
                        anchors.fill: parent
                        radius: 24
                        visible: islandWin.mode === "call" || islandWin.mode === "notification" || islandWin.mode === "waiting" || islandWin.mode === "ready" || islandWin.mode === "idle"
                        color: islandWin.mode === "call"
                            ? Qt.rgba(0.29, 0.87, 0.50, 0.22)
                            : Qt.rgba(Theme.primary.r, Theme.primary.g, Theme.primary.b, 0.22)
                        Icon {
                            anchors.centerIn: parent
                            visible: !(islandWin.mode === "notification" && appIcon22.status === Image.Ready)
                            width: 22; height: 22
                            path: islandWin.mode === "call"
                                ? Icons.call
                                : (islandWin.mode === "notification"
                                    ? Icons.bell
                                    : (islandWin.mode === "waiting"
                                        ? Icons.send
                                        : (islandWin.mode === "idle" ? Icons.sparkle : Icons.phone)))
                            color: islandWin.mode === "call" ? "#4ADE80" : Theme.primary
                        }
                        RoundedImage {
                            id: appIcon22
                            anchors.fill: parent
                            radius: 24
                            visible: islandWin.mode === "notification" && status === Image.Ready
                            source: islandWin.mode === "notification" ? NotificationList.latestIcon : ""
                            sourceSize: Qt.size(96, 96)
                        }
                    }
                }

                // Center title + subtitle + thin progress bar
                Column {
                    anchors.left: expLeftSlot.right
                    anchors.leftMargin: 12
                    anchors.right: expandedControlsRow.left
                    anchors.rightMargin: 10
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 3

                    Txt {
                        width: parent.width
                        role: "title"
                        size: 13
                        weight: Font.DemiBold
                        color: islandWin.ink
                        elide: Text.ElideRight
                        text: islandWin.primaryLabel
                    }

                    Txt {
                        width: parent.width
                        role: "caption"
                        size: 11
                        color: islandWin.wash(0.68)
                        elide: Text.ElideRight
                        text: islandWin.secondaryLabel
                    }

                    // No lyric line here: the open card owns the scrolling
                    // lyrics, and the compact pill keeps its one line.
                    // Sleek progress bar for media or transfer
                    Rectangle {
                        visible: (islandWin.mode === "media" && islandWin.mediaDuration > 0) || islandWin.mode === "transfer"
                        width: parent.width
                        height: 3.5
                        radius: 1.75
                        color: islandWin.wash(0.16)

                        Rectangle {
                            height: parent.height
                            radius: parent.radius
                            color: islandWin.mode === "transfer" && islandWin.transferJustFinished ? islandWin.transferEndColor : Theme.primary
                            width: {
                                if (islandWin.mode === "transfer")
                                    return parent.width * (islandWin.transferJustFinished ? 1.0 : Math.max(0.04, islandWin.activeTransferProgress))
                                if (islandWin.mediaDuration > 0)
                                    return parent.width * Math.max(0, Math.min(1, islandWin.mediaPositionNow / islandWin.mediaDuration))
                                return 0
                            }
                            Behavior on width { NumberAnimation { duration: 180; easing.type: Easing.OutCubic } }
                        }

                        // Seek by clicking or dragging the bar.
                        MouseArea {
                            visible: islandWin.mode === "media" && islandWin.mediaDuration > 0
                                && islandWin.mediaCanSeek
                            width: parent.width
                            height: parent.height + 12
                            y: -6
                            cursorShape: Qt.PointingHandCursor

                            function seekAt(x) {
                                const ratio = Math.max(0, Math.min(1, x / width))
                                MediaList.command(
                                    islandWin.mediaDeviceId,
                                    islandWin.mediaPlayerId,
                                    "seek",
                                    Math.round(ratio * islandWin.mediaDuration)
                                )
                            }
                            onClicked: seekAt(mouse.x)
                            onPositionChanged: if (pressed) seekAt(mouse.x)
                        }
                    }
                }

                // Right action buttons in expanded pill
                Row {
                    id: expandedControlsRow
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 6

                    // ---- Call controls: Answer (when ringing), Mute, Speaker, Hang up ----
                    Rectangle {
                        id: answerButton
                        visible: islandWin.mode === "call" && PhoneCall.ringing && PhoneCall.canEnd
                        width: 32; height: 32; radius: 16
                        color: "#22C55E"
                        Icon {
                            anchors.centerIn: parent
                            width: 15; height: 15
                            path: Icons.call
                            color: "#FFFFFF"
                        }
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: PhoneCall.answer() }
                        IslandTipHover { island: islandWin; tip: qsTr("Answer"); owner: answerButton }
                    }

                    Rectangle {
                        id: muteButton
                        visible: islandWin.mode === "call" && !PhoneCall.ringing && PhoneCall.controls
                        width: 30; height: 30; radius: 15
                        color: PhoneCall.muted ? islandWin.wash(0.26) : islandWin.wash(0.12)
                        Icon {
                            anchors.centerIn: parent
                            width: 14; height: 14
                            path: PhoneCall.muted ? Icons.micOff : Icons.mic
                            color: islandWin.ink
                        }
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: PhoneCall.setMute(!PhoneCall.muted) }
                        IslandTipHover {
                            island: islandWin
                            tip: PhoneCall.muted ? qsTr("Unmute") : qsTr("Mute")
                            owner: muteButton
                        }
                    }

                    // The call's sound on this PC (call.audio), lit while it's here.
                    Rectangle {
                        id: callAudioButton
                        visible: islandWin.mode === "call" && !PhoneCall.ringing && PhoneCall.callAudioAvailable
                        width: 30; height: 30; radius: 15
                        color: PhoneCall.callAudio ? islandWin.wash(0.26) : islandWin.wash(0.12)
                        Accessible.role: Accessible.Button
                        Accessible.name: PhoneCall.callAudio ? qsTr("Back to phone") : qsTr("On PC")
                        Icon {
                            anchors.centerIn: parent
                            width: 14; height: 14
                            path: Icons.desktop
                            color: islandWin.ink
                        }
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: PhoneCall.moveCallAudio(!PhoneCall.callAudio) }
                        IslandTipHover {
                            island: islandWin
                            tip: PhoneCall.callAudio ? qsTr("Back to phone") : qsTr("On PC")
                            owner: callAudioButton
                        }
                    }

                    Rectangle {
                        id: endCallButton
                        visible: islandWin.mode === "call" && PhoneCall.canEnd
                        width: 32; height: 32; radius: 16
                        color: "#EF4444"
                        Icon {
                            anchors.centerIn: parent
                            width: 15; height: 15
                            path: Icons.callEnd
                            color: "#FFFFFF"
                        }
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: PhoneCall.end() }
                        IslandTipHover { island: islandWin; tip: qsTr("Hang up"); owner: endCallButton }
                    }

                    // ---- Media controls: Switch source, Previous, Play/Pause, Next ----
                    Rectangle {
                        id: switchSourceButton
                        visible: islandWin.mode === "media" && islandWin.mediaSources > 1
                        width: 28; height: 28; radius: 14
                        anchors.verticalCenter: parent.verticalCenter
                        color: expSwitchHover.hovered ? islandWin.wash(0.20) : "transparent"
                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("Switch to the next player")
                        Icon {
                            anchors.centerIn: parent
                            width: 13; height: 13
                            path: islandWin.mediaDeviceId === "local" ? Icons.phone : Icons.desktop
                            color: islandWin.wash(0.85)
                        }
                        HoverHandler { id: expSwitchHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler { onTapped: islandWin.nextMediaSource() }
                        IslandTipHover {
                            island: islandWin
                            tip: qsTr("Switch to the next player")
                            owner: switchSourceButton
                        }
                    }

                    Rectangle {
                        id: previousButton
                        visible: islandWin.mode === "media" && islandWin.mediaCanPrev
                        width: 28; height: 28; radius: 14
                        anchors.verticalCenter: parent.verticalCenter
                        color: expPrevHover.hovered ? islandWin.wash(0.20) : islandWin.wash(0.08)
                        Icon {
                            anchors.centerIn: parent
                            width: 13; height: 13
                            path: Icons.skipPrevious
                            color: islandWin.ink
                        }
                        HoverHandler { id: expPrevHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: MediaList.command(islandWin.mediaDeviceId, islandWin.mediaPlayerId, "previous", 0)
                        }
                        IslandTipHover { island: islandWin; tip: qsTr("Previous track"); owner: previousButton }
                    }

                    Rectangle {
                        id: playButton
                        visible: islandWin.mode === "media"
                        width: 34; height: 34; radius: 17
                        anchors.verticalCenter: parent.verticalCenter
                        color: Theme.primary
                        Icon {
                            anchors.centerIn: parent
                            width: 15; height: 15
                            path: islandWin.mediaPlaying ? Icons.pause : Icons.play
                            color: Theme.primaryContent
                        }
                        HoverHandler { cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: MediaList.command(
                                islandWin.mediaDeviceId,
                                islandWin.mediaPlayerId,
                                islandWin.mediaPlaying ? "pause" : "play",
                                0
                            )
                        }
                        IslandTipHover {
                            island: islandWin
                            tip: islandWin.mediaPlaying ? qsTr("Pause") : qsTr("Play")
                            owner: playButton
                        }
                    }

                    Rectangle {
                        id: nextButton
                        visible: islandWin.mode === "media" && islandWin.mediaCanNext
                        width: 28; height: 28; radius: 14
                        anchors.verticalCenter: parent.verticalCenter
                        color: expNextHover.hovered ? islandWin.wash(0.20) : islandWin.wash(0.08)
                        Icon {
                            anchors.centerIn: parent
                            width: 13; height: 13
                            path: Icons.skipNext
                            color: islandWin.ink
                        }
                        HoverHandler { id: expNextHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: MediaList.command(islandWin.mediaDeviceId, islandWin.mediaPlayerId, "next", 0)
                        }
                        IslandTipHover { island: islandWin; tip: qsTr("Next track"); owner: nextButton }
                    }

                    // ---- Transfer cancel button ----
                    Rectangle {
                        id: cancelTransferButton
                        visible: islandWin.mode === "transfer" && !islandWin.transferJustFinished && islandWin.activeTransferId.length > 0
                        width: 28; height: 28; radius: 14
                        anchors.verticalCenter: parent.verticalCenter
                        color: expCancelHover.hovered ? Qt.rgba(1, 0.3, 0.3, 0.28) : islandWin.wash(0.10)
                        Icon {
                            anchors.centerIn: parent
                            width: 13; height: 13
                            path: Icons.close
                            color: islandWin.ink
                        }
                        HoverHandler { id: expCancelHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler {
                            onTapped: TransferList.cancel(islandWin.activeTransferId)
                        }
                        IslandTipHover {
                            island: islandWin
                            tip: qsTr("Cancel transfer")
                            owner: cancelTransferButton
                        }
                    }
                }
            }
        }

        // The stretched part of the Island: notifications, media source and
        // lyrics live here, inside the same window and the same surface.
        // Side docks: the whole card slides in from the chosen edge.
        Rectangle {
            id: sideSurface
            visible: islandWin.sideDock && islandWin.panelShown
            x: 0
            y: 0
            width: 340
            height: card.height
            radius: 22
            color: islandWin.islandSurfaceColor
            border.width: 1
            border.color: islandWin.mode === "call"
                ? Qt.rgba(0.29, 0.87, 0.50, 0.55)
                : islandWin.islandBorderColor
        }

        IslandCard {
            id: card
            island: islandWin
            x: 0
            y: islandWin.sideDock ? 0 : Math.max(0, pillBg.height - 6)
            width: parent.width
            height: islandWin.expandedCardHeight
            visible: islandWin.panelShown
            opacity: visible ? 1 : 0
            Behavior on opacity {
                enabled: !Theme.reduceMotion
                NumberAnimation { duration: 140; easing.type: Easing.OutCubic }
            }
        }

        // Side docks: a vertical rail at the edge. Hovering opens the card
        // inward; the rail itself disappears while the card is shown.
        Item {
            id: sideRail
            visible: islandWin.sideDock && !islandWin.panelShown
            width: islandWin.sideRailW
            height: islandWin.sideRailH
            x: 0
            y: 0

            HoverHandler {
                id: sideHover
                cursorShape: Qt.PointingHandCursor
                onHoveredChanged: if (!hovered && !islandWin.panelHovered)
                    islandWin.panelSuppressed = false
            }

            Rectangle {
                anchors.fill: parent
                radius: 26
                gradient: Gradient {
                    GradientStop { position: 0; color: islandWin.islandTopColor }
                    GradientStop { position: 1; color: islandWin.islandBottomColor }
                }
                border.width: 1
                border.color: islandWin.mode === "call"
                    ? Qt.rgba(0.29, 0.87, 0.50, 0.55)
                    : islandWin.islandBorderColor
            }

            Column {
                anchors.centerIn: parent
                spacing: 7
                width: parent.width

                Item {
                    width: 38
                    height: 38
                    anchors.horizontalCenter: parent.horizontalCenter

                    Rectangle {
                        anchors.fill: parent
                        radius: 11
                        visible: !(islandWin.mode === "media" && islandWin.mediaArt.length > 0)
                        color: islandWin.wash(0.10)
                        Icon {
                            anchors.centerIn: parent
                            width: 18
                            height: 18
                            path: islandWin.mode === "call" ? Icons.call
                                : islandWin.mode === "transfer" ? Icons.send
                                : islandWin.mode === "media" ? Icons.music
                                : Icons.island
                            color: islandWin.wash(0.72)
                        }
                    }
                    RoundedImage {
                        anchors.fill: parent
                        radius: 11
                        visible: islandWin.mode === "media" && islandWin.mediaArt.length > 0
                        source: islandWin.mediaArt
                        sourceSize: Qt.size(96, 96)
                    }
                }

                IconButton {
                    anchors.horizontalCenter: parent.horizontalCenter
                    visible: islandWin.mediaTitle.length > 0
                    iconPath: islandWin.mediaPlaying ? Icons.pause : Icons.play
                    iconSize: 18
                    iconColor: islandWin.wash(0.88)
                    label: islandWin.mediaPlaying ? qsTr("Pause") : qsTr("Play")
                    onClicked: MediaList.command(islandWin.mediaDeviceId, islandWin.mediaPlayerId,
                        islandWin.mediaPlaying ? "pause" : "play")
                }

                IconButton {
                    anchors.horizontalCenter: parent.horizontalCenter
                    visible: islandWin.mediaTitle.length > 0 && islandWin.mediaCanNext
                    iconPath: Icons.skipNext
                    iconSize: 17
                    iconColor: islandWin.wash(0.72)
                    label: qsTr("Next track")
                    onClicked: MediaList.command(islandWin.mediaDeviceId, islandWin.mediaPlayerId, "next")
                }

                Rectangle {
                    anchors.horizontalCenter: parent.horizontalCenter
                    visible: islandWin.unreadCount > 0
                    width: 24
                    height: 24
                    radius: 12
                    color: "#EF4444"
                    Txt {
                        anchors.centerIn: parent
                        text: islandWin.unreadCount > 99 ? "99+" : String(islandWin.unreadCount)
                        role: "mono"
                        size: 9.5
                        weight: Font.Bold
                        color: "#FFFFFF"
                    }
                }
            }

            // A thin progress line on the inner edge.
            Rectangle {
                visible: islandWin.mediaDuration > 0
                x: islandWin.sideRight ? 0 : width - 3
                y: 22
                width: 3
                height: parent.height - 44
                radius: 1.5
                color: islandWin.wash(0.14)

                Rectangle {
                    anchors.bottom: parent.bottom
                    width: parent.width
                    radius: parent.radius
                    height: parent.height * Math.max(0, Math.min(1,
                        islandWin.mediaPositionNow / islandWin.mediaDuration))
                    color: Theme.primary
                }
            }
        }

        // Follows the pointer while it is over one of the Island's buttons.
        Rectangle {
            id: islandTipBubble
            z: 5000
            visible: islandWin.tipText.length > 0
            width: Math.min(tipLabel.implicitWidth + 16, 240)
            height: tipLabel.implicitHeight + 8
            radius: 7
            color: islandWin.light ? "#FFFFFF" : "#1F1F22"
            border.width: 1
            border.color: islandWin.light ? "#D8D8D8" : "#3A3A40"
            x: Math.max(6, Math.min(root.width - width - 6, islandWin.tipX + 14))
            y: islandWin.tipY + height + 20 <= root.height - 6
                ? islandWin.tipY + 18
                : Math.max(6, islandWin.tipY - height - 10)

            Txt {
                id: tipLabel
                anchors.centerIn: parent
                width: Math.min(implicitWidth, 224)
                text: islandWin.tipText
                role: "caption"
                size: 10
                elide: Text.ElideRight
                color: islandWin.light ? "#1A1A1A" : "#FFFFFF"
            }
        }
    }

    // Click-through makes the Island itself ignore clicks. This small,
    // separate button stays clickable so the mode can always be turned off
    // from the Island, not only from the main Settings page.
    Window {
        id: unlockWindow
        title: "Island unlock"
        transientParent: islandWin
        flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.WindowDoesNotAcceptFocus
        color: "transparent"
        width: Math.round(34 * islandWin.islandScale)
        height: width
        x: islandWin.x + islandWin.width - width - 4
        y: islandWin.y + 4
        visible: islandWin.clickThroughActive && islandWin.expanded && islandWin.visible

        Rectangle {
            anchors.fill: parent
            radius: width / 2
            color: islandWin.light ? "#F7F7F7" : "#1F1F22"
            border.width: 1
            border.color: Theme.primary

            Icon {
                anchors.centerIn: parent
                width: Math.round(15 * islandWin.islandScale)
                height: width
                path: Icons.lock
                color: islandWin.light ? "#1A1A1A" : "#FFFFFF"
            }
            MouseArea {
                anchors.fill: parent
                hoverEnabled: true
                cursorShape: Qt.PointingHandCursor
                onContainsMouseChanged: {
                    if (containsMouse) {
                        const x = (unlockWindow.x - islandWin.x + width / 2)
                            / islandWin.islandScale
                        const y = (unlockWindow.y - islandWin.y + height / 2)
                            / islandWin.islandScale
                        islandWin.showTip(qsTr("Turn off click-through"), root, Qt.point(x, y))
                    } else {
                        islandWin.hideTip(root)
                    }
                }
                onClicked: islandWin.turnOffClickThrough()
            }
            Accessible.role: Accessible.Button
            Accessible.name: qsTr("Turn off click-through")
        }
    }
}
