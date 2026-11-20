// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Report a problem: what went wrong in the user's words, then the exact
// report to read before it goes anywhere. Send hands it to the report
// service, which files it as an issue; Open on GitHub fills in a new issue
// and puts the report on the clipboard to paste into it.
Sheet {
    id: sheet
    cardWidth: 600

    property int step: 0
    property string area: "other"
    // Parts to include: 1 about the devices, 2 checks, 4 feature health, 8 crashes, 16 log.
    property int parts: 31
    property bool copied: false

    readonly property var areas: [
        { id: "connection", label: qsTr("Connection") },
        { id: "app-windows", label: qsTr("Mirroring and app windows") },
        { id: "clipboard", label: qsTr("Clipboard") },
        { id: "audio", label: qsTr("Audio") },
        { id: "calls", label: qsTr("Calls and messages") },
        { id: "files", label: qsTr("Files and photos") },
        { id: "notifications", label: qsTr("Notifications") },
        { id: "other", label: qsTr("Something else") }
    ]
    readonly property bool ready: !Report.busy && Report.preview.length > 0
    readonly property bool contactWarning: Report.looksLikeContact(description.text) || Report.looksLikeContact(otherArea.text)

    function reset() {
        step = 0
        area = Report.guessArea()
        parts = 31
        copied = false
        description.text = ""
        otherArea.text = ""
        justNow.checked = true
    }
    function makeReport() {
        Report.prepare(area, otherArea.text, description.text, justNow.checked, parts)
    }
    function togglePart(bit) {
        parts = parts ^ bit
        makeReport()
    }

    onOpenedChanged: if (opened) reset()

    Column {
        width: parent.width
        spacing: 16

        Item {
            width: parent.width
            height: Math.max(heading.height, busy.height)
            Txt {
                id: heading
                anchors.verticalCenter: parent.verticalCenter
                text: sheet.step === 0 ? qsTr("Report a problem") : qsTr("Your report")
                role: "headline"
            }
            Spinner {
                id: busy
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                visible: Report.busy
            }
        }

        // Step 1: what went wrong.
        Column {
            width: parent.width
            spacing: 14
            visible: sheet.step === 0

            Txt { role: "label"; text: qsTr("What is it about?") }
            Flow {
                width: parent.width
                spacing: 8
                Repeater {
                    model: sheet.areas
                    delegate: Chip {
                        required property var modelData
                        interactive: true
                        text: modelData.label
                        selected: sheet.area === modelData.id
                        onClicked: sheet.area = modelData.id
                    }
                }
            }
            Rectangle {
                visible: sheet.area === "other"
                width: parent.width
                height: 40
                radius: Theme.radiusMd
                color: Theme.surfaceContainerHighest
                border.width: otherArea.activeFocus ? 2 : 1
                border.color: otherArea.activeFocus ? Theme.primary : Theme.outlineVariant
                Txt {
                    anchors.fill: parent
                    anchors.margins: 10
                    text: qsTr("What part of NectarLink?")
                    muted: true
                    visible: otherArea.text.length === 0
                }
                TextInput {
                    id: otherArea
                    anchors.fill: parent
                    anchors.margins: 10
                    color: Theme.surfaceContent
                    font.pixelSize: 14
                    maximumLength: 60
                    selectByMouse: true
                    clip: true
                    Accessible.name: qsTr("What part of NectarLink?")
                }
            }

            Txt { role: "label"; text: qsTr("What happened?") }
            Rectangle {
                width: parent.width
                height: 140
                radius: Theme.radiusMd
                color: Theme.surfaceContainerHighest
                border.width: description.activeFocus ? 2 : 1
                border.color: description.activeFocus ? Theme.primary : Theme.outlineVariant
                Flickable {
                    id: descriptionScroll
                    anchors.fill: parent
                    anchors.margins: 10
                    contentWidth: width
                    contentHeight: description.contentHeight
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    Txt {
                        width: parent.width
                        text: qsTr("What you did, what you expected, and what happened instead.")
                        muted: true
                        wrapMode: Text.WordWrap
                        visible: description.text.length === 0
                    }
                    TextEdit {
                        id: description
                        width: descriptionScroll.width
                        color: Theme.surfaceContent
                        font.pixelSize: 14
                        wrapMode: TextEdit.Wrap
                        selectByMouse: true
                        Accessible.name: qsTr("What happened?")
                        onTextChanged: if (text.length > 4000) remove(4000, text.length)
                    }
                }
            }

            Row {
                spacing: 12
                Toggle {
                    id: justNow
                    anchors.verticalCenter: parent.verticalCenter
                    checked: true
                    label: qsTr("It happened just now")
                    onToggled: (on) => justNow.checked = on
                }
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("It happened just now")
                    role: "body"
                }
            }

            Row {
                anchors.right: parent.right
                spacing: 10
                Button {
                    variant: "text"
                    text: qsTr("Check the connection first")
                    onClicked: {
                        sheet.close()
                        AppController.runDoctor()
                    }
                }
                Button {
                    text: qsTr("Next")
                    enabled: description.text.trim().length > 0 || sheet.area !== "other"
                    onClicked: {
                        sheet.step = 1
                        sheet.makeReport()
                    }
                }
            }
        }

        // Step 2: the exact report.
        Column {
            width: parent.width
            spacing: 12
            visible: sheet.step === 1

            Txt {
                width: parent.width
                wrapMode: Text.WordWrap
                role: "body"
                text: qsTr("This is everything in the report. It will be public on GitHub. It has no names, messages, files, numbers or addresses from NectarLink, only what you wrote above.")
            }
            Txt {
                width: parent.width
                visible: sheet.contactWarning
                wrapMode: Text.WordWrap
                role: "bodySmall"
                color: Theme.warning
                text: qsTr("What you wrote looks like it has an email address or phone number in it. Reports are public, so go back and remove it unless you mean to share it.")
            }
            Flow {
                width: parent.width
                spacing: 8
                Repeater {
                    model: [
                        { bit: 1, label: qsTr("About the devices") },
                        { bit: 2, label: qsTr("Checks") },
                        { bit: 4, label: qsTr("How features have been doing") },
                        { bit: 8, label: qsTr("Crashes") },
                        { bit: 16, label: qsTr("Activity log") }
                    ]
                    delegate: Chip {
                        required property var modelData
                        interactive: true
                        iconPath: (sheet.parts & modelData.bit) ? Icons.check : ""
                        text: modelData.label
                        selected: (sheet.parts & modelData.bit) !== 0
                        onClicked: sheet.togglePart(modelData.bit)
                        Accessible.role: Accessible.CheckBox
                        Accessible.checkable: true
                        Accessible.checked: selected
                    }
                }
            }
            Rectangle {
                width: parent.width
                height: 300
                radius: Theme.radiusMd
                color: Theme.surfaceContainerHighest
                border.width: 1
                border.color: Theme.outlineVariant
                Flickable {
                    id: previewScroll
                    anchors.fill: parent
                    anchors.margins: 10
                    contentWidth: width
                    contentHeight: preview.contentHeight
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    TextEdit {
                        id: preview
                        width: previewScroll.width
                        readOnly: true
                        selectByMouse: true
                        wrapMode: TextEdit.WrapAnywhere
                        color: Theme.surfaceContent
                        font.family: "Consolas, Menlo, monospace"
                        font.pixelSize: 12
                        text: Report.preview
                        opacity: Report.busy ? 0.55 : 1
                        Accessible.name: qsTr("Your report")
                    }
                }
            }
            Txt {
                width: parent.width
                visible: sheet.copied
                wrapMode: Text.WordWrap
                role: "bodySmall"
                muted: true
                text: qsTr("The report is copied. On the GitHub page, paste it into the Report box, then submit.")
            }
            Txt {
                width: parent.width
                visible: Report.phoneLogWaiting && (sheet.parts & 16) !== 0
                wrapMode: Text.WordWrap
                role: "bodySmall"
                muted: true
                text: qsTr("Your phone is asking whether to share its log. Tap Share on the phone to add it.")
            }
            Item {
                width: parent.width
                height: Math.max(sendStatus.height, viewSent.visible ? viewSent.height : 0)
                visible: Report.sendState !== ""
                Txt {
                    id: sendStatus
                    anchors.left: parent.left
                    anchors.right: viewSent.visible ? viewSent.left : parent.right
                    anchors.rightMargin: viewSent.visible ? 10 : 0
                    anchors.verticalCenter: parent.verticalCenter
                    wrapMode: Text.WordWrap
                    role: "bodySmall"
                    color: ["limited", "failed"].indexOf(Report.sendState) >= 0 ? Theme.warning : Theme.surfaceContent
                    text: {
                        switch (Report.sendState) {
                        case "sending": return qsTr("Sending the report")
                        case "filed": return qsTr("Sent. This is report #%1.").arg(Report.sentIssue)
                        case "queued": return qsTr("Sent. It will be filed shortly.")
                        case "later": return qsTr("NectarLink couldn't reach the report service. It will send the report later by itself.")
                        case "limited": return qsTr("Too many reports came from this network today. Use Open on GitHub, or send it tomorrow.")
                        case "failed": return qsTr("The report couldn't be sent. Use Open on GitHub instead.")
                        }
                        return ""
                    }
                }
                Button {
                    id: viewSent
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    visible: Report.sendState === "filed" && Report.sentIssue > 0
                    variant: "text"
                    text: qsTr("View it")
                    onClicked: Report.openSentIssue()
                }
            }
            Row {
                anchors.right: parent.right
                spacing: 10
                Button {
                    variant: "text"
                    text: qsTr("Back")
                    onClicked: sheet.step = 0
                }
                Button {
                    variant: "text"
                    iconPath: Icons.copy
                    text: qsTr("Copy")
                    enabled: sheet.ready
                    onClicked: {
                        AppController.copyText(Report.preview)
                        sheet.copied = true
                    }
                }
                Button {
                    variant: "tonal"
                    text: qsTr("Open on GitHub")
                    enabled: sheet.ready
                    onClicked: {
                        AppController.copyText(Report.preview)
                        sheet.copied = Report.openOnGithub()
                    }
                }
                Button {
                    objectName: "reportSend"
                    text: qsTr("Send")
                    enabled: sheet.ready && ["", "failed", "limited"].indexOf(Report.sendState) >= 0
                    onClicked: Report.send()
                }
            }
        }
    }
}
