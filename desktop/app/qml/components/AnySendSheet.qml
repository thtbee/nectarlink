// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtQuick.Dialogs
import app.nectarlink

Sheet {
    id: sheet
    cardWidth: 568
    onOpenedChanged: {
        if (opened)
            siteInput.text = Preferences.anysendSite
        else
            AnySend.cancel()
    }

    function begin(receive, url) {
        Preferences.anysendSite = siteInput.text.trim()
        AnySend.start(receive, url.toString(), Preferences.anysendSite, Preferences.networkInterface)
    }

    FileDialog {
        id: sourceFile
        title: qsTr("Send a file with AnySend")
        fileMode: FileDialog.OpenFile
        onAccepted: sheet.begin(false, selectedFile)
    }

    FolderDialog {
        id: destinationFolder
        title: qsTr("Choose where to save the received file")
        onAccepted: sheet.begin(true, selectedFolder)
    }

    Column {
        width: parent.width
        spacing: 12

        Txt {
            text: "AnySend"
            role: "headline"
        }
        Txt {
            width: parent.width
            text: qsTr("Exchange one file up to 128 MiB with a browser.")
            wrapMode: Text.WordWrap
            muted: true
        }
        Row {
            spacing: 8
            Button {
                text: qsTr("Send")
                iconPath: Icons.send
                onClicked: sourceFile.open()
            }
            Button {
                text: qsTr("Receive")
                iconPath: Icons.folder
                variant: "tonal"
                onClicked: destinationFolder.open()
            }
        }

        Txt {
            text: qsTr("Website for the global QR")
            role: "label"
        }
        Rectangle {
            width: parent.width
            height: 38
            radius: Theme.radiusSm
            color: Theme.surfaceContainer
            border.width: 1
            border.color: siteInput.activeFocus ? Theme.primary : Theme.outlineVariant
            TextInput {
                id: siteInput
                anchors.fill: parent
                anchors.margins: 8
                verticalAlignment: Text.AlignVCenter
                color: Theme.surfaceContent
                clip: true
                selectByMouse: true
                Accessible.name: qsTr("Website for the global QR")
                onEditingFinished: {
                    if (sheet.opened)
                        Preferences.anysendSite = text.trim()
                }
            }
        }

        Grid {
            id: routes
            visible: AnySend.phase === "waiting"
            width: parent.width
            columns: width >= 400 ? 2 : 1
            spacing: 12

            Repeater {
                model: [
                    { title: qsTr("Local network"), path: AnySend.lanPath, size: AnySend.lanSize, error: AnySend.lanError },
                    { title: qsTr("Global / iroh relay"), path: AnySend.relayPath, size: AnySend.relaySize, error: AnySend.relayError }
                ]
                delegate: Column {
                    required property var modelData
                    width: (routes.width - routes.spacing * (routes.columns - 1)) / routes.columns
                    spacing: 6
                    Txt {
                        width: parent.width
                        horizontalAlignment: Text.AlignHCenter
                        text: modelData.title
                        role: "label"
                    }
                    QrCode {
                        anchors.horizontalCenter: parent.horizontalCenter
                        visible: modelData.path.length > 0
                        width: Math.min(190, parent.width)
                        height: width
                        path: modelData.path
                        modules: Math.max(21, modelData.size)
                        Accessible.name: modelData.title
                    }
                    Txt {
                        width: parent.width
                        horizontalAlignment: Text.AlignHCenter
                        visible: modelData.path.length === 0
                        text: modelData.error || qsTr("Preparing…")
                        wrapMode: Text.WordWrap
                        muted: true
                    }
                }
            }
        }
        Txt {
            visible: AnySend.phase === "waiting"
            width: parent.width
            text: qsTr("Scan either QR within 10 minutes. Use the local QR on a trusted network.")
            wrapMode: Text.WordWrap
            muted: true
        }
        Txt {
            visible: AnySend.phase === "transferring" || AnySend.phase === "finished" ||
                     AnySend.phase === "expired" || AnySend.phase === "failed"
            width: parent.width
            wrapMode: Text.Wrap
            text: AnySend.phase === "transferring"
                  ? qsTr("%1% · %2").arg(Math.round(AnySend.progress * 100)).arg(
                        AnySend.activeRoute === "lan" ? qsTr("Local network") : qsTr("Iroh relay"))
                  : AnySend.phase === "finished"
                      ? (AnySend.savedPath.length ? qsTr("Saved to %1").arg(AnySend.savedPath) : qsTr("Transfer complete"))
                  : AnySend.phase === "expired" ? qsTr("These QR codes have expired.") : AnySend.error
        }
        Row {
            width: parent.width
            layoutDirection: Qt.RightToLeft
            spacing: 8
            Button {
                variant: "tonal"
                text: AnySend.phase === "transferring" ? qsTr("Cancel") : qsTr("Close")
                onClicked: sheet.close()
            }
        }
    }
}
