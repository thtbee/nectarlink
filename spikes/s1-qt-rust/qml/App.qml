// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink.spike

// Application root. Windows are created on demand and destroyed when closed,
// so the always-running part (core + tray) carries no window or GPU cost.
QtObject {
    id: app

    property Window mainWindow: null
    readonly property Component mainComponent: Component { Main {} }

    function openMain() {
        if (!mainWindow)
            mainWindow = mainComponent.createObject(null)
    }
    function closeMain() {
        if (mainWindow) {
            mainWindow.destroy()
            mainWindow = null
        }
    }

    // --no-window starts straight into tray mode (measures the floor).
    Component.onCompleted: {
        if (Qt.application.arguments.indexOf("--no-window") < 0)
            openMain()
    }

    // --tray-test: open the main window, close it, and report the memory of
    // each state. Measured in-process, so the numbers match Task Manager.
    readonly property bool trayTest: Qt.application.arguments.indexOf("--tray-test") >= 0
    readonly property DeviceModel probe: DeviceModel {}
    property string report: ""

    function sample(label) {
        probe.refreshMemory()
        report += " " + label + "_ws=" + probe.workingSetMb.toFixed(1)
                + " " + label + "_private=" + probe.privateMb.toFixed(1)
    }

    readonly property Timer trayStep1: Timer {
        running: app.trayTest; interval: 3000
        onTriggered: { app.sample("window"); app.closeMain(); app.trayStep2.start() }
    }
    readonly property Timer trayStep2: Timer {
        interval: 3000
        onTriggered: { MemoryTools.trimEngineCaches(); app.sample("tray"); app.trayStep3.start() }
    }
    readonly property Timer trayStep3: Timer {
        interval: 1000
        onTriggered: {
            app.probe.trimWorkingSet()
            app.sample("tray_trimmed")
            console.log("TRAYTEST" + app.report)
            Qt.quit()
        }
    }
}
