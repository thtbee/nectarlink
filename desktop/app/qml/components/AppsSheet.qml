// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// A phone's apps, to open one in a window of its own on this PC (Elevated:
// the phone runs it on a display of its own). Searchable; Enter opens the
// first match.
Sheet {
    id: sheet
    property string deviceId
    property string phoneName
    cardWidth: 640

    function openFor(device, name) {
        deviceId = device
        phoneName = name
        search.text = ""
        Mirror.loadApps(device)
        open()
        search.forceActiveFocus()
    }

    readonly property var apps: {
        if (Mirror.appsDevice !== deviceId)
            return []
        try { return JSON.parse(Mirror.apps) } catch (e) { return [] }
    }
    readonly property var recent: {
        if (Mirror.appsDevice !== deviceId)
            return []
        try { return JSON.parse(Mirror.recentApps) } catch (e) { return [] }
    }
    readonly property string query: search.text.trim().toLocaleLowerCase()
    readonly property var shown: query.length === 0 ? apps
        : apps.filter(a => a.label.toLocaleLowerCase().indexOf(query) >= 0 || a.pkg.indexOf(query) >= 0)
    readonly property string loadState: Mirror.appsDevice === deviceId ? Mirror.appsState : ""

    function launch(app) {
        Mirror.startApp(deviceId, app.pkg, app.label)
        close()
    }

    Column {
        width: parent.width
        spacing: 14

        Item {
            width: parent.width
            height: Math.max(title.height, refresh.height)
            Txt {
                id: title
                anchors.left: parent.left
                anchors.right: refresh.left
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Open an app from %1").arg(sheet.phoneName)
                role: "headline"
                elide: Text.ElideRight
            }
            IconButton {
                id: refresh
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.refresh
                label: qsTr("Refresh")
                enabled: sheet.loadState !== "loading"
                onClicked: Mirror.loadApps(sheet.deviceId)
            }
        }
        Txt {
            width: parent.width
            wrapMode: Text.WordWrap
            role: "bodySmall"
            muted: true
            text: qsTr("It opens in a window of its own and runs on the phone, beside what's on the phone's screen.")
        }

        Rectangle {
            width: parent.width
            height: 40
            radius: Theme.graphite ? Theme.radiusSm : height / 2
            color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
            border.width: search.activeFocus ? 2 : (Theme.graphite ? 1 : 0)
            border.color: search.activeFocus ? Theme.primary : Theme.outlineVariant
            Icon {
                id: searchIcon
                anchors.left: parent.left
                anchors.leftMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                width: 18; height: 18
                path: Icons.search
                color: Theme.surfaceContentVariant
            }
            TextInput {
                id: search
                anchors.left: searchIcon.right
                anchors.leftMargin: 10
                anchors.right: parent.right
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                font.family: Theme.fontUi
                font.pixelSize: 14
                color: Theme.surfaceContent
                selectionColor: Theme.primaryContainer
                selectedTextColor: Theme.primaryContainerContent
                clip: true
                Accessible.name: qsTr("Search apps")
                Keys.onReturnPressed: if (sheet.shown.length > 0) sheet.launch(sheet.shown[0])
                Keys.onEnterPressed: if (sheet.shown.length > 0) sheet.launch(sheet.shown[0])
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: search.text.length === 0
                    role: "body"
                    muted: true
                    text: qsTr("Search apps")
                }
            }
        }

        // Loading, failed or empty.
        Column {
            width: parent.width
            visible: sheet.loadState !== "ready" || sheet.shown.length === 0
            spacing: 10
            topPadding: 24
            bottomPadding: 24
            Spinner {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: sheet.loadState === "loading" || sheet.loadState === ""
                width: 24; height: 24
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: sheet.loadState === "failed" ? Mirror.appsError
                    : sheet.loadState === "ready" ? (sheet.query.length > 0 ? qsTr("No apps match.") : qsTr("No apps to open."))
                    : qsTr("Getting the phone's apps…")
            }
        }

        // Recently opened on this PC.
        Column {
            id: recentSection
            width: parent.width
            visible: sheet.loadState === "ready" && sheet.query.length === 0 && sheet.recent.length > 0
            spacing: 8

            Txt {
                role: "label"
                muted: true
                text: qsTr("Recent")
            }

            Row {
                width: parent.width
                spacing: 4
                Repeater {
                    model: sheet.recent
                    delegate: Item {
                        id: recentCell
                        required property var modelData
                        width: Math.min(96, Math.floor((recentSection.width - 20) / Math.max(1, sheet.recent.length)))
                        height: 82
                        Accessible.role: Accessible.Button
                        Accessible.name: modelData.label
                        Accessible.onPressAction: sheet.launch(modelData)

                        Rectangle {
                            anchors.fill: parent
                            anchors.margins: 2
                            radius: Theme.radiusMd
                            color: Theme.surfaceContent
                            opacity: recentTap.pressed ? 0.12 : (recentHover.hovered ? 0.06 : 0)
                        }
                        Column {
                            anchors.centerIn: parent
                            width: parent.width - 8
                            spacing: 6
                            Item {
                                anchors.horizontalCenter: parent.horizontalCenter
                                width: 38; height: 38
                                Image {
                                    id: recentIcon
                                    anchors.fill: parent
                                    source: recentCell.modelData.icon
                                    sourceSize: Qt.size(76, 76)
                                    smooth: true
                                    mipmap: true
                                    visible: status === Image.Ready
                                }
                                Avatar {
                                    anchors.fill: parent
                                    visible: recentIcon.status !== Image.Ready
                                    name: recentCell.modelData.label
                                }
                            }
                            Txt {
                                width: parent.width
                                horizontalAlignment: Text.AlignHCenter
                                role: "bodySmall"
                                text: recentCell.modelData.label
                                elide: Text.ElideRight
                                maximumLineCount: 1
                            }
                        }
                        HoverHandler { id: recentHover; cursorShape: Qt.PointingHandCursor }
                        TapHandler { id: recentTap; onTapped: sheet.launch(recentCell.modelData) }
                    }
                }
            }

            Divider {}

            Txt {
                role: "label"
                muted: true
                text: qsTr("All apps")
            }
        }

        GridView {
            id: grid
            width: parent.width
            height: Math.min(contentHeight, recentSection.visible ? 300 : 420)
            visible: sheet.loadState === "ready" && sheet.shown.length > 0
            clip: true
            model: sheet.shown
            cellWidth: Math.floor(width / Math.max(1, Math.floor(width / 112)))
            cellHeight: 104
            boundsBehavior: Flickable.StopAtBounds
            delegate: Item {
                id: cell
                required property var modelData
                width: grid.cellWidth
                height: grid.cellHeight
                Accessible.role: Accessible.Button
                Accessible.name: modelData.label
                Accessible.onPressAction: sheet.launch(modelData)

                Rectangle {
                    anchors.fill: parent
                    anchors.margins: 4
                    radius: Theme.radiusMd
                    color: Theme.surfaceContent
                    opacity: tap.pressed ? 0.12 : (hover.hovered ? 0.06 : 0)
                }
                Column {
                    anchors.centerIn: parent
                    width: parent.width - 12
                    spacing: 8
                    Item {
                        anchors.horizontalCenter: parent.horizontalCenter
                        width: 44; height: 44
                        Image {
                            id: icon
                            anchors.fill: parent
                            source: cell.modelData.icon
                            sourceSize: Qt.size(88, 88)
                            smooth: true
                            mipmap: true
                            visible: status === Image.Ready
                        }
                        Avatar {
                            anchors.fill: parent
                            visible: icon.status !== Image.Ready
                            name: cell.modelData.label
                        }
                    }
                    Txt {
                        width: parent.width
                        horizontalAlignment: Text.AlignHCenter
                        role: "bodySmall"
                        text: cell.modelData.label
                        elide: Text.ElideRight
                        maximumLineCount: 2
                        wrapMode: Text.WordWrap
                    }
                }
                HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
                TapHandler { id: tap; onTapped: sheet.launch(cell.modelData) }
            }
        }
    }
}
