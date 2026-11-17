// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Settings › Install on a phone: three steps (USB tools, the phone, the app),
// each checked live while this pane exists. It's loaded only while its
// section is open, so nothing here runs otherwise (see PhoneInstall.watch).
Column {
    id: pane
    spacing: Theme.gutter

    readonly property var s: {
        try { return JSON.parse(PhoneInstall.state) } catch (e) { return ({}) }
    }
    readonly property var tools: s.tools || ({ kind: "unknown" })
    readonly property var phones: s.phones || []
    readonly property var job: s.job || ({ kind: "idle" })
    readonly property bool busy: ["downloading", "sending", "installing", "uninstalling"].indexOf(job.kind) >= 0
    readonly property bool toolsReady: tools.kind === "ready" && s.server === true
    readonly property var readyPhones: phones.filter(p => p.state === "ready")
    readonly property bool phoneReady: readyPhones.length > 0
    readonly property bool installed: job.kind === "done"
        || readyPhones.some(p => p.installed && !p.outdated)
    readonly property bool motion: !Theme.reduceMotion
    // How long it's been waiting for a phone, for the "not showing up?" tips.
    property int waitedSeconds: 0

    Component.onCompleted: PhoneInstall.watch(true)
    Component.onDestruction: PhoneInstall.watch(false)

    Timer {
        interval: 1000
        repeat: true
        running: pane.toolsReady && pane.phones.length === 0
        onRunningChanged: if (!running) pane.waitedSeconds = 0
        onTriggered: pane.waitedSeconds++
    }

    function mb(bytes) { return (bytes / 1048576).toFixed(1) }
    function percent(done, total) { return total > 0 ? Math.min(100, Math.round(done * 100 / total)) : 0 }

    // ---- The picture: a PC and a phone, with what goes between them ----
    Card {
        width: parent.width
        padding: 22
        Column {
            width: parent.width
            spacing: 16

            Item {
                width: parent.width
                height: 76

                Rectangle {
                    id: pcBubble
                    width: 64; height: 64
                    radius: Theme.radiusLg
                    anchors.verticalCenter: parent.verticalCenter
                    x: parent.width / 2 - 140
                    color: Theme.secondaryContainer
                    Icon {
                        anchors.centerIn: parent
                        width: 30; height: 30
                        path: Icons.laptop
                        color: Theme.secondaryContainerContent
                    }
                }
                Rectangle {
                    id: phoneBubble
                    width: 64; height: 64
                    radius: Theme.radiusLg
                    anchors.verticalCenter: parent.verticalCenter
                    x: parent.width / 2 + 76
                    color: pane.installed ? Theme.primary : Theme.secondaryContainer
                    Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }
                    Icon {
                        anchors.centerIn: parent
                        width: 30; height: 30
                        path: pane.installed ? Icons.check : Icons.phone
                        color: pane.installed ? Theme.primaryContent : Theme.secondaryContainerContent
                    }
                    // A gentle pulse while it waits for the phone.
                    Rectangle {
                        anchors.centerIn: parent
                        width: parent.width; height: parent.height
                        radius: parent.radius
                        color: "transparent"
                        border.width: 2
                        border.color: Theme.primary
                        opacity: 0
                        visible: pulse.running
                        SequentialAnimation on opacity {
                            id: pulse
                            running: pane.motion && pane.toolsReady && !pane.phoneReady && pane.visible
                            loops: Animation.Infinite
                            NumberAnimation { from: 0.7; to: 0; duration: 1400; easing.type: Easing.OutCubic }
                            PauseAnimation { duration: 300 }
                        }
                        scale: 1 + (0.7 - opacity) * 0.35
                    }
                }
                // The cable: dotted, and a dot travelling along it while something is sent.
                Row {
                    id: cable
                    anchors.verticalCenter: parent.verticalCenter
                    x: pcBubble.x + pcBubble.width + 10
                    width: phoneBubble.x - x - 10
                    spacing: 6
                    Repeater {
                        model: Math.max(0, Math.floor(cable.width / 12))
                        Rectangle {
                            width: 6; height: 6
                            radius: 3
                            anchors.verticalCenter: parent.verticalCenter
                            color: pane.phoneReady ? Theme.primary : Theme.outlineVariant
                            Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }
                        }
                    }
                }
                Rectangle {
                    width: 12; height: 12
                    radius: 6
                    color: Theme.primary
                    anchors.verticalCenter: parent.verticalCenter
                    visible: travel.running
                    NumberAnimation on x {
                        id: travel
                        running: pane.motion && pane.busy && pane.visible
                        loops: Animation.Infinite
                        from: cable.x
                        to: cable.x + cable.width - 12
                        duration: 1100
                        easing.type: Easing.InOutSine
                    }
                }
            }

            Txt {
                width: parent.width
                role: "title"
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("Put NectarLink on a phone over USB")
            }
            Txt {
                width: parent.width
                role: "bodySmall"
                muted: true
                wrapMode: Text.WordWrap
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("For phones that block installing from a browser, or when a cable is simply easier. NectarLink checks what's needed and walks you through the rest.")
            }
        }
    }

    // ---- The three steps ----
    Card {
        width: parent.width
        padding: 20
        Column {
            width: parent.width
            spacing: 0

            Step {
                width: parent.width
                number: 1
                done: pane.toolsReady
                current: !pane.toolsReady
                title: qsTr("Android's USB tools")
                last: false

                Txt {
                    width: parent.width
                    visible: pane.s.usbOff === true
                    role: "bodySmall"
                    muted: true
                    wrapMode: Text.WordWrap
                    text: qsTr("USB is turned off for this copy of NectarLink.")
                }
                Txt {
                    width: parent.width
                    visible: pane.tools.kind === "ready"
                    role: "bodySmall"
                    muted: true
                    wrapMode: Text.WordWrap
                    text: !pane.s.server
                        ? qsTr("Starting them…")
                        : pane.tools.own
                            ? qsTr("Ready. NectarLink uses the copy it downloaded from Google.")
                            : qsTr("Ready. NectarLink uses the ones already on this PC.")
                }
                Column {
                    width: parent.width
                    spacing: 12
                    visible: pane.tools.kind === "missing" || pane.tools.kind === "failed"
                    Txt {
                        width: parent.width
                        role: "bodySmall"
                        muted: true
                        wrapMode: Text.WordWrap
                        text: qsTr("This PC doesn't have them yet. NectarLink can get them from Google: about 8 MB, kept in NectarLink's own folder, and also used to connect phones over a cable.")
                    }
                    Txt {
                        width: parent.width
                        visible: pane.tools.kind === "failed"
                        role: "bodySmall"
                        color: Theme.error
                        wrapMode: Text.WordWrap
                        text: qsTr("That didn't work: %1").arg(pane.tools.error || "")
                    }
                    Button {
                        text: pane.tools.kind === "failed" ? qsTr("Try again") : qsTr("Download from Google")
                        iconPath: Icons.download
                        onClicked: PhoneInstall.getTools()
                    }
                    Txt {
                        width: parent.width
                        role: "caption"
                        muted: true
                        wrapMode: Text.WordWrap
                        textFormat: Text.StyledText
                        linkColor: Theme.primary
                        text: qsTr("Downloading them means you accept Google's <a href=\"https://developer.android.com/studio/terms\">Android SDK terms</a>.")
                        onLinkActivated: (link) => Qt.openUrlExternally(link)
                        HoverHandler { cursorShape: parent.hoveredLink ? Qt.PointingHandCursor : Qt.ArrowCursor }
                    }
                }
                Column {
                    width: parent.width
                    spacing: 8
                    visible: pane.tools.kind === "getting"
                    Txt {
                        role: "bodySmall"
                        muted: true
                        text: pane.tools.total > 0
                            ? qsTr("Downloading from Google… %1 of %2 MB").arg(pane.mb(pane.tools.done || 0)).arg(pane.mb(pane.tools.total))
                            : qsTr("Asking Google which version is current…")
                    }
                    ProgressLine {
                        width: parent.width
                        value: pane.percent(pane.tools.done || 0, pane.tools.total || 0) / 100
                        indeterminate: !(pane.tools.total > 0)
                    }
                }
                Txt {
                    width: parent.width
                    visible: pane.tools.kind === "unavailable"
                    role: "bodySmall"
                    muted: true
                    wrapMode: Text.WordWrap
                    text: qsTr("Google doesn't make them for this kind of PC. Install adb from your distribution's packages (for example: sudo apt install adb), then come back here.")
                }
            }

            Step {
                width: parent.width
                number: 2
                done: pane.phoneReady
                current: pane.toolsReady && !pane.phoneReady
                enabled: pane.toolsReady
                title: pane.phoneReady
                    ? (pane.readyPhones.length === 1 ? qsTr("%1 is connected").arg(pane.readyPhones[0].name)
                                                     : qsTr("%1 phones are connected").arg(pane.readyPhones.length))
                    : qsTr("Connect the phone")
                last: false

                // Phones adb sees that aren't ready yet, each with what to do.
                Repeater {
                    model: pane.phones.filter(p => p.state !== "ready")
                    delegate: Hint {
                        required property var modelData
                        width: parent ? parent.width : 0
                        iconPath: modelData.state === "unauthorized" ? Icons.phone : Icons.warning
                        attention: modelData.state === "unauthorized"
                        text: modelData.state === "unauthorized"
                            ? qsTr("Unlock %1 and tap Allow on “Allow USB debugging?”. Tick “Always allow from this computer” so it doesn't ask again.").arg(modelData.name)
                            : modelData.state === "noPermissions"
                                ? qsTr("Linux needs Android's USB rules to reach %1. Install your distribution's android-udev-rules package (on Debian and Ubuntu: android-sdk-platform-tools-common), then plug the phone in again.").arg(modelData.name)
                                : qsTr("%1 isn't answering. Unplug it and plug it back in.").arg(modelData.name)
                    }
                }

                Column {
                    width: parent.width
                    spacing: 10
                    visible: pane.toolsReady && pane.phones.length === 0
                    Hint {
                        width: parent.width
                        visible: pane.s.driverMissing === true
                        iconPath: Icons.warning
                        attention: true
                        text: qsTr("Windows sees the phone but has no driver for its USB debugging. Install your phone maker's USB driver, then plug the phone in again.")
                        actionText: qsTr("Find the driver")
                        onAction: Qt.openUrlExternally("https://developer.android.com/studio/run/oem-usb")
                    }
                    Txt {
                        width: parent.width
                        role: "bodySmall"
                        muted: true
                        wrapMode: Text.WordWrap
                        text: qsTr("Plug the phone into this PC with a USB cable. If it's the first time, turn on USB debugging first:")
                    }
                    Checklist {
                        width: parent.width
                        items: [
                            qsTr("On the phone, open Settings › About phone and tap Build number seven times. On Xiaomi phones it's the OS version; on Samsung it's under Software information."),
                            qsTr("Go back to Settings, open Developer options (under System on many phones) and turn on USB debugging."),
                            qsTr("Use a cable that carries data, since some only charge. If the phone asks what the USB connection is for, pick File transfer.")
                        ]
                    }
                    Txt {
                        width: parent.width
                        visible: pane.waitedSeconds >= 20 && pane.s.driverMissing !== true
                        role: "caption"
                        muted: true
                        wrapMode: Text.WordWrap
                        text: qsTr("Still nothing? Try another USB port or cable, and check that USB debugging stayed on (some phones turn it off when unplugged).")
                    }
                }
            }

            Step {
                width: parent.width
                number: 3
                done: pane.installed
                current: pane.phoneReady && !pane.installed
                enabled: pane.phoneReady
                title: qsTr("Install NectarLink")
                last: true

                Txt {
                    width: parent.width
                    visible: !pane.phoneReady
                    role: "bodySmall"
                    muted: true
                    wrapMode: Text.WordWrap
                    text: pane.s.latest
                        ? qsTr("NectarLink %1 comes from its own releases on GitHub and is checked before it's sent to the phone.").arg(pane.s.latest)
                        : qsTr("NectarLink comes from its own releases on GitHub and is checked before it's sent to the phone.")
                }

                Repeater {
                    model: pane.readyPhones
                    delegate: PhoneRow {
                        required property var modelData
                        width: parent ? parent.width : 0
                        phone: modelData
                    }
                }
            }
        }
    }

    // ---- Pieces ----

    // One step: a numbered circle that becomes a check, a line to the next,
    // and what the step needs.
    component Step: Item {
        id: step
        property int number: 1
        property bool done: false
        property bool current: false
        property bool last: false
        property string title
        default property alias content: body.data
        // A Column's own height counts only what's shown (childrenRect counts hidden children too).
        implicitHeight: Math.max(dot.height, header.y + header.height)
            + (body.visible && body.height > 0 ? 6 + body.height : 0) + (last ? 0 : 22)
        opacity: enabled ? 1 : 0.5
        Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }

        Rectangle {
            id: dot
            width: 30; height: 30
            radius: 15
            color: step.done ? Theme.primary : step.current ? Theme.primaryContainer : Theme.surfaceContainerHigh
            border.width: step.done || step.current ? 0 : 1
            border.color: Theme.outlineVariant
            Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }
            Txt {
                anchors.centerIn: parent
                visible: !step.done
                role: "label"
                text: step.number
                color: step.current ? Theme.primaryContainerContent : Theme.surfaceContentVariant
            }
            Icon {
                id: tick
                anchors.centerIn: parent
                visible: step.done
                width: 16; height: 16
                path: Icons.check
                color: Theme.primaryContent
            }
            // A small pop when the step is done.
            SequentialAnimation {
                id: pop
                NumberAnimation { target: dot; property: "scale"; to: 1.18; duration: 140; easing.type: Easing.OutCubic }
                NumberAnimation { target: dot; property: "scale"; to: 1.0; duration: 220; easing.type: Easing.OutBack }
            }
        }
        onDoneChanged: if (done && pane.motion) pop.restart()

        // The line down to the next step, filling in once this one is done.
        Rectangle {
            visible: !step.last
            x: dot.width / 2 - 1
            y: dot.height + 4
            width: 2
            height: step.height - dot.height - 8
            radius: 1
            color: Theme.outlineVariant
            Rectangle {
                width: parent.width
                height: step.done ? parent.height : 0
                radius: 1
                color: Theme.primary
                Behavior on height { NumberAnimation { duration: pane.motion ? 360 : 0; easing.type: Easing.OutCubic } }
            }
        }

        Txt {
            id: header
            x: dot.width + 14
            width: step.width - x
            y: (dot.height - height) / 2
            role: "body"
            weight: Font.DemiBold
            wrapMode: Text.WordWrap
            text: step.title
        }
        Column {
            id: body
            x: header.x
            y: Math.max(dot.height, header.y + header.height) + 6
            width: header.width
            spacing: 10
            visible: step.enabled
        }
    }

    // A progress bar that eases to its value, or sweeps when it can't say.
    component ProgressLine: Rectangle {
        id: line
        property real value: 0
        property bool indeterminate: false
        height: 6
        radius: 3
        color: Theme.surfaceContainerHigh
        clip: true
        Rectangle {
            visible: !line.indeterminate
            height: parent.height
            radius: parent.radius
            width: parent.width * Math.max(0, Math.min(1, line.value))
            color: Theme.primary
            Behavior on width { NumberAnimation { duration: pane.motion ? 220 : 0; easing.type: Easing.OutCubic } }
        }
        Rectangle {
            id: sweep
            visible: line.indeterminate
            height: parent.height
            radius: parent.radius
            width: parent.width * 0.3
            color: Theme.primary
            NumberAnimation on x {
                running: line.indeterminate && line.visible && pane.motion
                loops: Animation.Infinite
                from: -sweep.width
                to: line.width
                duration: 1200
                easing.type: Easing.InOutSine
            }
        }
    }

    // A note with an icon, optionally asking for attention (a soft pulse) and with a button.
    component Hint: Rectangle {
        id: hint
        property string title
        property string text
        property string iconPath: Icons.info
        property bool attention: false
        property string actionText
        signal action()
        implicitHeight: hintRow.height + 24
        radius: Theme.radiusMd
        color: attention ? Theme.primaryContainer : Theme.surfaceContainerHigh
        Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }
        Row {
            id: hintRow
            x: 12; y: 12
            width: parent.width - 24
            spacing: 12
            Icon {
                id: hintIcon
                width: 20; height: 20
                path: hint.iconPath
                color: hint.attention ? Theme.primaryContainerContent : Theme.surfaceContentVariant
                SequentialAnimation on opacity {
                    running: hint.attention && hint.visible && pane.motion
                    loops: Animation.Infinite
                    NumberAnimation { from: 1; to: 0.35; duration: 700; easing.type: Easing.InOutSine }
                    NumberAnimation { from: 0.35; to: 1; duration: 700; easing.type: Easing.InOutSine }
                }
            }
            Column {
                width: parent.width - hintIcon.width - parent.spacing
                spacing: 8
                Txt {
                    width: parent.width
                    visible: hint.title.length > 0
                    role: "bodySmall"
                    weight: Font.DemiBold
                    wrapMode: Text.WordWrap
                    color: hint.attention ? Theme.primaryContainerContent : Theme.surfaceContent
                    text: hint.title
                }
                Txt {
                    width: parent.width
                    role: "bodySmall"
                    wrapMode: Text.WordWrap
                    color: hint.attention ? Theme.primaryContainerContent : Theme.surfaceContent
                    text: hint.text
                }
                Button {
                    visible: hint.actionText.length > 0
                    variant: "tonal"
                    size: "sm"
                    iconPath: Icons.openExternal
                    text: hint.actionText
                    onClicked: hint.action()
                }
            }
        }
    }

    // Numbered instructions.
    component Checklist: Column {
        id: list
        property var items: []
        spacing: 8
        Repeater {
            model: list.items
            delegate: Row {
                required property var modelData
                required property int index
                width: list.width
                spacing: 10
                Rectangle {
                    width: 22; height: 22
                    radius: 11
                    color: Theme.surfaceContainerHigh
                    Txt {
                        anchors.centerIn: parent
                        role: "caption"
                        text: index + 1
                    }
                }
                Txt {
                    width: parent.width - 32
                    role: "bodySmall"
                    wrapMode: Text.WordWrap
                    text: modelData
                }
            }
        }
    }

    // A ready phone: what's on it, the button, and the install's progress or result.
    component PhoneRow: Rectangle {
        id: row
        property var phone: ({})
        readonly property var mine: pane.job.serial === phone.serial ? pane.job : ({ kind: "idle" })
        readonly property bool working: ["downloading", "sending", "installing", "uninstalling"].indexOf(mine.kind) >= 0
        property bool confirmReplace: false
        implicitHeight: rowCol.height + 28
        radius: Theme.radiusMd
        color: mine.kind === "done" ? Theme.primaryContainer : Theme.surfaceContainerHigh
        Behavior on color { ColorAnimation { duration: Theme.fadeNormal } }

        Column {
            id: rowCol
            x: 14; y: 14
            width: parent.width - 28
            spacing: 10

            Item {
                width: parent.width
                height: Math.max(phoneText.height, actionBtn.height)
                Column {
                    id: phoneText
                    width: parent.width - actionBtn.width - 12
                    anchors.verticalCenter: parent.verticalCenter
                    spacing: 2
                    Txt {
                        width: parent.width
                        role: "body"
                        weight: Font.DemiBold
                        elide: Text.ElideRight
                        text: row.phone.name || ""
                    }
                    Txt {
                        width: parent.width
                        role: "caption"
                        muted: true
                        wrapMode: Text.WordWrap
                        text: {
                            const android = row.phone.android ? qsTr("Android %1").arg(row.phone.android) : ""
                            let app
                            if (row.phone.tooOld)
                                app = qsTr("needs Android 8.0 or newer")
                            else if (!row.phone.installed)
                                app = qsTr("NectarLink isn't on it yet")
                            else if (row.phone.outdated)
                                app = qsTr("has NectarLink %1, %2 is newer").arg(row.phone.installed).arg(pane.s.latest)
                            else
                                app = qsTr("has NectarLink %1").arg(row.phone.installed)
                            return android ? android + " · " + app : app
                        }
                    }
                }
                Button {
                    id: actionBtn
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    // While a result shows, its own buttons are the next step.
                    visible: !row.working && row.mine.kind !== "done" && row.mine.kind !== "failed"
                    enabled: !pane.busy && !row.phone.tooOld
                    variant: row.phone.installed && !row.phone.outdated ? "tonal" : "fill"
                    iconPath: row.phone.installed && !row.phone.outdated ? Icons.openExternal : Icons.download
                    text: row.phone.installed && !row.phone.outdated
                        ? qsTr("Open on phone")
                        : row.phone.installed ? qsTr("Update") : qsTr("Install")
                    onClicked: {
                        if (row.phone.installed && !row.phone.outdated)
                            PhoneInstall.openOnPhone(row.phone.serial)
                        else
                            PhoneInstall.install(row.phone.serial)
                    }
                }
            }

            // Progress.
            Column {
                width: parent.width
                spacing: 8
                visible: row.working
                Txt {
                    width: parent.width
                    role: "bodySmall"
                    wrapMode: Text.WordWrap
                    text: row.mine.kind === "downloading"
                        ? (row.mine.total > 0 ? qsTr("Downloading NectarLink… %1%").arg(pane.percent(row.mine.done, row.mine.total))
                                              : qsTr("Getting NectarLink…"))
                        : row.mine.kind === "sending"
                            ? qsTr("Sending it to the phone… %1%").arg(pane.percent(row.mine.done, row.mine.total))
                            : row.mine.kind === "uninstalling"
                                ? qsTr("Removing the other build…")
                                : qsTr("Installing… If the phone asks whether to allow it, tap Allow or Install.")
                }
                ProgressLine {
                    width: parent.width
                    indeterminate: row.mine.kind === "installing" || row.mine.kind === "uninstalling" || !(row.mine.total > 0)
                    value: row.mine.kind === "sending" ? pane.percent(row.mine.done, row.mine.total) / 100
                        : pane.percent(row.mine.done, row.mine.total) / 100
                }
            }

            // Done.
            Row {
                width: parent.width
                spacing: 12
                visible: row.mine.kind === "done"
                Rectangle {
                    id: doneBadge
                    width: 34; height: 34
                    radius: 17
                    color: Theme.primary
                    Icon {
                        anchors.centerIn: parent
                        width: 18; height: 18
                        path: Icons.check
                        color: Theme.primaryContent
                    }
                    scale: 1
                    onVisibleChanged: if (visible && pane.motion) donePop.restart()
                    SequentialAnimation {
                        id: donePop
                        NumberAnimation { target: doneBadge; property: "scale"; from: 0.4; to: 1.15; duration: 220; easing.type: Easing.OutCubic }
                        NumberAnimation { target: doneBadge; property: "scale"; to: 1.0; duration: 200; easing.type: Easing.OutBack }
                    }
                }
                Column {
                    width: parent.width - doneBadge.width - 12
                    spacing: 8
                    Txt {
                        width: parent.width
                        role: "bodySmall"
                        wrapMode: Text.WordWrap
                        color: Theme.primaryContainerContent
                        text: qsTr("NectarLink %1 is installed and opening on the phone. Finish its setup there, then pair it with this PC from Home.").arg(row.mine.version || "")
                    }
                    Row {
                        spacing: 8
                        Button {
                            variant: "tonal"
                            size: "sm"
                            text: qsTr("Open on phone")
                            onClicked: PhoneInstall.openOnPhone(row.phone.serial)
                        }
                        Button {
                            variant: "text"
                            size: "sm"
                            text: qsTr("Done")
                            onClicked: PhoneInstall.dismiss()
                        }
                    }
                }
            }

            // Why it didn't work, and what to do.
            Hint {
                width: parent.width
                visible: row.mine.kind === "failed" && !row.confirmReplace
                iconPath: Icons.warning
                attention: true
                title: row.mine.problem ? row.mine.problem.title : ""
                text: row.mine.problem ? row.mine.problem.text : ""
            }
            Row {
                spacing: 8
                visible: row.mine.kind === "failed" && !row.confirmReplace
                Button {
                    visible: row.mine.problem && row.mine.problem.action === "retry"
                    size: "sm"
                    iconPath: Icons.refresh
                    text: qsTr("Try again")
                    onClicked: PhoneInstall.install(row.phone.serial)
                }
                Button {
                    visible: row.mine.problem && row.mine.problem.action === "replace"
                    size: "sm"
                    text: qsTr("Replace it")
                    onClicked: row.confirmReplace = true
                }
                Button {
                    variant: "text"
                    size: "sm"
                    text: qsTr("Close")
                    onClicked: PhoneInstall.dismiss()
                }
            }
            // Replacing removes the other build's data: asked once more.
            Column {
                width: parent.width
                spacing: 8
                visible: row.confirmReplace
                Txt {
                    width: parent.width
                    role: "bodySmall"
                    wrapMode: Text.WordWrap
                    color: Theme.error
                    text: qsTr("Remove the NectarLink on %1 with everything it keeps, including the PCs it's paired with? This can't be undone.").arg(row.phone.name)
                }
                Row {
                    spacing: 8
                    Button {
                        size: "sm"
                        text: qsTr("Remove and install")
                        onClicked: {
                            row.confirmReplace = false
                            PhoneInstall.replace(row.phone.serial)
                        }
                    }
                    Button {
                        variant: "text"
                        size: "sm"
                        text: qsTr("Cancel")
                        onClicked: row.confirmReplace = false
                    }
                }
            }
        }
    }
}
