// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Choose a photo or a file on the phone to attach to a message. Tabs: Photos
// (the phone's gallery, newest first) and Files (its folders). A pick is
// brought to this PC and staged as the attachment (PhonePicker, in Rust).
// Another tab (the phone's camera) is another entry in `tabs`, a pane below
// and the calls behind it.
Sheet {
    id: sheet
    property string deviceId
    property string deviceName
    property string tab: "photos"
    // A file was picked and is staged.
    signal attached

    function feature(id) {
        return AppController.capsRevision >= 0 && deviceId.length > 0
            ? AppController.featureState(deviceId, id) : ({})
    }
    readonly property var photosFeature: feature("files.recent_photos")
    // Photos come as an ordinary transfer, so files must be allowed too.
    readonly property var sendFeature: feature("files.send")
    readonly property var filesFeature: feature("files.browse_phone")
    readonly property var photosGate: photosFeature.state === "available" ? sendFeature : photosFeature

    readonly property bool photosAvailable: photosFeature.state === "available" && sendFeature.state === "available"
    readonly property bool filesAvailable: filesFeature.state === "available"
    // Whether anything can be picked from the phone now.
    readonly property bool canPick: photosAvailable || filesAvailable

    function shown(f) { return f.state === "available" || f.state === "locked" || f.state === "partial" }
    // The tabs, by what the phone offers; one the phone offers but that is
    // locked stays, to say what unlocks it.
    readonly property var tabs: {
        const out = []
        if (shown(photosGate)) out.push({ value: "photos", label: qsTr("Photos") })
        if (shown(filesFeature)) out.push({ value: "files", label: qsTr("Files") })
        return out
    }
    readonly property real pickerHeight: Math.max(300, Math.min(500, sheet.height - 120))
    readonly property bool busy: PhonePicker.busy.length > 0

    cardWidth: 600

    function show() {
        PhonePicker.open(deviceId)
        photoModel.clear()
        tab = photosAvailable || !filesAvailable ? "photos" : "files"
        open()
        loadTab()
    }
    // Loads the tab shown, when it hasn't been.
    function loadTab() {
        if (!opened) return
        if (tab === "photos" && photosAvailable && PhonePicker.photosStatus === "idle")
            PhonePicker.loadPhotos()
        else if (tab === "files" && filesAvailable && PhonePicker.filesStatus === "idle")
            PhonePicker.openFolder("")
    }
    onTabChanged: loadTab()
    onOpenedChanged: if (!opened) PhonePicker.close()

    // The photos as a model that only grows, so the grid keeps its place as pages come.
    ListModel { id: photoModel }
    function syncPhotos() {
        let rows = []
        try { rows = JSON.parse(PhonePicker.photos) } catch (e) { rows = [] }
        let same = rows.length >= photoModel.count
        for (let i = 0; same && i < photoModel.count; i++)
            same = photoModel.get(i).id === rows[i].id
        if (!same) photoModel.clear()
        for (let i = photoModel.count; i < rows.length; i++)
            photoModel.append(rows[i])
    }
    Connections {
        target: PhonePicker
        function onPhotosChanged() { sheet.syncPhotos() }
        function onPickedChanged() {
            if (sheet.opened) {
                sheet.close()
                sheet.attached()
            }
        }
    }

    // Names the reason a tab has nothing to show, with a way on.
    component Notice: Column {
        id: notice
        property string icon: Icons.phone
        property string title
        property string hint
        property var feature: ({})
        property bool retry: false
        signal retried
        spacing: 10
        Icon {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 32; height: 32
            stroke: 1.5
            path: notice.icon
            color: Theme.surfaceContentVariant
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            role: "title"
            size: 15
            text: notice.title
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            role: "bodySmall"
            muted: true
            text: notice.hint
        }
        LockChip {
            anchors.horizontalCenter: parent.horizontalCenter
            visible: label.length > 0
            feature: notice.feature
        }
        Button {
            anchors.horizontalCenter: parent.horizontalCenter
            visible: notice.retry
            variant: "tonal"
            size: "sm"
            text: qsTr("Try again")
            onClicked: notice.retried()
        }
    }

    Item {
        width: parent.width
        height: sheet.pickerHeight

        Txt {
            id: heading
            anchors.left: parent.left
            anchors.right: closeButton.left
            anchors.rightMargin: 8
            anchors.verticalCenter: closeButton.verticalCenter
            elide: Text.ElideRight
            role: "title"
            text: qsTr("Attach from %1").arg(sheet.deviceName)
            Accessible.role: Accessible.Heading
        }
        IconButton {
            id: closeButton
            anchors.right: parent.right
            anchors.top: parent.top
            iconPath: Icons.close
            label: qsTr("Close")
            onClicked: sheet.close()
        }

        Segmented {
            id: tabBar
            anchors.top: closeButton.bottom
            anchors.topMargin: 8
            anchors.horizontalCenter: parent.horizontalCenter
            visible: sheet.tabs.length > 1
            options: sheet.tabs
            value: sheet.tab
            onPicked: (value) => sheet.tab = value
        }

        Txt {
            id: problem
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            visible: PhonePicker.problem.length > 0
            height: visible ? implicitHeight + 8 : 0
            wrapMode: Text.WordWrap
            role: "bodySmall"
            color: Theme.error
            text: PhonePicker.problem
            Accessible.role: Accessible.AlertMessage
        }

        Item {
            id: panes
            anchors.top: tabBar.visible ? tabBar.bottom : closeButton.bottom
            anchors.topMargin: 12
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: problem.top
            clip: true

            // ---- Photos ----
            Item {
                id: photosPane
                anchors.fill: parent
                visible: sheet.tab === "photos"

                Spinner {
                    anchors.centerIn: parent
                    visible: PhonePicker.photosStatus === "loading" && sheet.photosAvailable
                }
                Notice {
                    anchors.centerIn: parent
                    width: Math.min(parent.width - 24, 380)
                    visible: !sheet.photosAvailable
                        || (PhonePicker.photosStatus !== "ready" && PhonePicker.photosStatus !== "loading"
                            && PhonePicker.photosStatus !== "idle")
                    icon: Icons.photo
                    feature: sheet.photosAvailable ? ({}) : sheet.photosGate
                    retry: sheet.photosAvailable
                    title: !sheet.photosAvailable ? qsTr("Photos aren't available")
                        : PhonePicker.photosStatus === "off" ? qsTr("Photos are off for %1").arg(sheet.deviceName)
                        : PhonePicker.photosStatus === "unsupported" ? qsTr("Allow photos on %1").arg(sheet.deviceName)
                        : PhonePicker.photosStatus === "offline" ? qsTr("%1 isn't connected").arg(sheet.deviceName)
                        : qsTr("Couldn't load photos")
                    hint: !sheet.photosAvailable ? qsTr("Turn on photos and file transfers for this phone first.")
                        : PhonePicker.photosStatus === "off" ? qsTr("Enable Photos in Settings.")
                        : PhonePicker.photosStatus === "unsupported" ? qsTr("Grant photos permission on %1.").arg(sheet.deviceName)
                        : PhonePicker.photosStatus === "offline" ? qsTr("Connect %1 to choose a photo.").arg(sheet.deviceName)
                        : qsTr("Unable to load photos.")
                    onRetried: PhonePicker.loadPhotos()
                }
                Notice {
                    anchors.centerIn: parent
                    width: Math.min(parent.width - 24, 380)
                    visible: sheet.photosAvailable && PhonePicker.photosStatus === "ready" && photoModel.count === 0
                    icon: Icons.photo
                    title: qsTr("No photos")
                    hint: qsTr("Videos can't be attached to a message.")
                }

                GridView {
                    id: grid
                    anchors.fill: parent
                    visible: photoModel.count > 0
                    readonly property int columns: Math.max(3, Math.floor(width / 112))
                    cellWidth: Math.floor(width / columns)
                    cellHeight: cellWidth
                    model: photoModel
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    cacheBuffer: cellHeight * 2
                    onAtYEndChanged: if (atYEnd && PhonePicker.photosMore && !PhonePicker.photosLoadingOlder)
                        PhonePicker.loadOlderPhotos()
                    footer: Item {
                        width: grid.width
                        height: PhonePicker.photosLoadingOlder ? 44 : 0
                        Spinner {
                            anchors.centerIn: parent
                            visible: PhonePicker.photosLoadingOlder
                        }
                    }

                    delegate: Item {
                        id: tile
                        required property string id
                        required property string name
                        required property real date
                        width: grid.cellWidth
                        height: grid.cellHeight
                        readonly property bool picking: PhonePicker.busy === id
                        readonly property string thumb: {
                            const revision = PhonePicker.thumbRevision
                            return revision >= 0 ? PhonePicker.thumbUrl(id) : ""
                        }
                        activeFocusOnTab: true
                        Accessible.role: Accessible.Button
                        Accessible.name: qsTr("%1, %2").arg(name).arg(new Date(date).toLocaleDateString(Qt.locale(), Locale.ShortFormat))
                        Accessible.onPressAction: tile.pick()
                        function pick() { if (!sheet.busy) PhonePicker.pickPhoto(id) }
                        Keys.onPressed: (event) => {
                            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                tile.pick()
                                event.accepted = true
                            }
                        }

                        Rectangle {
                            anchors.fill: parent
                            anchors.margins: 3
                            radius: Theme.radiusSm
                            color: Theme.surfaceContainerHigh
                            clip: true
                            border.width: Theme.focusVisible(tile) ? 2 : 0
                            border.color: Theme.primary
                            Icon {
                                anchors.centerIn: parent
                                width: 24; height: 24
                                visible: tile.thumb.length === 0
                                path: Icons.photo
                                color: Theme.surfaceContentVariant
                            }
                            Image {
                                anchors.fill: parent
                                autoTransform: true
                                asynchronous: true
                                fillMode: Image.PreserveAspectCrop
                                sourceSize: Qt.size(256, 256)
                                source: tile.thumb
                                visible: status === Image.Ready
                            }
                            Rectangle {
                                anchors.fill: parent
                                color: Theme.surfaceContent
                                opacity: tileMouse.pressed ? 0.18 : (tileMouse.containsMouse && !sheet.busy ? 0.08 : 0)
                            }
                            Rectangle {
                                anchors.fill: parent
                                visible: tile.picking
                                color: Theme.scrim
                                Spinner { anchors.centerIn: parent; color: "white" }
                            }
                        }
                        MouseArea {
                            id: tileMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            enabled: !sheet.busy
                            cursorShape: Theme.controlCursor
                            onClicked: tile.pick()
                        }
                    }
                }
            }

            // ---- Files ----
            Item {
                id: filesPane
                anchors.fill: parent
                visible: sheet.tab === "files"

                readonly property var crumbs: { try { return JSON.parse(PhonePicker.crumbs) } catch (e) { return [] } }
                readonly property var entries: { try { return JSON.parse(PhonePicker.entries) } catch (e) { return [] } }
                // The folder above the one shown ("" at the root).
                readonly property string parentPath: crumbs.length > 1 ? crumbs[crumbs.length - 2].path : ""

                function iconOf(name) {
                    const ext = name.split(".").pop().toLowerCase()
                    if (["m4a", "mp3", "aac", "ogg", "wav", "amr", "3gp"].indexOf(ext) >= 0) return Icons.music
                    if (["png", "jpg", "jpeg", "webp", "gif", "bmp"].indexOf(ext) >= 0) return Icons.photo
                    return Icons.text
                }

                Row {
                    id: crumbBar
                    anchors.top: parent.top
                    anchors.left: parent.left
                    anchors.right: parent.right
                    height: 34
                    spacing: 4
                    visible: sheet.filesAvailable
                    IconButton {
                        anchors.verticalCenter: parent.verticalCenter
                        iconPath: Icons.back
                        label: qsTr("Up one folder")
                        enabled: filesPane.crumbs.length > 1 && PhonePicker.filesStatus !== "loading"
                        onClicked: PhonePicker.openFolder(filesPane.parentPath)
                    }
                    Flickable {
                        id: crumbFlick
                        anchors.verticalCenter: parent.verticalCenter
                        width: parent.width - 38
                        height: parent.height
                        contentWidth: crumbRow.width
                        contentX: Math.max(0, contentWidth - width)
                        clip: true
                        interactive: contentWidth > width
                        boundsBehavior: Flickable.StopAtBounds
                        Row {
                            id: crumbRow
                            height: parent.height
                            spacing: 2
                            Repeater {
                                model: filesPane.crumbs
                                delegate: Row {
                                    id: crumb
                                    required property var modelData
                                    required property int index
                                    readonly property bool last: index === filesPane.crumbs.length - 1
                                    height: crumbRow.height
                                    spacing: 2
                                    Icon {
                                        visible: crumb.index > 0
                                        anchors.verticalCenter: parent.verticalCenter
                                        width: 14; height: 14
                                        path: Icons.chevronRight
                                        color: Theme.surfaceContentVariant
                                    }
                                    Chip {
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: crumb.modelData.name.length > 0 ? crumb.modelData.name : qsTr("Phone")
                                        iconPath: crumb.index === 0 ? Icons.phone : ""
                                        selected: crumb.last
                                        interactive: !crumb.last && PhonePicker.filesStatus !== "loading"
                                        onClicked: PhonePicker.openFolder(crumb.modelData.path)
                                    }
                                }
                            }
                        }
                    }
                }

                Spinner {
                    anchors.centerIn: parent
                    visible: PhonePicker.filesStatus === "loading"
                }
                Notice {
                    anchors.centerIn: parent
                    width: Math.min(parent.width - 24, 380)
                    visible: !sheet.filesAvailable
                        || (PhonePicker.filesStatus !== "ready" && PhonePicker.filesStatus !== "loading"
                            && PhonePicker.filesStatus !== "idle")
                    icon: Icons.folder
                    feature: sheet.filesAvailable ? ({}) : sheet.filesFeature
                    retry: sheet.filesAvailable
                    title: !sheet.filesAvailable ? qsTr("Phone files aren't available")
                        : PhonePicker.filesStatus === "off" ? qsTr("Files are off for this PC")
                        : PhonePicker.filesStatus === "unsupported" ? qsTr("Allow files on %1").arg(sheet.deviceName)
                        : PhonePicker.filesStatus === "offline" ? qsTr("%1 isn't connected").arg(sheet.deviceName)
                        : qsTr("Couldn't open the folder")
                    hint: !sheet.filesAvailable ? qsTr("Turn on Phone storage for this phone in Settings first.")
                        : PhonePicker.filesStatus === "off" ? qsTr("Allow this PC to browse files in the NectarLink app on the phone, then try again.")
                        : PhonePicker.filesStatus === "unsupported" ? qsTr("Grant access to files in the NectarLink app on %1.").arg(sheet.deviceName)
                        : PhonePicker.filesStatus === "offline" ? qsTr("Connect %1 to choose a file.").arg(sheet.deviceName)
                        : qsTr("Unable to list that folder.")
                    onRetried: PhonePicker.openFolder(PhonePicker.folder)
                }
                Notice {
                    anchors.centerIn: parent
                    width: Math.min(parent.width - 24, 380)
                    visible: sheet.filesAvailable && PhonePicker.filesStatus === "ready" && filesPane.entries.length === 0
                    icon: Icons.folder
                    title: qsTr("This folder is empty")
                    hint: ""
                }

                ListView {
                    id: fileList
                    anchors.top: crumbBar.bottom
                    anchors.topMargin: 6
                    anchors.bottom: parent.bottom
                    anchors.left: parent.left
                    anchors.right: parent.right
                    visible: sheet.filesAvailable && PhonePicker.filesStatus === "ready" && filesPane.entries.length > 0
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    model: filesPane.entries
                    // A new folder starts at its top.
                    onModelChanged: positionViewAtBeginning()

                    header: Txt {
                        width: fileList.width
                        height: implicitHeight + 8
                        role: "caption"
                        muted: true
                        wrapMode: Text.WordWrap
                        text: qsTr("Pictures and voice notes can be attached to a message.")
                    }

                    delegate: Item {
                        id: row
                        required property var modelData
                        readonly property bool dir: modelData.isDir
                        readonly property bool usable: dir || modelData.canAttach
                        readonly property bool picking: PhonePicker.busy === modelData.path
                        width: fileList.width
                        height: 48
                        activeFocusOnTab: usable
                        Accessible.role: Accessible.Button
                        Accessible.name: dir ? qsTr("Folder %1").arg(modelData.name)
                            : (usable ? modelData.name : qsTr("%1, can't be attached").arg(modelData.name))
                        Accessible.onPressAction: row.open()
                        function open() {
                            if (!usable || sheet.busy || PhonePicker.filesStatus === "loading") return
                            if (dir) PhonePicker.openFolder(modelData.path)
                            else PhonePicker.pickFile(modelData.path)
                        }
                        Keys.onPressed: (event) => {
                            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                row.open()
                                event.accepted = true
                            }
                        }

                        Rectangle {
                            anchors.fill: parent
                            anchors.topMargin: 1
                            anchors.bottomMargin: 1
                            radius: Theme.radiusSm
                            color: rowMouse.containsMouse && row.usable && !sheet.busy ? Theme.surfaceContainerHigh : "transparent"
                            border.width: Theme.focusVisible(row) ? 2 : 0
                            border.color: Theme.primary
                        }
                        Row {
                            anchors.left: parent.left
                            anchors.leftMargin: 10
                            anchors.right: trail.left
                            anchors.rightMargin: 8
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 12
                            opacity: row.usable ? 1 : 0.45
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 20; height: 20
                                path: row.dir ? Icons.folder : filesPane.iconOf(row.modelData.name)
                                color: row.dir ? Theme.primary : Theme.surfaceContentVariant
                            }
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                width: parent.width - 32
                                elide: Text.ElideMiddle
                                role: "body"
                                text: row.modelData.name
                            }
                        }
                        Row {
                            id: trail
                            anchors.right: parent.right
                            anchors.rightMargin: 10
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 8
                            Txt {
                                anchors.verticalCenter: parent.verticalCenter
                                visible: !row.dir && !row.picking
                                role: "bodySmall"
                                muted: true
                                text: row.modelData.size
                            }
                            Icon {
                                anchors.verticalCenter: parent.verticalCenter
                                visible: row.dir
                                width: 16; height: 16
                                path: Icons.chevronRight
                                color: Theme.surfaceContentVariant
                            }
                            Spinner {
                                anchors.verticalCenter: parent.verticalCenter
                                width: 18; height: 18
                                visible: row.picking
                            }
                        }
                        MouseArea {
                            id: rowMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            enabled: row.usable && !sheet.busy
                            cursorShape: Theme.controlCursor
                            onClicked: row.open()
                        }
                    }
                }
            }
        }
    }
}
