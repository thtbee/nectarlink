// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQml
import QtQml.Models
import app.nectarlink

// Application root. The main window exists only while it's open: closing it
// destroys it (freeing its GPU resources) and Nectarlink keeps running in
// the tray. A tray click, a second launch or a ringing request reopens it.
QtObject {
    id: app

    readonly property var mainWindow: mainLoader.item
    readonly property Loader mainLoader: Loader {
        onLoaded: {
            item.closeRequested.connect(app.onMainClosed)
            item.bringToFront()
        }
    }
    readonly property Loader laserLoader: Loader {
        active: AppController.laserActive
        source: active ? "qrc:/qt/qml/app/nectarlink/qml/LaserOverlay.qml" : ""
    }
    // A window for each phone screen or app being mirrored. The model
    // keeps a row per window (by key), so a window lives as long as its
    // mirroring, whatever else changes.
    readonly property ListModel mirrorWindows: ListModel {}
    readonly property Instantiator mirrorInstantiator: Instantiator {
        model: app.mirrorWindows
        delegate: Loader {
            required property string key
            Component.onCompleted: setSource("qrc:/qt/qml/app/nectarlink/qml/MirrorWindow.qml", { mirrorKey: key })
        }
    }
    readonly property Connections mirrorSync: Connections {
        target: Mirror
        function onWindowsChanged() { app.syncMirrorWindows() }
    }

    function syncMirrorWindows() {
        let keys = []
        try {
            keys = JSON.parse(Mirror.windows).map(w => w.key)
        } catch (e) {}
        for (let i = mirrorWindows.count - 1; i >= 0; i--) {
            if (keys.indexOf(mirrorWindows.get(i).key) < 0)
                mirrorWindows.remove(i)
        }
        for (const key of keys) {
            let known = false
            for (let i = 0; i < mirrorWindows.count; i++)
                known = known || mirrorWindows.get(i).key === key
            if (!known)
                mirrorWindows.append({ key: key })
        }
        if (!app.mainWindow && mirrorWindows.count === 0)
            trayTrimTimer.restart()
    }
    readonly property bool startMinimized: Qt.application.arguments.indexOf("--minimized") >= 0
        || Qt.application.arguments.indexOf("--send-to") >= 0

    readonly property Timer trayTrimTimer: Timer {
        interval: 350
        onTriggered: {
            if (!app.mainWindow && app.mirrorWindows.count === 0) {
                gc()
                AppController.trimWorkingSet()
            }
        }
    }

    function showMain() {
        trayTrimTimer.stop()
        if (!mainLoader.item) {
            mainLoader.setSource("qrc:/qt/qml/app/nectarlink/qml/MainWindow.qml")
        } else {
            mainLoader.item.bringToFront()
        }
    }

    function onMainClosed() {
        if (!Preferences.closeToTray) {
            Qt.quit()
            return
        }
        // Unload after the close event has finished.
        Qt.callLater(() => {
            if (mainLoader.item)
                mainLoader.item.releaseAndTeardown()
            mainLoader.setSource("")
            trayTrimTimer.restart()
        })
    }

    Component.onCompleted: {
        if (!startMinimized)
            showMain()
        else
            trayTrimTimer.restart()
    }

    // Qt follows the system color scheme unless told otherwise; the window
    // frame and Mica tint must follow the app's theme instead.
    readonly property Binding colorScheme: Binding {
        target: Qt.styleHints
        property: "colorScheme"
        value: Theme.dark ? Qt.ColorScheme.Dark : Qt.ColorScheme.Light
    }

    readonly property Connections controller: Connections {
        target: AppController
        function onActivateRequested() { app.showMain() }
        function onQuitRequested() { Qt.quit() }
        function onRingingFromChanged() {
            if (AppController.ringingFrom.length > 0) {
                app.showMain()
                if (app.mainWindow)
                    app.mainWindow.flash()
            }
        }
        function onRemotePromptDeviceIdChanged() {
            if (AppController.remotePromptDeviceId.length > 0) {
                app.showMain()
                if (app.mainWindow)
                    app.mainWindow.flash()
            }
        }
    }
}
