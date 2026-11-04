// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtCore
import QtMultimedia
import app.nectarlink

// Internet radio drawn as an analogue tuner: the frequency, an FM/AM band, a
// curved scale with a needle and a dial you turn. Stations come from the public
// Radio Browser directory. They have no broadcast frequency, so a station whose
// name carries one (like "KQED 88.5 FM") sits there, and the rest get their own
// slot on the dial. FM holds music stations, AM news, talk and sport.
// Streams play over plain HTTP: the bundled FFmpeg has no TLS.
Rectangle {
    id: radio
    implicitHeight: body.height + 24
    radius: Theme.radiusLg
    color: Qt.rgba(Theme.surface.r, Theme.surface.g, Theme.surface.b, Theme.dark ? 0.42 : 0.60)
    border.width: 1
    border.color: Qt.rgba(1, 1, 1, Theme.dark ? 0.10 : 0.40)
    clip: true

    // The chosen design's colours.
    readonly property color accent: Theme.primary
    readonly property color needle: Theme.primary
    readonly property color ink: Theme.surfaceContent
    readonly property bool fm: prefs.band !== "AM"
    readonly property real bandMin: fm ? 87.5 : 530
    readonly property real bandMax: fm ? 108.0 : 1700
    // Pixels per band unit on the scale: per MHz for FM, per kHz for AM.
    readonly property real ppu: fm ? 64 : 0.64

    Settings {
        id: prefs
        location: StandardPaths.writableLocation(StandardPaths.AppConfigLocation) + "/home-widgets.ini"
        category: "radio"
        property string country: ""
        property string presets: "[]"
        property string last: ""
        property real volume: 0.6
        property string band: "FM"
    }

    // The country typed under the chevron, else this PC's time zone's.
    readonly property string detected: AppController.systemCountry()
    readonly property string country: prefs.country.trim().length === 2 ? prefs.country.trim().toUpperCase() : detected
    property var all: []
    readonly property var stations: placed(all.filter(s => fm ? !s.talk : s.talk))
    property real freq: bandMin
    property var current: null
    property bool on: false
    property string message: ""
    property int failures: 0
    property bool settingsOpen: false
    readonly property bool playing: on && player.mediaStatus === MediaPlayer.BufferedMedia
    readonly property var presets: {
        try {
            const p = JSON.parse(prefs.presets)
            return Array.isArray(p) ? p : []
        } catch (e) {
            return []
        }
    }

    // ---- Stations on the dial ----
    function parsedFreq(name, isFm) {
        const re = isFm ? /(?:^|[^\d])(8[89]|9\d|10[0-8])[.,](\d)(?!\d)/ : /(?:^|[^\d])(5[3-9]\d|[6-9]\d\d|1[0-6]\d\d|1700)(?:\s*(?:AM|kHz))/i
        const m = name.match(re)
        if (!m)
            return -1
        return isFm ? parseFloat(m[1] + "." + m[2]) : parseInt(m[1])
    }
    function placed(list) {
        const isFm = fm
        const step = isFm ? 0.2 : 10
        const lo = isFm ? 87.9 : 540
        const hi = isFm ? 107.9 : 1690
        const taken = {}
        const key = f => isFm ? Math.round(f * 10) : Math.round(f)
        const out = []
        const rest = []
        for (const s of list) {
            const f = parsedFreq(s.name, isFm)
            if (f > 0 && !taken[key(f)]) {
                taken[key(f)] = true
                out.push(Object.assign({}, s, { f: f }))
            } else {
                rest.push(s)
            }
        }
        // The rest spread evenly over the band, skipping slots already taken.
        const gap = rest.length > 0 ? Math.max(step, (hi - lo) / rest.length) : step
        let f = lo
        for (const s of rest) {
            let g = Math.round(f / step) * step
            while (taken[key(g)] && g < hi)
                g += step
            taken[key(g)] = true
            out.push(Object.assign({}, s, { f: isFm ? Math.round(g * 10) / 10 : g }))
            f += gap
        }
        return out.sort((a, b) => a.f - b.f)
    }
    function nearest(f) {
        let best = null
        for (const s of stations)
            if (!best || Math.abs(s.f - f) < Math.abs(best.f - f))
                best = s
        return best
    }

    // ---- Directory ----
    function getJson(url, ok, fail) {
        const xhr = new XMLHttpRequest()
        xhr.onreadystatechange = function() {
            if (xhr.readyState !== XMLHttpRequest.DONE)
                return
            let data = null
            if (xhr.status >= 200 && xhr.status < 300) {
                try { data = JSON.parse(xhr.responseText) } catch (e) {}
            }
            if (data !== null)
                ok(data)
            else
                fail()
        }
        xhr.open("GET", url)
        xhr.setRequestHeader("User-Agent", "NectarLink-Radio")
        xhr.send()
    }
    function load() {
        const cc = radio.country
        const path = cc.length === 2 ? "bycountrycodeexact/" + cc + "?" : "topvote?"
        message = qsTr("Finding stations…")
        fetchFrom(0, path + "order=votes&reverse=true&hidebroken=true&limit=80")
    }
    // Radio Browser runs several mirrors; any one of them can be down.
    readonly property var mirrors: ["de2", "de1", "fi1", "nl1"]
    function fetchFrom(i, query) {
        if (i >= mirrors.length) {
            message = qsTr("Can't reach the station list")
            return
        }
        getJson("https://" + mirrors[i] + ".api.radio-browser.info/json/stations/" + query,
                (list) => {
                    all = list.filter(s => (s.url_resolved || s.url))
                        .map(s => ({ name: (s.name || "").trim(), url: s.url_resolved || s.url, alt: s.url || "",
                                     country: s.countrycode || "", codec: s.codec || "", bitrate: s.bitrate || 0,
                                     talk: /news|talk|sport|politic|info/i.test(s.tags || "") }))
                    message = all.length === 0 ? qsTr("No stations for %1").arg(radio.country) : ""
                    let saved = null
                    try { saved = JSON.parse(prefs.last) } catch (e) {}
                    const back = saved && stations.find(s => s.url === saved.url)
                    select(back || stations[0] || null, false)
                    if (playWhenLoaded && current) {
                        playWhenLoaded = false
                        toggle()
                    }
                },
                () => fetchFrom(i + 1, query))
    }

    // ---- Playing ----
    function streamUrl(s) {
        if (!s.url.startsWith("https://"))
            return s.url
        if (s.alt.startsWith("http://"))
            return s.alt
        return "http://" + s.url.slice(8)
    }
    function play() {
        if (!current)
            return
        prefs.last = JSON.stringify(current)
        player.stop()
        player.source = streamUrl(current)
        player.play()
        openTimeout.restart()
    }
    function select(station, andPlay) {
        current = station
        if (station)
            freq = station.f
        if (station && on && andPlay !== false)
            play()
    }
    function step(dir) {
        ensureLoaded()
        if (stations.length === 0)
            return
        const i = current ? stations.findIndex(s => s.url === current.url) : -1
        select(stations[(i + dir + stations.length) % stations.length])
    }
    function toggle() {
        if (!loaded) {
            playWhenLoaded = true
            ensureLoaded()
            return
        }
        if (!current)
            return
        on = !on
        failures = 0
        if (on) {
            play()
        } else {
            openTimeout.stop()
            player.stop()
            player.source = ""
            message = ""
        }
    }
    function setBand(b) {
        if (prefs.band === b)
            return
        prefs.band = b
        ensureLoaded()
        select(stations[0] || null)
    }
    // A station that won't open: say so and move on, but don't scan forever.
    function failed() {
        if (!on)
            return
        openTimeout.stop()
        failures++
        if (failures >= 5) {
            message = qsTr("Stations aren't playing. Check the connection.")
            toggle()
            return
        }
        message = qsTr("%1 won't play, trying the next").arg(current ? current.name : "")
        skipTimer.start()
    }

    // Nothing is fetched until the radio is used: play, tune or its settings.
    property bool loaded: false
    property bool playWhenLoaded: false
    function ensureLoaded() {
        if (loaded)
            return
        loaded = true
        load()
    }

    MediaPlayer {
        id: player
        audioOutput: AudioOutput { volume: prefs.volume }
        onErrorOccurred: radio.failed()
        onMediaStatusChanged: {
            if (mediaStatus === MediaPlayer.BufferedMedia) {
                openTimeout.stop()
                radio.failures = 0
                radio.message = ""
            } else if (mediaStatus === MediaPlayer.InvalidMedia) {
                radio.failed()
            }
        }
    }
    Timer { id: openTimeout; interval: 15000; onTriggered: radio.failed() }
    Timer { id: skipTimer; interval: 1200; onTriggered: radio.step(1) }

    Column {
        id: body
        x: 16
        y: 10
        width: parent.width - 32
        spacing: 4

        // ---- Header ----
        Item {
            width: parent.width
            height: 30
            Txt {
                anchors.verticalCenter: parent.verticalCenter
                role: "headline"
                size: 22
                weight: 700
                color: Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.38)
                text: qsTr("Radio")
            }
            Rectangle {
                id: chevron
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                width: 30
                height: 30
                radius: 15
                color: chevronMouse.containsMouse ? Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.08) : "transparent"
                Accessible.role: Accessible.Button
                Accessible.name: radio.settingsOpen ? qsTr("Hide radio settings") : qsTr("Radio settings")
                Icon {
                    anchors.centerIn: parent
                    width: 16
                    height: 16
                    path: radio.settingsOpen ? Icons.chevronUp : Icons.chevronDown
                    color: radio.ink
                }
                MouseArea {
                    id: chevronMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        radio.settingsOpen = !radio.settingsOpen
                        radio.ensureLoaded()
                    }
                }
            }
        }

        // ---- Station and frequency ----
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            role: "body"
            size: 14
            weight: 600
            color: radio.ink
            elide: Text.ElideRight
            maximumLineCount: 1
            text: radio.message.length > 0 ? radio.message
                  : radio.current ? radio.current.name
                  : radio.loaded ? qsTr("No station") : qsTr("Press play to tune in")
        }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            readonly property string digits: radio.fm ? radio.freq.toFixed(1) : String(Math.round(radio.freq))
            readonly property int width0: radio.fm ? 5 : 4
            readonly property string pad: "0".repeat(Math.max(0, width0 - digits.length))
            Txt {
                role: "displayLarge"
                size: 48
                weight: 300
                color: Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.30)
                text: parent.pad
            }
            Txt {
                role: "displayLarge"
                size: 48
                weight: 700
                color: radio.ink
                text: parent.digits
            }
        }
        Row {
            anchors.horizontalCenter: parent.horizontalCenter
            spacing: 22
            Repeater {
                model: ["FM", "AM"]
                delegate: Item {
                    required property string modelData
                    readonly property bool active: prefs.band === modelData || (modelData === "FM" && prefs.band !== "AM")
                    width: bandText.implicitWidth
                    height: bandText.implicitHeight + 10
                    Accessible.role: Accessible.Button
                    Accessible.name: modelData
                    Txt {
                        id: bandText
                        role: "title"
                        size: 15
                        weight: 600
                        color: parent.active ? radio.accent : Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.45)
                        text: parent.modelData
                    }
                    Rectangle {
                        anchors.horizontalCenter: parent.horizontalCenter
                        anchors.bottom: parent.bottom
                        visible: parent.active
                        width: 5
                        height: 5
                        radius: 2.5
                        color: radio.accent
                    }
                    MouseArea {
                        anchors.fill: parent
                        anchors.margins: -6
                        cursorShape: Qt.PointingHandCursor
                        onClicked: radio.setBand(parent.modelData)
                    }
                }
            }
        }

        // ---- Scale, needle and dial: drag or scroll to tune ----
        Item {
            id: tuner
            width: parent.width
            height: 140
            property real dragFrom: 0
            property real freqFrom: 0

            Canvas {
                id: scale
                anchors.fill: parent
                property real f: radio.freq
                property var marks: radio.stations
                property color tint: radio.ink
                property color mark: radio.accent
                onTintChanged: requestPaint()
                onMarkChanged: requestPaint()
                onFChanged: requestPaint()
                onMarksChanged: requestPaint()
                onWidthChanged: requestPaint()
                onPaint: {
                    const ctx = getContext("2d")
                    ctx.reset()
                    const cx = width / 2, base = 40, ppu = radio.ppu
                    const minor = radio.fm ? 0.1 : 10, label = radio.fm ? 1 : 100
                    const arc = t => base + t * t * 22
                    const fade = t => Math.max(0, 1 - Math.abs(t) * 0.85)
                    const from = Math.max(radio.bandMin, f - cx / ppu), to = Math.min(radio.bandMax, f + cx / ppu)
                    const first = Math.ceil(from / minor) * minor
                    ctx.textAlign = "center"
                    for (let v = first; v <= to + 1e-6; v += minor) {
                        const x = cx + (v - f) * ppu
                        const t = (x - cx) / cx
                        const y = arc(t)
                        const isLabel = Math.abs(v / label - Math.round(v / label)) < 1e-6
                        const isHalf = Math.abs((v * 2) / label - Math.round((v * 2) / label)) < 1e-6
                        const h = isLabel ? 14 : isHalf ? 9 : 5
                        ctx.strokeStyle = Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.45 * fade(t))
                        ctx.lineWidth = isLabel ? 1.4 : 1
                        ctx.beginPath()
                        ctx.moveTo(x, y)
                        ctx.lineTo(x, y + h)
                        ctx.stroke()
                        if (isLabel) {
                            ctx.fillStyle = Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.85 * fade(t))
                            ctx.font = (Math.abs(t) < 0.25 ? "bold " : "") + "11px sans-serif"
                            ctx.fillText(radio.fm ? Math.round(v) : Math.round(v), x, y - 6 - Math.abs(t) * 4)
                        }
                    }
                    // Where stations are.
                    for (const s of marks) {
                        const x = cx + (s.f - f) * ppu
                        if (x < 0 || x > width)
                            continue
                        const t = (x - cx) / cx
                        ctx.fillStyle = radio.current && s.url === radio.current.url ? radio.accent : Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.55 * fade(t))
                        ctx.beginPath()
                        ctx.arc(x, arc(t) + 20, 1.8, 0, 2 * Math.PI)
                        ctx.fill()
                    }

                    // The dial: a wide arc turning with the frequency.
                    const R = width * 0.95, dcx = cx, dcy = height + R * 0.72
                    const ring = (r, w, c) => {
                        ctx.beginPath()
                        ctx.arc(dcx, dcy, r, Math.PI * 1.15, Math.PI * 1.85)
                        ctx.lineWidth = w
                        ctx.strokeStyle = c
                        ctx.stroke()
                    }
                    ring(R - 10, 22, Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.07))
                    ring(R - 22, 1, Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.10))
                    ring(R + 2, 1, Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.12))
                    const turn = (f - radio.bandMin) * (radio.fm ? 0.35 : 0.004)
                    ctx.strokeStyle = Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.22)
                    ctx.lineWidth = 1
                    for (let a = 0; a < Math.PI * 2; a += Math.PI / 150) {
                        const ang = a + turn
                        if (ang % (Math.PI * 2) < Math.PI * 1.15 || ang % (Math.PI * 2) > Math.PI * 1.85)
                            continue
                        ctx.beginPath()
                        ctx.moveTo(dcx + Math.cos(ang) * (R - 20), dcy + Math.sin(ang) * (R - 20))
                        ctx.lineTo(dcx + Math.cos(ang) * (R - 2), dcy + Math.sin(ang) * (R - 2))
                        ctx.stroke()
                    }
                    // Hub.
                    ctx.beginPath()
                    ctx.arc(dcx, height - 14, 9, 0, 2 * Math.PI)
                    ctx.strokeStyle = Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.12)
                    ctx.stroke()
                }
            }

            // Needle.
            Canvas {
                id: marker
                x: (parent.width - width) / 2
                y: 16
                width: 12
                height: 10
                property color mark: radio.accent
                onMarkChanged: requestPaint()
                onPaint: {
                    const ctx = getContext("2d")
                    ctx.reset()
                    ctx.fillStyle = radio.accent
                    ctx.beginPath()
                    ctx.moveTo(0, 0)
                    ctx.lineTo(width, 0)
                    ctx.lineTo(width / 2, height)
                    ctx.closePath()
                    ctx.fill()
                }
            }
            Rectangle {
                x: Math.round(parent.width / 2 - 1)
                y: 30
                width: 2
                height: 40
                color: radio.needle
            }
            Rectangle {
                x: Math.round(parent.width / 2 - 9)
                y: 46
                width: 18
                height: 30
                radius: 4
                color: Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.05)
                border.width: 1
                border.color: Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.22)
            }

            MouseArea {
                anchors.fill: parent
                cursorShape: pressed ? Qt.ClosedHandCursor : Qt.OpenHandCursor
                Accessible.role: Accessible.Slider
                Accessible.name: qsTr("Tuning dial")
                onPressed: (mouse) => {
                    radio.ensureLoaded()
                    tuner.dragFrom = mouse.x
                    tuner.freqFrom = radio.freq
                }
                onPositionChanged: (mouse) => {
                    const f = tuner.freqFrom - (mouse.x - tuner.dragFrom) / radio.ppu
                    radio.freq = Math.max(radio.bandMin, Math.min(radio.bandMax, f))
                }
                onReleased: {
                    const s = radio.nearest(radio.freq)
                    if (s)
                        radio.select(s)
                }
                onWheel: (wheel) => radio.step(wheel.angleDelta.y > 0 ? -1 : 1)
            }
            Behavior on implicitWidth { enabled: false; NumberAnimation {} }
        }

        // ---- Previous, play/pause, next ----
        Item {
            width: parent.width
            height: 56
            Repeater {
                model: [
                    { icon: Icons.skipPrevious, act: -1, x: 0.14 },
                    { icon: radio.on ? Icons.pause : Icons.play, act: 0, x: 0.5 },
                    { icon: Icons.skipNext, act: 1, x: 0.86 }
                ]
                delegate: Rectangle {
                    id: ctl
                    required property var modelData
                    readonly property bool big: modelData.act === 0
                    readonly property bool usable: big || radio.stations.length > 0 || !radio.loaded
                    width: big ? 54 : 42
                    height: width
                    radius: width / 2
                    x: parent.width * modelData.x - width / 2
                    anchors.verticalCenter: parent.verticalCenter
                    color: big ? Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, ctlMouse.pressed ? 0.12 : 0.06) : "transparent"
                    border.width: big ? 1 : 0
                    border.color: radio.playing ? radio.accent : Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.16)
                    opacity: usable ? 1 : 0.4
                    Accessible.role: Accessible.Button
                    Accessible.name: modelData.act === -1 ? qsTr("Previous station")
                                     : modelData.act === 1 ? qsTr("Next station")
                                     : radio.on ? qsTr("Pause the radio") : qsTr("Play the radio")
                    Icon {
                        anchors.centerIn: parent
                        width: ctl.big ? 26 : 22
                        height: width
                        path: ctl.modelData.icon
                        color: radio.ink
                        opacity: ctl.big ? 1 : 0.6
                    }
                    MouseArea {
                        id: ctlMouse
                        anchors.fill: parent
                        anchors.margins: -6
                        cursorShape: ctl.usable ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: {
                            if (!ctl.usable)
                                return
                            if (ctl.modelData.act === 0)
                                radio.toggle()
                            else
                                radio.step(ctl.modelData.act)
                        }
                    }
                }
            }
        }

        // ---- Settings (under the chevron): volume, presets, country ----
        Column {
            width: parent.width
            spacing: 10
            visible: radio.settingsOpen
            topPadding: 4
            bottomPadding: 4

            Row {
                width: parent.width
                spacing: 10
                Icon {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 16
                    height: 16
                    path: Icons.volumeDown
                    color: Theme.surfaceContentVariant
                }
                ValueSlider {
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width - 52
                    from: 0
                    to: 100
                    step: 5
                    value: Math.round(prefs.volume * 100)
                    label: qsTr("Radio volume")
                    onMoved: (v) => prefs.volume = v / 100
                }
                Icon {
                    anchors.verticalCenter: parent.verticalCenter
                    width: 16
                    height: 16
                    path: Icons.volumeUp
                    color: Theme.surfaceContentVariant
                }
            }

            // Presets: tap to recall, hold to store.
            Row {
                id: presetRow
                width: parent.width
                spacing: 6
                readonly property real cell: (width - spacing * 5) / 6
                Repeater {
                    model: 6
                    delegate: Rectangle {
                        id: preset
                        required property int index
                        readonly property var saved: radio.presets[index] || null
                        readonly property bool active: !!saved && !!radio.current && saved.url === radio.current.url
                        width: presetRow.cell
                        height: 40
                        radius: 8
                        color: presetMouse.pressed ? Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.12) : Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.05)
                        border.width: 1
                        border.color: active ? radio.accent : Qt.rgba(radio.ink.r, radio.ink.g, radio.ink.b, 0.10)
                        Accessible.role: Accessible.Button
                        Accessible.name: saved ? qsTr("Preset %1: %2").arg(index + 1).arg(saved.name)
                                               : qsTr("Preset %1, empty. Hold to store the current station").arg(index + 1)
                        Column {
                            anchors.centerIn: parent
                            width: parent.width - 6
                            Txt {
                                anchors.horizontalCenter: parent.horizontalCenter
                                role: "label"
                                weight: 700
                                color: preset.active ? radio.accent : radio.ink
                                text: String(preset.index + 1)
                            }
                            Txt {
                                width: parent.width
                                horizontalAlignment: Text.AlignHCenter
                                role: "caption"
                                size: 9
                                color: Theme.surfaceContentVariant
                                elide: Text.ElideRight
                                maximumLineCount: 1
                                text: preset.saved ? preset.saved.name : "—"
                            }
                        }
                        MouseArea {
                            id: presetMouse
                            anchors.fill: parent
                            cursorShape: Qt.PointingHandCursor
                            pressAndHoldInterval: 700
                            onPressAndHold: {
                                if (!radio.current)
                                    return
                                const p = radio.presets.slice()
                                while (p.length < 6)
                                    p.push(null)
                                p[preset.index] = radio.current
                                prefs.presets = JSON.stringify(p)
                                radio.message = qsTr("Stored on %1").arg(preset.index + 1)
                            }
                            onClicked: {
                                if (!preset.saved) {
                                    radio.message = qsTr("Hold %1 to store this station").arg(preset.index + 1)
                                    return
                                }
                                if (preset.saved.talk !== undefined)
                                    prefs.band = preset.saved.talk ? "AM" : "FM"
                                radio.current = preset.saved
                                radio.freq = preset.saved.f || radio.freq
                                if (radio.on)
                                    radio.play()
                                else
                                    radio.toggle()
                            }
                        }
                    }
                }
            }

            Rectangle {
                width: parent.width
                height: 30
                radius: 15
                color: Theme.surfaceContainerHigh
                border.width: countryInput.activeFocus ? 1 : 0
                border.color: radio.accent
                TextInput {
                    id: countryInput
                    anchors.fill: parent
                    anchors.leftMargin: 12
                    anchors.rightMargin: 12
                    verticalAlignment: TextInput.AlignVCenter
                    color: radio.ink
                    font.pixelSize: 12
                    maximumLength: 2
                    validator: RegularExpressionValidator { regularExpression: /[A-Za-z]{0,2}|\*/ }
                    selectByMouse: true
                    Accessible.name: qsTr("Country code for stations")
                    onAccepted: {
                        prefs.country = text.trim() === "*" ? "" : text.trim().toUpperCase()
                        text = ""
                        focus = false
                        radio.load()
                    }
                }
                Txt {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.leftMargin: 12
                    anchors.rightMargin: 12
                    anchors.verticalCenter: parent.verticalCenter
                    visible: countryInput.text.length === 0 && !countryInput.activeFocus
                    role: "caption"
                    color: Theme.surfaceContentVariant
                    elide: Text.ElideRight
                    text: prefs.country.trim().length === 2
                          ? qsTr("Stations from %1 · type a code to change, or * for this PC's").arg(radio.country)
                          : radio.detected.length === 2
                            ? qsTr("Stations from %1 (this PC's time zone) · type a code to change").arg(radio.detected)
                            : qsTr("Top stations worldwide · type a country code, e.g. EG")
                }
            }
            Txt {
                width: parent.width
                role: "caption"
                color: Theme.surfaceContentVariant
                wrapMode: Text.WordWrap
                text: qsTr("Internet stations have no broadcast frequency: those that name one sit there, the rest get their own place on the dial.")
            }
        }
    }
}
