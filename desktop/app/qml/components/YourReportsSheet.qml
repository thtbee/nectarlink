// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Your reports: the problem reports this PC sent, with what was answered on
// them, and a field to reply from here, so people without a GitHub account
// can talk it through too. Answers are read from the issue on GitHub;
// replies go through the report service, which posts them there.
Sheet {
    id: sheet
    cardWidth: 620

    // The report a reply is being written to, for its status line.
    property string replyRid: ""
    readonly property var conversations: {
        let list = []
        try { list = JSON.parse(Report.conversations || "[]") } catch (e) { list = [] }
        // The one a notification was about, first.
        const focus = Report.focusRid
        if (focus.length > 0)
            list.sort((a, b) => (b.rid === focus) - (a.rid === focus))
        return list
    }

    function whoLabel(answer) {
        switch (answer.who) {
        case "team": return qsTr("NectarLink")
        case "you": return qsTr("You")
        case "other": return qsTr("Someone with the same problem")
        default: return answer.login
        }
    }
    function when(iso) {
        const d = new Date(iso)
        return isNaN(d.getTime()) ? "" : d.toLocaleString(Qt.locale(), Locale.ShortFormat)
    }
    function sentOn(secs) {
        return new Date(secs * 1000).toLocaleDateString(Qt.locale(), Locale.ShortFormat)
    }

    onOpenedChanged: if (opened) { replyRid = ""; Report.loadConversations() }

    Column {
        width: parent.width
        spacing: 14

        Item {
            width: parent.width
            height: Math.max(heading.height, refresh.height)
            Txt {
                id: heading
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Your reports")
                role: "headline"
            }
            Spinner {
                anchors.right: refresh.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                visible: Report.conversationsBusy
            }
            IconButton {
                id: refresh
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.refresh
                label: qsTr("Check for answers")
                enabled: !Report.conversationsBusy
                onClicked: Report.loadConversations()
            }
        }

        Txt {
            width: parent.width
            wrapMode: Text.WordWrap
            muted: true
            text: qsTr("Reports sent from NectarLink stay here for 30 days, with the answers to them. You can reply from here without a GitHub account. Replies are public on GitHub, like the report.")
        }

        Txt {
            visible: sheet.conversations.length === 0
            width: parent.width
            wrapMode: Text.WordWrap
            text: qsTr("No reports sent from this PC yet.")
        }

        Repeater {
            model: sheet.conversations
            delegate: Rectangle {
                id: conv
                required property var modelData
                readonly property bool replying: sheet.replyRid === modelData.rid
                width: parent.width
                height: convCol.implicitHeight + 28
                radius: Theme.radiusMd
                color: Theme.surfaceContainerHigh
                border.width: modelData.rid === Report.focusRid ? 2 : 0
                border.color: Theme.primary

                Column {
                    id: convCol
                    x: 14
                    y: 14
                    width: parent.width - 28
                    spacing: 10

                    Item {
                        width: parent.width
                        height: Math.max(convTitle.height, onGithub.height)
                        Column {
                            id: convTitle
                            anchors.verticalCenter: parent.verticalCenter
                            width: parent.width - onGithub.width - 8
                            spacing: 2
                            Txt {
                                role: "title"
                                text: conv.modelData.issue > 0
                                    ? (conv.modelData.closed ? qsTr("Report #%1 · closed") : qsTr("Report #%1")).arg(conv.modelData.issue)
                                    : qsTr("Waiting to be filed")
                            }
                            Txt {
                                role: "bodySmall"
                                muted: true
                                text: qsTr("Sent %1").arg(sheet.sentOn(conv.modelData.at))
                            }
                        }
                        Button {
                            id: onGithub
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            visible: conv.modelData.issue > 0
                            variant: "text"
                            size: "sm"
                            iconPath: Icons.openExternal
                            text: qsTr("On GitHub")
                            onClicked: Links.openIssue(conv.modelData.issue)
                        }
                    }

                    Txt {
                        visible: conv.modelData.issue > 0 && conv.modelData.loaded && conv.modelData.answers.length === 0
                        width: parent.width
                        wrapMode: Text.WordWrap
                        muted: true
                        text: qsTr("No answers yet. You'll get a notification when there is one.")
                    }
                    Txt {
                        visible: conv.modelData.issue > 0 && !conv.modelData.loaded && !Report.conversationsBusy
                        width: parent.width
                        wrapMode: Text.WordWrap
                        muted: true
                        text: qsTr("The answers couldn't be loaded. Check the connection and try again.")
                    }

                    Repeater {
                        model: conv.modelData.answers
                        delegate: Rectangle {
                            id: bubble
                            required property var modelData
                            readonly property bool mine: modelData.who === "you"
                            width: parent.width
                            height: bubbleCol.implicitHeight + 20
                            radius: Theme.radiusMd
                            color: mine ? Theme.secondaryContainer : Theme.surfaceContainerHighest
                            Column {
                                id: bubbleCol
                                x: 12
                                y: 10
                                width: parent.width - 24
                                spacing: 4
                                Txt {
                                    width: parent.width
                                    elide: Text.ElideRight
                                    role: "label"
                                    text: sheet.whoLabel(bubble.modelData) + "  ·  " + sheet.when(bubble.modelData.at)
                                }
                                TextEdit {
                                    width: parent.width
                                    readOnly: true
                                    selectByMouse: true
                                    wrapMode: TextEdit.Wrap
                                    textFormat: TextEdit.PlainText
                                    color: bubble.mine ? Theme.secondaryContainerContent : Theme.surfaceContent
                                    font.pixelSize: 14
                                    text: bubble.modelData.text
                                    Accessible.name: sheet.whoLabel(bubble.modelData)
                                }
                            }
                        }
                    }

                    Txt {
                        visible: conv.modelData.issue > 0 && !conv.modelData.canReply
                        width: parent.width
                        wrapMode: Text.WordWrap
                        role: "bodySmall"
                        muted: true
                        text: qsTr("This report was sent before replies from NectarLink, so answer it on GitHub.")
                    }

                    // The reply field.
                    Column {
                        visible: conv.modelData.canReply
                        width: parent.width
                        spacing: 8
                        Rectangle {
                            width: parent.width
                            height: 84
                            radius: Theme.radiusMd
                            color: Theme.surfaceContainerHighest
                            border.width: replyText.activeFocus ? 2 : 1
                            border.color: replyText.activeFocus ? Theme.primary : Theme.outlineVariant
                            Flickable {
                                id: replyScroll
                                anchors.fill: parent
                                anchors.margins: 10
                                contentWidth: width
                                contentHeight: replyText.contentHeight
                                clip: true
                                boundsBehavior: Flickable.StopAtBounds
                                Txt {
                                    width: parent.width
                                    text: qsTr("Reply")
                                    muted: true
                                    visible: replyText.text.length === 0
                                }
                                TextEdit {
                                    id: replyText
                                    width: replyScroll.width
                                    color: Theme.surfaceContent
                                    font.pixelSize: 14
                                    wrapMode: TextEdit.Wrap
                                    selectByMouse: true
                                    Accessible.name: qsTr("Reply to report %1").arg(conv.modelData.issue)
                                    onTextChanged: if (text.length > 2000) remove(2000, text.length)
                                }
                            }
                        }
                        Item {
                            width: parent.width
                            height: sendReply.height
                            Txt {
                                anchors.left: parent.left
                                anchors.right: sendReply.left
                                anchors.rightMargin: 10
                                anchors.verticalCenter: parent.verticalCenter
                                wrapMode: Text.WordWrap
                                role: "bodySmall"
                                muted: Report.replyState !== "limited" && Report.replyState !== "later" && Report.replyState !== "refused"
                                color: muted ? Theme.surfaceContentVariant : Theme.error
                                visible: conv.replying && Report.replyState.length > 0
                                text: {
                                    switch (Report.replyState) {
                                    case "sending": return qsTr("Sending")
                                    case "sent": return qsTr("Sent. It's on the report now.")
                                    case "limited": return qsTr("That's the most replies for today. Try again tomorrow.")
                                    case "later": return qsTr("It couldn't be sent. Check the connection and try again.")
                                    case "refused": return qsTr("This report can't take replies any more.")
                                    }
                                    return ""
                                }
                            }
                            Button {
                                id: sendReply
                                anchors.right: parent.right
                                text: qsTr("Send")
                                enabled: replyText.text.trim().length > 0 && !(conv.replying && Report.replyState === "sending")
                                onClicked: {
                                    sheet.replyRid = conv.modelData.rid
                                    Report.sendReply(conv.modelData.rid, replyText.text)
                                }
                            }
                        }
                        Connections {
                            target: Report
                            function onReplyStateChanged() {
                                if (conv.replying && Report.replyState === "sent")
                                    replyText.text = ""
                            }
                        }
                    }
                }
            }
        }

        Row {
            anchors.right: parent.right
            Button {
                variant: "text"
                text: qsTr("Close")
                onClicked: sheet.close()
            }
        }
    }
}
