import QtQuick
import app.nectarlink.spike

// Live measurements for the S1 gate: frame rate, events per second,
// memory and startup time.
Rectangle {
    id: panel
    required property DeviceModel model
    required property MirrorPanel mirror
    property real fps: 0
    property bool measuring: false

    width: 262
    height: column.implicitHeight + 24
    radius: Theme.radiusLg
    color: Qt.rgba(0.12, 0.10, 0.08, 0.86)

    FrameAnimation {
        running: panel.measuring
        onTriggered: panel.fps = smoothFrameTime > 0 ? 1.0 / smoothFrameTime : 0
    }
    Timer { interval: 1000; running: true; repeat: true; onTriggered: panel.model.refreshMemory() }

    Column {
        id: column
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: 12 }
        spacing: 6
        Text { text: "S1 · LIVE MEASUREMENTS"; color: "#FFDDB8"; font.family: "Cascadia Mono"; font.pixelSize: 10; font.letterSpacing: 1 }
        Repeater {
            model: [
                ["Frame rate", panel.measuring ? panel.fps.toFixed(0) + " fps" : "idle (no redraws)"],
                ["Core events", panel.model.eventsPerSecond + " / s"],
                ["Events handled", Math.round(panel.model.eventsTotal).toLocaleString(Qt.locale(), "f", 0)],
                ["Memory", panel.model.workingSetMb.toFixed(1) + " MB"],
                ["First frame", panel.model.startupMs > 0 ? panel.model.startupMs.toFixed(0) + " ms" : "…"]
            ]
            delegate: Row {
                required property var modelData
                width: column.width
                Text { width: 116; text: modelData[0]; color: "#C9B9A8"; font.family: "Segoe UI Variable Text"; font.pixelSize: 12 }
                Text { text: modelData[1]; color: "white"; font.family: "Cascadia Mono"; font.pixelSize: 12; font.weight: Font.DemiBold }
            }
        }
        Row {
            spacing: 6
            topPadding: 4
            Repeater {
                model: [["1k/s", 1000], ["10k/s", 10000], ["Stop", 0]]
                delegate: Rectangle {
                    required property var modelData
                    width: 64; height: 26; radius: 13
                    color: tap.pressed ? "#5c4630" : "#3d2f22"
                    Text { anchors.centerIn: parent; text: modelData[0]; color: "white"; font.family: "Segoe UI Variable Text"; font.pixelSize: 12; font.weight: Font.DemiBold }
                    TapHandler {
                        id: tap
                        onTapped: modelData[1] > 0 ? panel.model.startStress(modelData[1]) : panel.model.stopStress()
                    }
                }
            }
        }
        Rectangle {
            width: 200; height: 26; radius: 13
            color: panel.measuring ? "#8A5100" : "#3d2f22"
            Text { anchors.centerIn: parent; text: panel.measuring ? "Measuring frame rate" : "Measure frame rate"; color: "white"; font.family: "Segoe UI Variable Text"; font.pixelSize: 12; font.weight: Font.DemiBold }
            TapHandler { onTapped: panel.measuring = !panel.measuring }
        }
        Rectangle {
            width: 200; height: 26; radius: 13
            color: panel.mirror.running ? "#8A5100" : "#3d2f22"
            Text { anchors.centerIn: parent; text: panel.mirror.running ? "Stop video" : "Video 1080p60 (GPU)"; color: "white"; font.family: "Segoe UI Variable Text"; font.pixelSize: 12; font.weight: Font.DemiBold }
            TapHandler { onTapped: panel.mirror.running = !panel.mirror.running }
        }
    }
}
