// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// What's new in this version (#461), from the release notes it was built
// with. After an update it opens by itself, leading with what's new since
// the last look (Nightly to Nightly too); Settings › Connection & About
// opens it any time. Earlier versions fold away below.
Sheet {
    id: sheet
    cardWidth: 600

    // Read when it opens, so the list doesn't change under the reader.
    property var notes: ({ releases: [], newCount: 0 })
    property string show: "new"
    property var expanded: ({})
    property bool wasOpened: false

    readonly property var releases: notes.releases || []
    readonly property var current: releases.length > 0 ? releases[0] : null
    readonly property var earlier: releases.slice(1)
    readonly property int newCount: notes.newCount || 0
    readonly property int totalCount: {
        if (!current) return 0
        let n = 0
        for (const s of current.sections) n += s.entries.length
        return n
    }
    // Only the new entries when there are some and not everything is new.
    readonly property bool onlyNew: show === "new" && newCount > 0 && newCount < totalCount

    function openNotes() {
        try { notes = JSON.parse(AppController.whatsNew()) } catch (e) { notes = ({ releases: [], newCount: 0 }) }
        show = "new"
        expanded = ({})
        wasOpened = true
        open()
    }

    function formatDate(iso) {
        if (!iso || iso.length === 0) return ""
        const d = new Date(iso + "T00:00:00")
        return isNaN(d.getTime()) ? iso : d.toLocaleDateString(Qt.locale(), Locale.LongFormat)
    }

    function sectionIcon(title) {
        switch (title) {
        case "New": return Icons.sparkles
        case "Fixed": return Icons.check
        case "Changed": return Icons.refresh
        default: return Icons.chevronRight
        }
    }

    onOpenedChanged: {
        if (!opened && wasOpened) {
            wasOpened = false
            AppController.markWhatsNewSeen()
        }
    }

    Column {
        width: parent.width
        spacing: 18

        // Heading: the version this is.
        Row {
            width: parent.width
            spacing: 14
            Rectangle {
                width: 44; height: 44
                radius: 22
                color: Theme.primaryContainer
                Icon {
                    anchors.centerIn: parent
                    width: 22; height: 22
                    path: Icons.sparkles
                    color: Theme.primaryContainerContent
                }
            }
            Column {
                anchors.verticalCenter: parent.verticalCenter
                width: parent.width - 58
                spacing: 2
                Txt { text: qsTr("What's new"); role: "headline" }
                Txt {
                    width: parent.width
                    text: qsTr("NectarLink %1").arg(AppController.version)
                    role: "bodySmall"
                    muted: true
                    elide: Text.ElideRight
                }
            }
        }

        Txt {
            width: parent.width
            visible: text.length > 0
            wrapMode: Text.WordWrap
            role: "body"
            text: sheet.newCount > 0 && sheet.newCount < sheet.totalCount
                ? (sheet.newCount === 1 ? qsTr("1 change since you last looked.")
                    : qsTr("%1 changes since you last looked.").arg(sheet.newCount))
                : (sheet.current && sheet.current.summary.length > 0 ? sheet.current.summary : "")
            textFormat: Text.StyledText
            linkColor: Theme.primary
            onLinkActivated: (link) => Qt.openUrlExternally(link)
        }

        Segmented {
            visible: sheet.newCount > 0 && sheet.newCount < sheet.totalCount
            options: [
                { value: "new", label: qsTr("New to you") },
                { value: "all", label: qsTr("Everything in this version") }
            ]
            value: sheet.show
            onPicked: (value) => sheet.show = value
        }

        Txt {
            width: parent.width
            visible: !sheet.current
            wrapMode: Text.WordWrap
            muted: true
            text: qsTr("The release notes aren't in this build. They're on GitHub.")
        }

        // This version, section by section.
        Repeater {
            model: sheet.current ? sheet.current.sections : []
            delegate: Column {
                id: section
                required property var modelData
                readonly property var shown: sheet.onlyNew
                    ? modelData.entries.filter(e => e.new) : modelData.entries
                width: parent.width
                spacing: 6
                visible: shown.length > 0

                Row {
                    spacing: 8
                    bottomPadding: 2
                    Icon {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 16; height: 16
                        path: sheet.sectionIcon(section.modelData.title)
                        color: Theme.primary
                    }
                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        text: section.modelData.title
                        role: "label"
                        color: Theme.primary
                    }
                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        text: section.shown.length
                        role: "label"
                        muted: true
                    }
                }

                Repeater {
                    model: section.shown
                    delegate: Rectangle {
                        id: entry
                        required property var modelData
                        width: section.width
                        height: entryText.height + 24
                        radius: Theme.radiusMd
                        color: Theme.surfaceContainer

                        // A dot for what wasn't here last time.
                        Rectangle {
                            visible: entry.modelData.new && !sheet.onlyNew && sheet.newCount < sheet.totalCount
                            x: 10
                            y: 18
                            width: 6; height: 6
                            radius: 3
                            color: Theme.primary
                        }
                        Column {
                            id: entryText
                            x: 24
                            y: 12
                            width: parent.width - 40
                            spacing: 3
                            Txt {
                                width: parent.width
                                visible: entry.modelData.title.length > 0
                                text: entry.modelData.title
                                role: "title"
                                size: 14
                                wrapMode: Text.WordWrap
                            }
                            Txt {
                                width: parent.width
                                visible: entry.modelData.body.length > 0
                                text: entry.modelData.body
                                textFormat: Text.StyledText
                                role: entry.modelData.title.length > 0 ? "bodySmall" : "body"
                                muted: entry.modelData.title.length > 0
                                wrapMode: Text.WordWrap
                                linkColor: Theme.primary
                                onLinkActivated: (link) => Qt.openUrlExternally(link)
                                HoverHandler {
                                    enabled: parent.hoveredLink.length > 0
                                    cursorShape: Qt.PointingHandCursor
                                }
                            }
                        }
                    }
                }
            }
        }

        // Earlier versions, folded.
        Column {
            width: parent.width
            spacing: 6
            visible: sheet.earlier.length > 0 && !sheet.onlyNew

            Txt { text: qsTr("Earlier versions"); role: "label"; muted: true; bottomPadding: 2 }

            Repeater {
                model: sheet.earlier
                delegate: Column {
                    id: release
                    required property var modelData
                    readonly property bool open: !!sheet.expanded[modelData.version]
                    width: parent.width
                    spacing: 6

                    Rectangle {
                        width: parent.width
                        height: 44
                        radius: Theme.radiusMd
                        color: releaseArea.containsMouse ? Theme.surfaceContainerHigh : Theme.surfaceContainer
                        Behavior on color { ColorAnimation { duration: Theme.fadeFast } }
                        Accessible.role: Accessible.Button
                        Accessible.name: release.modelData.version
                        Txt {
                            anchors.left: parent.left
                            anchors.leftMargin: 16
                            anchors.verticalCenter: parent.verticalCenter
                            text: release.modelData.version
                            role: "title"
                            size: 14
                        }
                        Txt {
                            anchors.right: chevron.left
                            anchors.rightMargin: 10
                            anchors.verticalCenter: parent.verticalCenter
                            text: sheet.formatDate(release.modelData.date)
                            role: "bodySmall"
                            muted: true
                        }
                        Icon {
                            id: chevron
                            anchors.right: parent.right
                            anchors.rightMargin: 14
                            anchors.verticalCenter: parent.verticalCenter
                            width: 16; height: 16
                            path: Icons.chevronDown
                            color: Theme.surfaceContentVariant
                            rotation: release.open ? 180 : 0
                            Behavior on rotation { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeNormal; easing.type: Easing.OutCubic } }
                        }
                        MouseArea {
                            id: releaseArea
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: {
                                const next = Object.assign({}, sheet.expanded)
                                next[release.modelData.version] = !release.open
                                sheet.expanded = next
                            }
                        }
                    }

                    Loader {
                        width: parent.width
                        active: release.open
                        visible: active
                        sourceComponent: Column {
                            width: parent ? parent.width : 0
                            spacing: 10
                            Txt {
                                width: parent.width
                                visible: release.modelData.summary.length > 0
                                leftPadding: 16
                                rightPadding: 16
                                text: release.modelData.summary
                                textFormat: Text.StyledText
                                wrapMode: Text.WordWrap
                                role: "bodySmall"
                                muted: true
                                linkColor: Theme.primary
                                onLinkActivated: (link) => Qt.openUrlExternally(link)
                            }
                            Repeater {
                                model: release.modelData.sections
                                delegate: Column {
                                    id: oldSection
                                    required property var modelData
                                    width: parent.width
                                    spacing: 4
                                    leftPadding: 16
                                    rightPadding: 16
                                    Txt { text: oldSection.modelData.title; role: "label"; color: Theme.primary }
                                    Repeater {
                                        model: oldSection.modelData.entries
                                        delegate: Txt {
                                            required property var modelData
                                            width: oldSection.width - 32
                                            text: (modelData.title.length > 0
                                                ? "<b>" + modelData.title.replace(/&/g, "&amp;").replace(/</g, "&lt;") + "</b> "
                                                : "") + modelData.body
                                            textFormat: Text.StyledText
                                            wrapMode: Text.WordWrap
                                            role: "bodySmall"
                                            linkColor: Theme.primary
                                            onLinkActivated: (link) => Qt.openUrlExternally(link)
                                            bottomPadding: 4
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Item {
            width: parent.width
            height: doneBtn.height
            Button {
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("All release notes")
                iconPath: Icons.openExternal
                variant: "text"
                onClicked: Qt.openUrlExternally("https://github.com/thtbee/nectarlink/releases")
            }
            Button {
                id: doneBtn
                anchors.right: parent.right
                text: qsTr("Got it")
                onClicked: sheet.close()
            }
        }
    }
}
