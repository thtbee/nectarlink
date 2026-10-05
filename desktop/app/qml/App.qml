// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Application root. The main window exists only while it's open: closing it
// destroys it (freeing its GPU resources) and Nectarlink keeps running in
// the tray. A tray click, a second launch or a ringing request reopens it.
QtObject {
    id: app

    property MainWindow mainWindow: null
    readonly property Component mainComponent: Component { MainWindow {} }
    readonly property bool startMinimized: Qt.application.arguments.indexOf("--minimized") >= 0
        || Qt.application.arguments.indexOf("--send-to") >= 0

    function showMain() {
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
        Qt.callLater(() => window.destroy())
    }

    Component.onCompleted: if (!startMinimized) showMain()

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
                app.mainWindow.flash()
            }
        }
    }
}
