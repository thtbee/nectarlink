// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// What's new (#461): a small panel under the top bar's sparkle button, with
// one short line per change since the last look, grouped New, Changed and
// Fixed, and a link to the full release notes. It never opens by itself: a
// dot on the button says there's something new, and opening it clears that.
Item {
    id: panel
    anchors.fill: parent
    z: 120
    visible: opened || fade.running

    property Item anchorItem
    property bool opened: false
    property var notes: ({ version: "", mode: "empty", newCount: 0, groups: [], url: "" })
    // Flat rows for the list: { kind: "head" | "item" | "more", text, first }.
    property var rows: []

    function groupTitle(title) {
        switch (title) {
        case "New": return qsTr("New")
        case "Changed": return qsTr("Changed")
        case "Fixed": return qsTr("Fixed")
        default: return title
        }
    }

    function buildRows(n) {
        const out = []
        const groups = n.groups || []
        for (let g = 0; g < groups.length; g++) {
            const group = groups[g]
            const head = n.mode === "highlights" ? qsTr("In this version") : groupTitle(group.title)
            out.push({ kind: "head", text: head, first: g === 0 })
            for (const item of group.items)
                out.push({ kind: "item", text: item, first: false })
            if (group.more > 0)
                out.push({ kind: "more", text: qsTr("and %1 more").arg(group.more), first: false })
        }
        return out
    }

    function open() {
        try { notes = JSON.parse(AppController.whatsNew()) } catch (e) {}
        AppController.markWhatsNewSeen()
        rows = buildRows(notes)
        list.positionViewAtBeginning()
        opened = true
        card.forceActiveFocus(Qt.PopupFocusReason)
    }

    function close() {
        if (!opened)
            return
        opened = false
        if (anchorItem)
            anchorItem.forceActiveFocus(Qt.PopupFocusReason)
    }

    Shortcut {
        sequence: "Esc"
        enabled: panel.opened
        onActivated: panel.close()
    }

    // Outside the panel: a click closes it, and the wheel and hover stop here
    // instead of reaching the page behind.
    MouseArea {
        anchors.fill: parent
        enabled: panel.opened
        acceptedButtons: Qt.AllButtons
        hoverEnabled: true
        onPressed: panel.close()
        onWheel: (wheel) => wheel.accepted = true
    }

    Rectangle {
        id: card
        readonly property point anchorPos: panel.anchorItem
            ? panel.anchorItem.mapToItem(panel, panel.anchorItem.width, panel.anchorItem.height)
            : Qt.point(panel.width - 8, Theme.topBarHeight)
        width: Math.min(360, panel.width - 16)
        height: Math.min(header.height + list.contentHeight + footer.height + 4, 480, panel.height - y - 12)
        x: Math.max(8, Math.min(anchorPos.x - width, panel.width - width - 8))
        y: Math.max(anchorPos.y + 6, Theme.topBarHeight) + (panel.opened || Theme.reduceMotion ? 0 : -4)
        radius: Theme.radiusLg
        color: Theme.surfaceContainerHigh
        border.width: 1
        border.color: Theme.outlineVariant
        opacity: panel.opened ? 1 : 0
        activeFocusOnTab: true
        Behavior on opacity { NumberAnimation { id: fade; duration: Theme.fadeFast; easing.type: Easing.OutCubic } }
        Behavior on y { enabled: !Theme.reduceMotion; NumberAnimation { duration: Theme.fadeFast; easing.type: Easing.OutCubic } }

        Accessible.role: Accessible.Dialog
        Accessible.name: qsTr("What's new")

        // The card keeps its clicks and wheel from the outside area.
        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onWheel: (wheel) => wheel.accepted = true
        }

        Item {
            id: header
            width: parent.width
            height: 48
            Row {
                anchors.left: parent.left
                anchors.leftMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                Txt {
                    id: titleText
                    text: qsTr("What's new")
                    role: "title"
                    size: 15
                }
                Txt {
                    anchors.baseline: titleText.baseline
                    text: panel.notes.version || ""
                    role: "caption"
                    muted: true
                }
            }
            IconButton {
                anchors.right: parent.right
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                width: 28
                height: 28
                iconPath: Icons.close
                iconSize: 14
                label: qsTr("Close")
                onClicked: panel.close()
            }
        }

        ListView {
            id: list
            anchors.top: header.bottom
            anchors.bottom: footer.top
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 16
            anchors.rightMargin: 16
            clip: true
            model: panel.rows
            boundsBehavior: Flickable.StopAtBounds
            interactive: contentHeight > height
            cacheBuffer: 2000
            delegate: Item {
                id: row
                required property var modelData
                width: ListView.view.width
                height: modelData.kind === "head" ? (modelData.first ? 28 : 36)
                    : modelData.kind === "more" ? 24
                    : itemText.implicitHeight + 6

                Txt {
                    visible: row.modelData.kind === "head"
                    anchors.bottom: parent.bottom
                    anchors.bottomMargin: 4
                    text: row.modelData.kind === "head" ? row.modelData.text : ""
                    role: "label"
                    color: Theme.primary
                }
                Rectangle {
                    visible: row.modelData.kind === "item"
                    x: 0
                    y: 8
                    width: 4
                    height: 4
                    radius: 2
                    color: Theme.surfaceContentVariant
                }
                Txt {
                    id: itemText
                    visible: row.modelData.kind === "item"
                    x: 12
                    width: parent.width - 12
                    text: row.modelData.kind === "item" ? row.modelData.text : ""
                    role: "bodySmall"
                    color: Theme.surfaceContent
                    wrapMode: Text.WordWrap
                    maximumLineCount: 2
                    elide: Text.ElideRight
                }
                Txt {
                    visible: row.modelData.kind === "more"
                    x: 12
                    anchors.verticalCenter: parent.verticalCenter
                    text: row.modelData.kind === "more" ? row.modelData.text : ""
                    role: "caption"
                    muted: true
                }
            }

            Txt {
                visible: panel.rows.length === 0
                y: 4
                width: parent.width
                text: qsTr("Nothing to list for this build.")
                role: "bodySmall"
                muted: true
                wrapMode: Text.WordWrap
            }
        }
        ScrollRail { flick: list; inset: 2 }

        Item {
            id: footer
            anchors.bottom: parent.bottom
            width: parent.width
            height: 40
            Divider { width: parent.width; anchors.top: parent.top }
            Button {
                anchors.left: parent.left
                anchors.leftMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                variant: "text"
                size: "sm"
                iconPath: Icons.openExternal
                text: qsTr("Release notes on GitHub")
                onClicked: {
                    Links.open("releases")
                    panel.close()
                }
            }
        }
    }
}
