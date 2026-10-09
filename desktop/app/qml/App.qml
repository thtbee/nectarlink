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
    property var shelfWindow: null
    readonly property Component shelfWindowComponent: Component {
        ShelfWindow {}
    }

    function syncShelfWindow() {
        if (AppController.shelfOpen) {
            trayTrimTimer.stop()
            if (!app.shelfWindow) {
                const w = shelfWindowComponent.createObject(app)
                if (w) {
                    app.shelfWindow = w
                } else {
                    console.warn("ShelfWindow createObject failed:", shelfWindowComponent.errorString())
                }
            }
        } else {
            if (app.shelfWindow) {
                app.shelfWindow.destroy()
                app.shelfWindow = null
            }
            if (!app.mainWindow && Object.keys(app.openMirrors).length === 0)
                trayTrimTimer.restart()
        }
    }

    // A window for each phone screen or app being mirrored. Keyed by
    // `mirrorKey`, so a window lives as long as its mirroring.
    property var openMirrors: ({})
    readonly property Component mirrorWindowComponent: Component {
        MirrorWindow {}
    }

    function syncMirrorWindows() {
        let keys = []
        try {
            keys = JSON.parse(Mirror.windows).map(w => w.key)
        } catch (e) {}
        const current = app.openMirrors
        for (const k of Object.keys(current)) {
            if (keys.indexOf(k) < 0) {
                if (current[k])
                    current[k].destroy()
                delete current[k]
            }
        }
        for (const key of keys) {
            if (!current[key]) {
                const w = mirrorWindowComponent.createObject(app, { mirrorKey: key })
                if (w) {
                    current[key] = w
                } else {
                    console.warn("MirrorWindow createObject failed:", mirrorWindowComponent.errorString())
                }
            }
        }
        if (!app.mainWindow && Object.keys(current).length === 0 && !AppController.shelfOpen)
            trayTrimTimer.restart()
    }
    readonly property bool startMinimized: Qt.application.arguments.indexOf("--minimized") >= 0
        || Qt.application.arguments.indexOf("--send-to") >= 0

    readonly property Timer trayTrimTimer: Timer {
        interval: 350
        onTriggered: {
            if (!app.mainWindow && Object.keys(app.openMirrors).length === 0 && !AppController.shelfOpen) {
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
        Mirror.windowsChanged.connect(app.syncMirrorWindows)
        AppController.shelfOpenChanged.connect(app.syncShelfWindow)
        app.syncMirrorWindows()
        app.syncShelfWindow()
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
        function onShelfOpenChanged() {
            app.syncShelfWindow()
        }
    }
}
