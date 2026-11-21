// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Apps: every app on the phone, to open one in a window of its own on this PC
// (Elevated: the phone runs it on a display of its own). Searchable; Enter
// opens the first match. Apps already open here are marked.
Item {
    id: page
    property bool active: false
    property string deviceId
    property string deviceName

    readonly property var feature: AppController.capsRevision >= 0 && deviceId.length > 0
        ? AppController.featureState(deviceId, "mirroring.app_windows") : ({})
    readonly property bool available: feature.state === "available" || feature.state === "partial"

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
    // Packages with a window open on this PC.
    readonly property var openPkgs: {
        try {
            const open = {}
            JSON.parse(Mirror.windows).forEach(w => {
                if (w.device === page.deviceId && w.app && w.pkg && w.phase !== "ended")
                    open[w.pkg] = true
            })
            return open
        } catch (e) {
            return ({})
        }
    }
    // Apps pinned to the top (#477), in the order they were pinned.
    readonly property var pinnedPkgs: {
        Preferences.pinsRevision
        try { return JSON.parse(Preferences.pinnedApps(deviceId)) } catch (e) { return [] }
    }
    readonly property var pinned: pinnedPkgs.map(pkg => apps.find(a => a.pkg === pkg)).filter(a => a !== undefined)
    function isPinned(app) { return app && pinnedPkgs.indexOf(app.pkg) >= 0 }
    readonly property string query: search.text.trim().toLocaleLowerCase()
    readonly property var shown: query.length === 0 ? apps
        : apps.filter(a => a.label.toLocaleLowerCase().indexOf(query) >= 0 || a.pkg.indexOf(query) >= 0)
    readonly property string loadState: Mirror.appsDevice === deviceId ? Mirror.appsState : ""

    function launch(app) {
        Mirror.startApp(deviceId, app.pkg, app.label)
    }

    // An app's other actions: right-click, the Menu key, or a long press.
    property var actionApp: null
    property bool confirmUninstall: false
    readonly property var uninstallFeature: AppController.capsRevision >= 0 && deviceId.length > 0
        ? AppController.featureState(deviceId, "mirroring.app_uninstall") : ({})
    readonly property bool canUninstall: uninstallFeature.state === "available"
    function showActions(app) {
        actionApp = app
        confirmUninstall = false
        appActions.open()
    }

    // Loads the list the first time the page shows for a phone; Refresh reloads it.
    function loadIfNeeded() {
        if (!active || !available || deviceId.length === 0)
            return
        if (Mirror.appsDevice !== deviceId || Mirror.appsState === "" || Mirror.appsState === "failed")
            Mirror.loadApps(deviceId)
    }
    onActiveChanged: if (active) { loadIfNeeded(); search.forceActiveFocus() }
    onDeviceIdChanged: { search.text = ""; loadIfNeeded() }
    onAvailableChanged: loadIfNeeded()
    Component.onCompleted: loadIfNeeded()

    // ---- No phone, or opening apps isn't available yet ----
    Column {
        anchors.centerIn: parent
        width: Math.min(parent.width - Theme.contentPadding * 2, 420)
        spacing: 12
        visible: page.deviceId.length === 0 || !page.available
        Icon {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 40; height: 40
            stroke: 1.5
            path: Icons.apps
            color: Theme.surfaceContentVariant
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            role: "title"
            text: page.deviceId.length === 0 ? qsTr("Phone apps")
                : qsTr("Open %1's apps here").arg(page.deviceName)
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            role: "body"
            muted: true
            text: page.deviceId.length === 0
                ? qsTr("Pair your phone to open its apps in windows on this PC.")
                : page.feature.reason === "notOnThisDevice"
                    ? qsTr("Opening apps in windows isn't supported on this phone.")
                    : String(page.feature.reason || "").startsWith("android:")
                        ? qsTr("Opening apps in windows needs Android %1 or newer on the phone.").arg(page.feature.reason.slice(8))
                        : (page.feature.reason || qsTr("Set up Wireless debugging in NectarLink on the phone to open its apps here."))
        }
        LockChip {
            anchors.horizontalCenter: parent.horizontalCenter
            visible: page.deviceId.length > 0 && label.length > 0
            feature: page.feature
        }
    }

    // ---- The apps ----
    Item {
        id: content
        visible: page.deviceId.length > 0 && page.available
        anchors.fill: parent
        anchors.leftMargin: Theme.contentPadding
        anchors.rightMargin: Theme.contentPadding
        anchors.topMargin: 8

        Row {
            id: toolbar
            width: parent.width
            height: 40
            spacing: 8

            Rectangle {
                width: parent.width - refresh.width - parent.spacing
                height: parent.height
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
                    Keys.onReturnPressed: if (page.shown.length > 0) page.launch(page.shown[0])
                    Keys.onEnterPressed: if (page.shown.length > 0) page.launch(page.shown[0])
                    Keys.onEscapePressed: text = ""
                    Txt {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: search.text.length === 0
                        role: "body"
                        muted: true
                        text: qsTr("Search %1's apps").arg(page.deviceName)
                    }
                }
            }
            IconButton {
                id: refresh
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.refresh
                label: qsTr("Refresh")
                enabled: page.loadState !== "loading"
                onClicked: Mirror.loadApps(page.deviceId)
            }
        }

        // Loading, failed or empty.
        Column {
            anchors.top: toolbar.bottom
            anchors.topMargin: 48
            width: parent.width
            visible: page.loadState !== "ready" || page.shown.length === 0
            spacing: 12
            Spinner {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.loadState === "loading" || page.loadState === ""
                width: 24; height: 24
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                role: "body"
                muted: true
                text: page.loadState === "failed" ? Mirror.appsError
                    : page.loadState === "ready" ? (page.query.length > 0 ? qsTr("No apps match.") : qsTr("No apps to open."))
                    : qsTr("Getting the phone's apps…")
            }
            Button {
                anchors.horizontalCenter: parent.horizontalCenter
                visible: page.loadState === "failed"
                variant: "tonal"
                size: "sm"
                text: qsTr("Try again")
                onClicked: Mirror.loadApps(page.deviceId)
            }
        }

        ScrollRail { flick: grid }
        GridView {
            id: grid
            anchors.top: toolbar.bottom
            anchors.topMargin: 12
            anchors.bottom: parent.bottom
            width: parent.width
            visible: page.loadState === "ready" && page.shown.length > 0
            clip: true
            model: page.shown
            cellWidth: Math.floor(width / Math.max(1, Math.floor(width / 120)))
            cellHeight: 116
            boundsBehavior: Flickable.StopAtBounds
            reuseItems: true
            cacheBuffer: 400

            header: Column {
                width: grid.width
                visible: page.query.length === 0 && (page.recent.length > 0 || page.pinned.length > 0)
                height: visible ? implicitHeight : 0
                spacing: 8
                bottomPadding: 8

                Txt {
                    visible: page.pinned.length > 0
                    role: "label"
                    muted: true
                    text: qsTr("Pinned")
                }
                Flow {
                    visible: page.pinned.length > 0
                    width: parent.width
                    Repeater {
                        model: page.pinned
                        delegate: AppCell {
                            required property var modelData
                            app: modelData
                            width: grid.cellWidth
                        }
                    }
                }
                Divider { visible: page.pinned.length > 0 && page.recent.length > 0 }
                Txt {
                    visible: page.recent.length > 0
                    role: "label"
                    muted: true
                    text: qsTr("Recent")
                }
                Row {
                    visible: page.recent.length > 0
                    width: parent.width
                    clip: true
                    spacing: 0
                    Repeater {
                        model: page.recent
                        delegate: AppCell {
                            required property var modelData
                            app: modelData
                            width: grid.cellWidth
                        }
                    }
                }
                Divider {}
                Txt {
                    topPadding: 4
                    role: "label"
                    muted: true
                    text: qsTr("All apps")
                }
            }

            delegate: AppCell {
                required property var modelData
                app: modelData
                width: grid.cellWidth
            }
        }
    }

    component AppCell: Item {
        id: cell
        property var app: ({})
        readonly property bool isOpen: page.openPkgs[app.pkg] === true
        height: grid.cellHeight
        activeFocusOnTab: true
        Accessible.role: Accessible.Button
        Accessible.name: isOpen ? qsTr("%1, open").arg(app.label) : app.label
        Accessible.onPressAction: page.launch(app)
        Keys.onReturnPressed: page.launch(app)
        Keys.onEnterPressed: page.launch(app)
        Keys.onSpacePressed: page.launch(app)

        Rectangle {
            anchors.fill: parent
            anchors.margins: 4
            radius: Theme.radiusMd
            color: Theme.surfaceContent
            opacity: tap.pressed ? 0.12 : (hover.hovered ? 0.06 : 0)
        }
        Rectangle {
            anchors.fill: parent
            anchors.margins: 4
            radius: Theme.radiusMd
            color: "transparent"
            border.width: Theme.focusVisible(cell) ? 2 : 0
            border.color: Theme.primary
        }
        Column {
            anchors.centerIn: parent
            width: parent.width - 12
            spacing: 8
            Item {
                anchors.horizontalCenter: parent.horizontalCenter
                width: 48; height: 48
                Image {
                    id: icon
                    anchors.fill: parent
                    source: cell.app.icon || ""
                    sourceSize: Qt.size(96, 96)
                    asynchronous: true
                    smooth: true
                    mipmap: true
                    visible: status === Image.Ready
                }
                Avatar {
                    anchors.fill: parent
                    visible: icon.status !== Image.Ready
                    name: cell.app.label || ""
                }
                // Open in a window on this PC.
                Rectangle {
                    visible: cell.isOpen
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.top: parent.bottom
                    anchors.topMargin: 3
                    width: 6; height: 6
                    radius: 3
                    color: Theme.primary
                }
            }
            Txt {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                role: "bodySmall"
                text: cell.app.label || ""
                elide: Text.ElideRight
                maximumLineCount: 2
                wrapMode: Text.WordWrap
            }
        }
        HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
        Keys.onMenuPressed: page.showActions(app)
        TapHandler { id: tap; onTapped: page.launch(cell.app); onLongPressed: page.showActions(cell.app) }
        TapHandler { acceptedButtons: Qt.RightButton; onTapped: page.showActions(cell.app) }
    }

    Sheet {
        id: appActions
        cardWidth: 380
        Column {
            width: parent.width
            spacing: 10

            Txt {
                width: parent.width
                role: "headline"
                text: page.actionApp ? page.actionApp.label : ""
                elide: Text.ElideRight
            }
            Button {
                width: parent.width
                variant: "tonal"
                iconPath: Icons.window
                text: qsTr("Open in a window")
                onClicked: {
                    appActions.close()
                    page.launch(page.actionApp)
                }
            }
            Button {
                width: parent.width
                variant: "tonal"
                iconPath: Icons.pin
                text: page.isPinned(page.actionApp) ? qsTr("Unpin") : qsTr("Pin to the top")
                onClicked: {
                    const app = page.actionApp
                    appActions.close()
                    Preferences.setAppPinned(page.deviceId, app.pkg, !page.isPinned(app))
                }
            }
            // A shortcut that opens it straight in a window (#370).
            Button {
                width: parent.width
                visible: Mirror.appShortcutsSupported()
                variant: "tonal"
                iconPath: Icons.pin
                text: Qt.platform.os === "windows" ? qsTr("Add to desktop") : qsTr("Add to app menu")
                onClicked: {
                    appActions.close()
                    Mirror.addAppShortcut(page.deviceId, page.actionApp.pkg, page.actionApp.label)
                }
            }
            // Through Elevated, after a second press confirms it (D349).
            Button {
                width: parent.width
                visible: page.canUninstall
                variant: page.confirmUninstall ? "fill" : "outline"
                iconPath: Icons.trash
                text: page.confirmUninstall
                    ? qsTr("Uninstall it from %1").arg(page.deviceName)
                    : qsTr("Uninstall")
                onClicked: {
                    if (!page.confirmUninstall) {
                        page.confirmUninstall = true
                        return
                    }
                    appActions.close()
                    Mirror.uninstallApp(page.deviceId, page.actionApp.pkg, page.actionApp.label)
                }
            }
        }
    }
}
