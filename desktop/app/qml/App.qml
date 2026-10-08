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

    property MainWindow mainWindow: null
    property LaserOverlay laserOverlay: null
    readonly property Component laserComponent: Component { LaserOverlay {} }
    // A window for each phone screen or app being mirrored. The model
    // keeps a row per window (by key), so a window lives as long as its
    // mirroring, whatever else changes.
    readonly property ListModel mirrorWindows: ListModel {}
    readonly property Instantiator mirrorInstantiator: Instantiator {
        model: app.mirrorWindows
        delegate: MirrorWindow {
            required property string key
            mirrorKey: key
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
    readonly property Component mainComponent: Component { MainWindow {} }
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
        if (!mainWindow) {
            mainWindow = mainComponent.createObject(null)
            mainWindow.closeRequested.connect(app.onMainClosed)
        }
        mainWindow.bringToFront()
    }

    function onMainClosed() {
        if (!Preferences.closeToTray) {
            Qt.quit()
            return
        }
        // Destroy after the close event has finished.
        const window = mainWindow
        mainWindow = null
        Qt.callLater(() => {
            window.destroy()
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
        function onLaserActiveChanged() {
            if (AppController.laserActive && !app.laserOverlay) {
                app.laserOverlay = app.laserComponent.createObject(null)
            } else if (!AppController.laserActive && app.laserOverlay) {
                const overlay = app.laserOverlay
                app.laserOverlay = null
                Qt.callLater(() => overlay.destroy())
            }
        }
        function onRingingFromChanged() {
            if (AppController.ringingFrom.length > 0) {
                app.showMain()
                app.mainWindow.flash()
            }
        }
        function onRemotePromptDeviceIdChanged() {
            if (AppController.remotePromptDeviceId.length > 0) {
                app.showMain()
                app.mainWindow.flash()
            }
        }
    }
}
