// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import app.nectarlink

// Calls: recent calls and contacts on the left, the active call and a
// keypad dialer on the right. Audio stays on the phone; calls and texts
// started here go through the phone.
Item {
    id: page
    property bool active: false
    // The phone shown (the one on Home).
    property string deviceId
    property string deviceName
    // "recent" (call log) or "contacts".
    property string tab: "recent"
    // Contact shown above the keypad when picked from the list.
    property var selectedContact: null

    signal textRequested(string number, string name)

    readonly property var callLog: { try { return JSON.parse(PhoneCall.callLog) } catch (e) { return [] } }
    readonly property var contacts: { try { return JSON.parse(PhoneCall.contacts) } catch (e) { return [] } }
    readonly property string query: search.text.trim().toLocaleLowerCase()

    readonly property var shownCalls: query.length === 0 ? callLog
        : callLog.filter(c => ((c.name || "") + " " + c.number).toLocaleLowerCase().indexOf(query) >= 0)

    readonly property var shownContacts: query.length === 0 ? contacts
        : contacts.filter(c => {
            const nums = (c.numbers || []).map(n => n.number + " " + (n.label || "")).join(" ")
            return (c.name + " " + nums).toLocaleLowerCase().indexOf(query) >= 0
        })

    // Digits for matching a typed number to a contact.
    function digits(s) {
        const d = (s || "").replace(/\D/g, "")
        return d.length > 9 ? d.slice(d.length - 9) : d
    }

    // Contact matching the number currently in the dialer, if any.
    readonly property var dialMatchedContact: {
        const d = digits(dialInput.text)
        if (d.length < 3) return null
        for (let i = 0; i < contacts.length; i++) {
            const c = contacts[i]
            const nums = c.numbers || []
            for (let j = 0; j < nums.length; j++) {
                if (digits(nums[j].number) === d)
                    return { name: c.name, label: nums[j].label || "" }
            }
        }
        return null
    }

    // A stable color for a contact (in Bloom).
    function hueOf(name) {
        let h = 0
        for (let i = 0; i < name.length; i++)
            h = (h * 31 + name.charCodeAt(i)) % 360
        return h / 360
    }

    // "Today", "Yesterday", or the date, for call log separators.
    function dayLabel(ms) {
        const date = new Date(ms)
        const today = new Date()
        const yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1)
        if (date.toDateString() === today.toDateString()) return qsTr("Today")
        if (date.toDateString() === yesterday.toDateString()) return qsTr("Yesterday")
        return date.toLocaleDateString(Qt.locale(), date.getFullYear() === today.getFullYear() ? "dddd, d MMMM" : Locale.LongFormat)
    }

    // "14:05", "Yesterday", "Tue", or a date.
    function shortTime(ms) {
        const date = new Date(ms)
        const now = new Date()
        if (date.toDateString() === now.toDateString())
            return date.toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
        const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1)
        if (date.toDateString() === yesterday.toDateString())
            return qsTr("Yesterday")
        if (now - date < 6 * 24 * 3600 * 1000)
            return date.toLocaleDateString(Qt.locale(), "ddd")
        return date.toLocaleDateString(Qt.locale(), Locale.ShortFormat)
    }

    // "1:05" or "42s" for a completed call's duration.
    function formatDuration(seconds) {
        if (!seconds || seconds <= 0) return ""
        const h = Math.floor(seconds / 3600)
        const m = Math.floor((seconds % 3600) / 60)
        const s = seconds % 60
        if (h > 0)
            return h + ":" + String(m).padStart(2, "0") + ":" + String(s).padStart(2, "0")
        if (m > 0)
            return m + ":" + String(s).padStart(2, "0")
        return qsTr("%1s").arg(s)
    }

    function placeCall(number) {
        const n = (number || "").trim()
        if (n.length === 0 || !PhoneCall.canDial || PhoneCall.dialing)
            return
        PhoneCall.dial(n)
    }

    function startText(number, name) {
        const n = (number || "").trim()
        if (n.length === 0)
            return
        page.textRequested(n, name || "")
    }

    function consumePendingDial() {
        if (AppController.pendingDial.length > 0) {
            dialInput.text = AppController.pendingDial
            AppController.pendingDial = ""
        }
    }

    function load() {
        if (active && deviceId.length > 0)
            PhoneCall.open(deviceId)
        consumePendingDial()
    }

    Connections {
        target: AppController
        function onPendingDialChanged() { page.consumePendingDial() }
    }

    onActiveChanged: if (active) load()
    Component.onCompleted: load()
    onDeviceIdChanged: {
        selectedContact = null
        load()
    }

    opacity: active ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: Theme.fadeNormal } }
    transform: Translate {
        y: page.active || Theme.reduceMotion ? 0 : 12
        Behavior on y { SpringAnimation { spring: Theme.springGentle; damping: Theme.dampingGentle } }
    }

    Shortcut {
        sequence: StandardKey.Find
        enabled: page.active && page.deviceId.length > 0
        onActivated: search.forceActiveFocus()
    }
    Shortcut {
        sequence: "Esc"
        enabled: page.active && (page.selectedContact !== null || search.text.length > 0)
        onActivated: {
            if (search.text.length > 0)
                search.text = ""
            else
                page.selectedContact = null
        }
    }

    readonly property bool phoneOffline: page.deviceId.length === 0
        || (PhoneCall.logStatus === "offline" && PhoneCall.contactsStatus === "offline"
            && page.callLog.length === 0 && page.contacts.length === 0)

    // ---- No phone paired, or phone offline with nothing cached ----
    Column {
        anchors.centerIn: parent
        width: Math.min(parent.width - Theme.contentPadding * 2, 420)
        spacing: 12
        visible: page.phoneOffline
        Icon {
            anchors.horizontalCenter: parent.horizontalCenter
            width: 40; height: 40
            stroke: 1.5
            path: Icons.call
            color: Theme.surfaceContentVariant
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            role: "title"
            text: page.deviceId.length === 0 ? qsTr("Call from your PC")
                : qsTr("%1 isn't connected").arg(page.deviceName)
        }
        Txt {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            role: "body"
            muted: true
            text: page.deviceId.length === 0 ? qsTr("Pair your phone to see recent calls, browse contacts, and place calls here.")
                : qsTr("Your call log and contacts show up here when it's back.")
        }
    }

    // ---- Left pane: Recent calls and Contacts ----
    Item {
        id: list
        visible: !page.phoneOffline
        width: Math.min(370, Math.max(280, page.width * 0.36))
        height: parent.height

        Item {
            id: listHeader
            width: parent.width
            height: 56
            Segmented {
                anchors.left: parent.left
                anchors.leftMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                options: [
                    { value: "recent", label: qsTr("Recent") },
                    { value: "contacts", label: qsTr("Contacts") }
                ]
                value: page.tab
                onPicked: (v) => page.tab = v
            }
            IconButton {
                anchors.right: parent.right
                anchors.rightMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                iconPath: Icons.refresh
                label: qsTr("Refresh")
                enabled: PhoneCall.logStatus !== "loading" && PhoneCall.contactsStatus !== "loading"
                onClicked: PhoneCall.refresh()
            }
        }

        // Search: by name or number.
        Rectangle {
            id: searchBox
            anchors.top: listHeader.bottom
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.leftMargin: 16
            anchors.rightMargin: 16
            height: 38
            radius: Theme.graphite ? Theme.radiusSm : height / 2
            color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
            border.width: search.activeFocus ? 2 : (Theme.graphite ? 1 : 0)
            border.color: search.activeFocus ? Theme.primary : Theme.outlineVariant
            Icon {
                id: searchIcon
                anchors.left: parent.left
                anchors.leftMargin: 12
                anchors.verticalCenter: parent.verticalCenter
                width: 16; height: 16
                path: Icons.search
                color: Theme.surfaceContentVariant
            }
            TextInput {
                id: search
                anchors.left: searchIcon.right
                anchors.leftMargin: 8
                anchors.right: clearSearch.left
                anchors.verticalCenter: parent.verticalCenter
                font.family: Theme.fontUi
                font.pixelSize: 14
                color: Theme.surfaceContent
                selectionColor: Theme.primaryContainer
                selectedTextColor: Theme.primaryContainerContent
                clip: true
                Accessible.name: page.tab === "recent" ? qsTr("Search calls") : qsTr("Search contacts")
                Keys.onEscapePressed: (event) => {
                    event.accepted = text.length > 0
                    text = ""
                }
                Txt {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: search.text.length === 0
                    role: "body"
                    muted: true
                    text: page.tab === "recent" ? qsTr("Search calls") : qsTr("Search contacts")
                }
            }
            IconButton {
                id: clearSearch
                anchors.right: parent.right
                anchors.rightMargin: 2
                anchors.verticalCenter: parent.verticalCenter
                width: 32; height: 32
                visible: search.text.length > 0
                iconPath: Icons.close
                label: qsTr("Clear search")
                onClicked: search.text = ""
            }
        }

        // ---- Recent calls tab ----
        Item {
            anchors.top: searchBox.bottom
            anchors.topMargin: 8
            anchors.bottom: parent.bottom
            width: parent.width
            visible: page.tab === "recent"

            Spinner {
                anchors.centerIn: parent
                visible: PhoneCall.logStatus === "loading" && page.callLog.length === 0
            }

            // Permission / toggle / error state for call log.
            Column {
                anchors.centerIn: parent
                width: parent.width - 40
                spacing: 10
                visible: page.callLog.length === 0 && PhoneCall.logStatus !== "loading" && PhoneCall.logStatus !== "ready"
                Icon {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 32; height: 32
                    stroke: 1.5
                    path: Icons.call
                    color: Theme.surfaceContentVariant
                }
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    role: "title"
                    size: 15
                    text: PhoneCall.logStatus === "off" ? qsTr("Calls are off for %1").arg(page.deviceName)
                        : PhoneCall.logStatus === "unsupported" ? qsTr("Allow call log on %1").arg(page.deviceName)
                        : PhoneCall.logStatus === "offline" ? qsTr("%1 isn't connected").arg(page.deviceName)
                        : qsTr("Couldn't load recent calls")
                }
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    role: "bodySmall"
                    muted: true
                    text: PhoneCall.logStatus === "off" ? qsTr("Turn on Calls for this phone, here in Settings and in the Nectarlink app on the phone.")
                        : PhoneCall.logStatus === "unsupported" ? qsTr("In the Nectarlink app on your phone, tap Allow on “Call log” or “Call and answer”.")
                        : PhoneCall.logStatus === "offline" ? qsTr("Your recent calls show up here when it's back.")
                        : qsTr("Something went wrong reading the call log from the phone.")
                }
                Button {
                    anchors.horizontalCenter: parent.horizontalCenter
                    visible: PhoneCall.logStatus === "failed"
                    variant: "tonal"
                    size: "sm"
                    text: qsTr("Try again")
                    onClicked: PhoneCall.refresh()
                }
            }

            ListView {
                id: recentList
                anchors.fill: parent
                visible: page.callLog.length > 0 || PhoneCall.logStatus === "ready"
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                model: page.shownCalls
                onAtYEndChanged: if (atYEnd && PhoneCall.moreLog && count > 0) PhoneCall.loadOlder()
                delegate: CallRow {
                    width: recentList.width
                    entry: modelData
                    previous: index > 0 ? page.shownCalls[index - 1] : null
                }
                footer: Item {
                    width: recentList.width
                    height: PhoneCall.loadingOlder ? 44 : 0
                    Spinner { anchors.centerIn: parent; visible: PhoneCall.loadingOlder }
                }
                Txt {
                    anchors.centerIn: parent
                    visible: PhoneCall.logStatus === "ready" && page.shownCalls.length === 0
                    role: "body"
                    muted: true
                    text: page.query.length > 0 ? qsTr("No calls match.") : qsTr("No recent calls.")
                }
            }
        }

        // ---- Contacts tab ----
        Item {
            anchors.top: searchBox.bottom
            anchors.topMargin: 8
            anchors.bottom: parent.bottom
            width: parent.width
            visible: page.tab === "contacts"

            Spinner {
                anchors.centerIn: parent
                visible: PhoneCall.contactsStatus === "loading" && page.contacts.length === 0
            }

            // Permission / toggle / error state for contacts.
            Column {
                anchors.centerIn: parent
                width: parent.width - 40
                spacing: 10
                visible: page.contacts.length === 0 && PhoneCall.contactsStatus !== "loading" && PhoneCall.contactsStatus !== "ready"
                Icon {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: 32; height: 32
                    stroke: 1.5
                    path: Icons.person
                    color: Theme.surfaceContentVariant
                }
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    role: "title"
                    size: 15
                    text: PhoneCall.contactsStatus === "off" ? qsTr("Contacts are off for %1").arg(page.deviceName)
                        : PhoneCall.contactsStatus === "unsupported" ? qsTr("Allow contacts on %1").arg(page.deviceName)
                        : PhoneCall.contactsStatus === "offline" ? qsTr("%1 isn't connected").arg(page.deviceName)
                        : qsTr("Couldn't load contacts")
                }
                Txt {
                    width: parent.width
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                    role: "bodySmall"
                    muted: true
                    text: PhoneCall.contactsStatus === "off" ? qsTr("Turn on Contacts for this phone, here in Settings and in the Nectarlink app on the phone.")
                        : PhoneCall.contactsStatus === "unsupported" ? qsTr("In the Nectarlink app on your phone, tap Allow on “Contacts”.")
                        : PhoneCall.contactsStatus === "offline" ? qsTr("Your contacts show up here when it's back.")
                        : qsTr("Something went wrong reading contacts from the phone.")
                }
                Button {
                    anchors.horizontalCenter: parent.horizontalCenter
                    visible: PhoneCall.contactsStatus === "failed"
                    variant: "tonal"
                    size: "sm"
                    text: qsTr("Try again")
                    onClicked: PhoneCall.refresh()
                }
            }

            ListView {
                id: contactList
                anchors.fill: parent
                visible: page.contacts.length > 0 || PhoneCall.contactsStatus === "ready"
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                model: page.shownContacts
                delegate: ContactRow {
                    width: contactList.width
                    contact: modelData
                    previous: index > 0 ? page.shownContacts[index - 1] : null
                }
                Txt {
                    anchors.centerIn: parent
                    visible: PhoneCall.contactsStatus === "ready" && page.shownContacts.length === 0
                    role: "body"
                    muted: true
                    text: page.query.length > 0 ? qsTr("No contacts match.") : qsTr("No contacts on this phone.")
                }
            }
        }

        Divider { anchors.right: parent.right; width: 1; height: parent.height }
    }

    // ---- Right pane: Active call, selected contact, and keypad dialer ----
    Flickable {
        id: rightPane
        visible: !page.phoneOffline
        anchors.left: list.right
        anchors.right: parent.right
        height: parent.height
        contentHeight: Math.max(height, rightColumn.height + Theme.contentPadding * 2)
        boundsBehavior: Flickable.StopAtBounds
        clip: true

        Column {
            id: rightColumn
            width: Math.min(380, rightPane.width - Theme.contentPadding * 2)
            x: Math.max(Theme.contentPadding, (rightPane.width - width) / 2)
            y: Math.max(Theme.contentPadding, (rightPane.height - height) / 2)
            spacing: 16

            // Active call on the phone (reuses CallCard).
            CallCard {
                width: parent.width
                visible: PhoneCall.active && PhoneCall.device === page.deviceId
            }

            // Selected contact details (when a contact with one or more numbers is clicked).
            Card {
                id: contactDetailCard
                width: parent.width
                visible: page.selectedContact !== null
                Column {
                    width: parent.width
                    spacing: 10
                    Item {
                        width: parent.width
                        height: 44
                        Item {
                            id: detailAvatar
                            width: 40; height: 40
                            anchors.verticalCenter: parent.verticalCenter
                            RoundedImage {
                                anchors.fill: parent
                                radius: width / 2
                                visible: (page.selectedContact ? page.selectedContact.photo : "").length > 0 && status === Image.Ready
                                source: page.selectedContact ? page.selectedContact.photo : ""
                                sourceSize: Qt.size(80, 80)
                                fillMode: Image.PreserveAspectCrop
                            }
                            Avatar {
                                anchors.fill: parent
                                visible: !page.selectedContact || page.selectedContact.photo.length === 0
                                name: page.selectedContact ? page.selectedContact.name : ""
                                hue: page.selectedContact ? page.hueOf(page.selectedContact.name) : -1
                            }
                        }
                        Column {
                            anchors.left: detailAvatar.right
                            anchors.leftMargin: 12
                            anchors.right: closeDetail.left
                            anchors.rightMargin: 8
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 2
                            Row {
                                spacing: 6
                                Txt {
                                    text: page.selectedContact ? page.selectedContact.name : ""
                                    role: "title"
                                    elide: Text.ElideRight
                                }
                                Icon {
                                    anchors.verticalCenter: parent.verticalCenter
                                    visible: !!(page.selectedContact && page.selectedContact.starred)
                                    width: 14; height: 14
                                    path: Icons.star
                                    color: Theme.primary
                                }
                            }
                        }
                        IconButton {
                            id: closeDetail
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            width: 32; height: 32
                            iconPath: Icons.close
                            label: qsTr("Close contact")
                            onClicked: page.selectedContact = null
                        }
                    }

                    Repeater {
                        model: page.selectedContact ? (page.selectedContact.numbers || []) : []
                        delegate: Item {
                            id: numRow
                            required property var modelData
                            width: parent.width
                            height: 44
                            Column {
                                anchors.left: parent.left
                                anchors.right: numActions.left
                                anchors.rightMargin: 8
                                anchors.verticalCenter: parent.verticalCenter
                                Txt {
                                    width: parent.width
                                    text: numRow.modelData.number
                                    role: "body"
                                    elide: Text.ElideRight
                                }
                                Txt {
                                    width: parent.width
                                    visible: (numRow.modelData.label || "").length > 0
                                    text: numRow.modelData.label || ""
                                    role: "caption"
                                    muted: true
                                    elide: Text.ElideRight
                                }
                            }
                            Row {
                                id: numActions
                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 4
                                IconButton {
                                    width: 34; height: 34
                                    iconPath: Icons.copy
                                    label: qsTr("Copy number")
                                    onClicked: PhoneCall.copy(numRow.modelData.number)
                                }
                                IconButton {
                                    width: 34; height: 34
                                    iconPath: Icons.messages
                                    label: qsTr("Text %1").arg(numRow.modelData.number)
                                    onClicked: page.startText(numRow.modelData.number, page.selectedContact ? page.selectedContact.name : "")
                                }
                                IconButton {
                                    width: 34; height: 34
                                    tonal: true
                                    enabled: PhoneCall.canDial && !PhoneCall.dialing
                                    iconPath: Icons.call
                                    label: qsTr("Call %1").arg(numRow.modelData.number)
                                    onClicked: {
                                        dialInput.text = numRow.modelData.number
                                        page.placeCall(numRow.modelData.number)
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // ---- Dialer / Keypad ----
            Card {
                id: dialerCard
                width: parent.width

                readonly property var dialFeature: AppController.capsRevision >= 0 && page.deviceId.length > 0
                    ? AppController.featureState(page.deviceId, "calls.dial") : ({})

                Column {
                    width: parent.width
                    spacing: 14

                    // Number entry box + backspace.
                    Rectangle {
                        width: parent.width
                        height: dialMatch.visible ? 64 : 52
                        radius: Theme.graphite ? Theme.radiusSm : Theme.radiusLg
                        color: Theme.graphite ? "transparent" : Theme.surfaceContainerHighest
                        border.width: dialInput.activeFocus ? 2 : (Theme.graphite ? 1 : 0)
                        border.color: dialInput.activeFocus ? Theme.primary : Theme.outlineVariant

                        Column {
                            anchors.left: parent.left
                            anchors.leftMargin: 16
                            anchors.right: backspaceButton.visible ? backspaceButton.left : parent.right
                            anchors.rightMargin: 8
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 2

                            TextInput {
                                id: dialInput
                                width: parent.width
                                horizontalAlignment: TextInput.AlignHCenter
                                font.family: Theme.fontMono
                                font.pixelSize: 20
                                font.weight: 600
                                color: Theme.surfaceContent
                                selectionColor: Theme.primaryContainer
                                selectedTextColor: Theme.primaryContainerContent
                                inputMethodHints: Qt.ImhDialableCharactersOnly
                                clip: true
                                Accessible.name: qsTr("Phone number to call")
                                Keys.onReturnPressed: page.placeCall(dialInput.text)
                                Keys.onEnterPressed: page.placeCall(dialInput.text)
                                Txt {
                                    anchors.centerIn: parent
                                    visible: dialInput.text.length === 0
                                    role: "body"
                                    muted: true
                                    text: qsTr("Enter a number")
                                }
                            }
                            Txt {
                                id: dialMatch
                                width: parent.width
                                horizontalAlignment: Text.AlignHCenter
                                visible: page.dialMatchedContact !== null
                                role: "caption"
                                color: Theme.primary
                                elide: Text.ElideRight
                                text: page.dialMatchedContact
                                    ? (page.dialMatchedContact.label.length > 0
                                        ? page.dialMatchedContact.name + " · " + page.dialMatchedContact.label
                                        : page.dialMatchedContact.name)
                                    : ""
                            }
                        }

                        IconButton {
                            id: backspaceButton
                            anchors.right: parent.right
                            anchors.rightMargin: 6
                            anchors.verticalCenter: parent.verticalCenter
                            width: 36; height: 36
                            visible: dialInput.text.length > 0
                            iconPath: Icons.backspace
                            label: qsTr("Backspace")
                            onClicked: dialInput.text = dialInput.text.slice(0, -1)
                        }
                    }

                    // 3x4 telephone keypad.
                    Grid {
                        anchors.horizontalCenter: parent.horizontalCenter
                        columns: 3
                        columnSpacing: 12
                        rowSpacing: 10
                        Repeater {
                            model: [
                                { digit: "1", sub: "" },
                                { digit: "2", sub: "ABC" },
                                { digit: "3", sub: "DEF" },
                                { digit: "4", sub: "GHI" },
                                { digit: "5", sub: "JKL" },
                                { digit: "6", sub: "MNO" },
                                { digit: "7", sub: "PQRS" },
                                { digit: "8", sub: "TUV" },
                                { digit: "9", sub: "WXYZ" },
                                { digit: "*", sub: "" },
                                { digit: "0", sub: "+" },
                                { digit: "#", sub: "" }
                            ]
                            delegate: Rectangle {
                                id: padKey
                                required property var modelData
                                width: 84; height: 50
                                radius: Theme.pill(height)
                                activeFocusOnTab: true
                                color: padTap.pressed
                                    ? Theme.secondaryContainer
                                    : padHover.hovered
                                      ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.08)
                                      : (Theme.graphite ? "transparent" : Theme.surfaceContainerHigh)
                                border.width: Theme.focusVisible(padKey) ? 2 : (Theme.graphite ? 1 : 0)
                                border.color: Theme.focusVisible(padKey) ? Theme.primary : Theme.outlineVariant
                                scale: padTap.pressed ? Theme.pressScale : 1
                                Behavior on scale { SpringAnimation { spring: Theme.springSnappy; damping: Theme.dampingSnappy } }

                                function tapDigit(ch) {
                                    dialInput.text = dialInput.text + ch
                                    if (PhoneCall.active && PhoneCall.controls)
                                        PhoneCall.press(ch)
                                }

                                Keys.onPressed: (event) => {
                                    if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                                        tapDigit(modelData.digit)
                                        event.accepted = true
                                    }
                                }

                                Accessible.role: Accessible.Button
                                Accessible.name: modelData.sub.length > 0 ? modelData.digit + " " + modelData.sub : modelData.digit
                                Accessible.onPressAction: tapDigit(modelData.digit)

                                Column {
                                    anchors.centerIn: parent
                                    spacing: -2
                                    Txt {
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        role: "title"
                                        size: 18
                                        text: padKey.modelData.digit
                                    }
                                    Txt {
                                        anchors.horizontalCenter: parent.horizontalCenter
                                        visible: padKey.modelData.sub.length > 0
                                        role: "caption"
                                        size: 9
                                        muted: true
                                        text: padKey.modelData.sub
                                    }
                                }
                                HoverHandler { id: padHover; cursorShape: Qt.PointingHandCursor }
                                TapHandler {
                                    id: padTap
                                    onTapped: padKey.tapDigit(padKey.modelData.digit)
                                    onLongPressed: {
                                        if (padKey.modelData.digit === "0")
                                            padKey.tapDigit("+")
                                    }
                                }
                            }
                        }
                    }

                    // Call and Text buttons.
                    Row {
                        anchors.horizontalCenter: parent.horizontalCenter
                        spacing: 14

                        Button {
                            anchors.verticalCenter: parent.verticalCenter
                            variant: "tonal"
                            iconPath: Icons.messages
                            text: qsTr("Text")
                            enabled: dialInput.text.trim().length > 0
                            onClicked: page.startText(
                                dialInput.text,
                                page.dialMatchedContact ? page.dialMatchedContact.name : ""
                            )
                        }

                        Button {
                            anchors.verticalCenter: parent.verticalCenter
                            variant: "fill"
                            iconPath: Icons.call
                            text: PhoneCall.dialing ? qsTr("Calling…") : qsTr("Call")
                            enabled: dialInput.text.trim().length > 0 && PhoneCall.canDial && !PhoneCall.dialing
                            onClicked: page.placeCall(dialInput.text)
                        }

                        IconButton {
                            anchors.verticalCenter: parent.verticalCenter
                            visible: dialInput.text.length > 0
                            iconPath: Icons.close
                            label: qsTr("Clear number")
                            onClicked: dialInput.text = ""
                        }
                    }

                    LockChip {
                        anchors.horizontalCenter: parent.horizontalCenter
                        visible: !PhoneCall.canDial && label.length > 0
                        feature: dialerCard.dialFeature
                    }
                }
            }
        }
    }

    // ---- A row in the Recent calls list ----
    component CallRow: Item {
        id: row
        property var entry
        property var previous: null
        readonly property bool newDay: !previous || new Date(previous.date).toDateString() !== new Date(entry.date).toDateString()
        readonly property bool missed: entry.direction === "missed"
        readonly property string displayName: (entry.name && entry.name.length > 0) ? entry.name : entry.number
        readonly property bool named: !!(entry.name && entry.name.length > 0 && !/^[\d+(#*]/.test(entry.name))
        readonly property string durText: page.formatDuration(entry.duration)
        readonly property string dirLabel: entry.direction === "missed" ? qsTr("Missed")
            : entry.direction === "outgoing" ? qsTr("Outgoing")
            : entry.direction === "rejected" ? qsTr("Declined")
            : qsTr("Incoming")

        height: dayHeader.height + 68
        activeFocusOnTab: true
        Accessible.role: Accessible.ListItem
        Accessible.name: displayName + ", " + dirLabel + ", " + page.shortTime(entry.date)
        Accessible.onPressAction: selectCall()

        function selectCall() {
            dialInput.text = entry.number
        }

        Keys.onPressed: (event) => {
            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                selectCall()
                event.accepted = true
            }
        }

        // Day separator ("Today", "Yesterday", date).
        Txt {
            id: dayHeader
            width: parent.width
            height: row.newDay ? implicitHeight + 18 : 0
            visible: row.newDay
            topPadding: 10
            leftPadding: 20
            role: "label"
            muted: true
            text: row.newDay ? page.dayLabel(row.entry.date) : ""
        }

        Item {
            anchors.top: dayHeader.bottom
            width: parent.width
            height: 68

            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                radius: Theme.radiusLg
                color: callHover.hovered
                    ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.06)
                    : "transparent"
                border.width: Theme.focusVisible(row) ? 2 : 0
                border.color: Theme.primary
                Behavior on color { ColorAnimation { duration: Theme.fadeFast } }
            }

            Item {
                id: callFace
                x: 20
                anchors.verticalCenter: parent.verticalCenter
                width: 40; height: 40
                RoundedImage {
                    anchors.fill: parent
                    radius: width / 2
                    visible: (row.entry.photo || "").length > 0 && status === Image.Ready
                    source: row.entry.photo || ""
                    sourceSize: Qt.size(80, 80)
                    fillMode: Image.PreserveAspectCrop
                }
                Avatar {
                    anchors.fill: parent
                    visible: (row.entry.photo || "").length === 0
                    name: row.named ? row.displayName : ""
                    iconPath: row.named ? "" : Icons.person
                    hue: row.named ? page.hueOf(row.displayName) : -1
                    emphasized: row.missed
                }
            }

            Column {
                anchors.left: callFace.right
                anchors.leftMargin: 12
                anchors.right: callRowActions.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2

                Txt {
                    width: parent.width
                    text: row.displayName
                    role: "body"
                    weight: row.missed ? 650 : 500
                    color: row.missed ? Theme.error : Theme.surfaceContent
                    elide: Text.ElideRight
                }

                Row {
                    width: parent.width
                    spacing: 5
                    Icon {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 13; height: 13
                        stroke: 2
                        path: row.missed ? Icons.callMissed
                            : row.entry.direction === "outgoing" ? Icons.callOutgoing
                            : Icons.callIncoming
                        color: row.missed ? Theme.error : Theme.surfaceContentVariant
                    }
                    Txt {
                        width: parent.width - 18
                        role: "bodySmall"
                        color: row.missed ? Theme.error : Theme.surfaceContentVariant
                        elide: Text.ElideRight
                        text: {
                            const parts = [row.dirLabel]
                            if (row.durText.length > 0)
                                parts.push(row.durText)
                            parts.push(page.shortTime(row.entry.date))
                            return parts.join(" · ")
                        }
                    }
                }
            }

            Row {
                id: callRowActions
                anchors.right: parent.right
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2
                IconButton {
                    width: 34; height: 34
                    iconPath: Icons.messages
                    label: qsTr("Text %1").arg(row.displayName)
                    onClicked: page.startText(row.entry.number, row.entry.name || "")
                }
                IconButton {
                    width: 34; height: 34
                    tonal: callHover.hovered
                    enabled: PhoneCall.canDial && !PhoneCall.dialing && row.entry.number.length > 0
                    iconPath: Icons.call
                    label: qsTr("Call %1").arg(row.displayName)
                    onClicked: {
                        dialInput.text = row.entry.number
                        page.placeCall(row.entry.number)
                    }
                }
            }

            HoverHandler { id: callHover; cursorShape: Qt.PointingHandCursor }
            TapHandler { onTapped: row.selectCall() }
        }
    }

    // ---- A row in the Contacts list ----
    component ContactRow: Item {
        id: crow
        property var contact
        property var previous: null
        readonly property var primaryEntry: (contact.numbers && contact.numbers.length > 0) ? contact.numbers[0] : null
        readonly property string primaryNumber: primaryEntry ? primaryEntry.number : ""
        readonly property string primaryLabel: primaryEntry && primaryEntry.label ? primaryEntry.label : ""
        readonly property bool selected: page.selectedContact !== null && page.selectedContact.id === contact.id
        // Section header ("Favorites" / "All contacts") when not filtering.
        readonly property bool showSection: page.query.length === 0
            && (!previous || previous.starred !== contact.starred)

        height: sectionLabel.height + 68
        activeFocusOnTab: true
        Accessible.role: Accessible.ListItem
        Accessible.name: contact.name + (primaryNumber.length > 0 ? ", " + primaryNumber : "")
        Accessible.onPressAction: pickContact()

        function pickContact() {
            page.selectedContact = contact
            if (primaryNumber.length > 0)
                dialInput.text = primaryNumber
        }

        Keys.onPressed: (event) => {
            if (event.key === Qt.Key_Return || event.key === Qt.Key_Enter || event.key === Qt.Key_Space) {
                pickContact()
                event.accepted = true
            }
        }

        Row {
            id: sectionLabel
            x: 20
            height: crow.showSection ? 28 : 0
            visible: crow.showSection
            spacing: 6
            Icon {
                anchors.verticalCenter: parent.verticalCenter
                visible: !!crow.contact.starred
                width: 13; height: 13
                path: Icons.star
                color: Theme.primary
            }
            Txt {
                anchors.verticalCenter: parent.verticalCenter
                role: "label"
                muted: true
                text: crow.contact.starred ? qsTr("Favorites") : qsTr("Contacts")
            }
        }

        Item {
            anchors.top: sectionLabel.bottom
            width: parent.width
            height: 68

            Rectangle {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 8
                radius: Theme.radiusLg
                color: crow.selected ? Theme.secondaryContainer
                    : contactHover.hovered ? Qt.rgba(Theme.surfaceContent.r, Theme.surfaceContent.g, Theme.surfaceContent.b, 0.06)
                    : "transparent"
                border.width: Theme.focusVisible(crow) ? 2 : 0
                border.color: Theme.primary
                Behavior on color { ColorAnimation { duration: Theme.fadeFast } }
            }

            Item {
                id: contactFace
                x: 20
                anchors.verticalCenter: parent.verticalCenter
                width: 40; height: 40
                RoundedImage {
                    anchors.fill: parent
                    radius: width / 2
                    visible: (crow.contact.photo || "").length > 0 && status === Image.Ready
                    source: crow.contact.photo || ""
                    sourceSize: Qt.size(80, 80)
                    fillMode: Image.PreserveAspectCrop
                }
                Avatar {
                    anchors.fill: parent
                    visible: (crow.contact.photo || "").length === 0
                    name: crow.contact.name
                    hue: page.hueOf(crow.contact.name)
                    emphasized: !!crow.contact.starred
                }
            }

            Column {
                anchors.left: contactFace.right
                anchors.leftMargin: 12
                anchors.right: contactActions.left
                anchors.rightMargin: 8
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2

                Row {
                    width: parent.width
                    spacing: 6
                    Txt {
                        width: Math.min(implicitWidth, parent.width - (crow.contact.starred ? 20 : 0))
                        text: crow.contact.name
                        role: "body"
                        weight: 500
                        elide: Text.ElideRight
                    }
                    Icon {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: !!crow.contact.starred
                        width: 13; height: 13
                        path: Icons.star
                        color: Theme.primary
                    }
                }

                Txt {
                    width: parent.width
                    role: "bodySmall"
                    muted: true
                    elide: Text.ElideRight
                    text: {
                        const parts = []
                        if (crow.primaryLabel.length > 0)
                            parts.push(crow.primaryLabel)
                        if (crow.primaryNumber.length > 0)
                            parts.push(crow.primaryNumber)
                        const extra = (crow.contact.numbers ? crow.contact.numbers.length : 0) - 1
                        if (extra > 0)
                            parts.push(qsTr("+%1 more").arg(extra))
                        return parts.join(" · ")
                    }
                }
            }

            Row {
                id: contactActions
                anchors.right: parent.right
                anchors.rightMargin: 14
                anchors.verticalCenter: parent.verticalCenter
                spacing: 2
                IconButton {
                    width: 34; height: 34
                    visible: crow.primaryNumber.length > 0
                    iconPath: Icons.messages
                    label: qsTr("Text %1").arg(crow.contact.name)
                    onClicked: page.startText(crow.primaryNumber, crow.contact.name)
                }
                IconButton {
                    width: 34; height: 34
                    visible: crow.primaryNumber.length > 0
                    tonal: contactHover.hovered
                    enabled: PhoneCall.canDial && !PhoneCall.dialing
                    iconPath: Icons.call
                    label: qsTr("Call %1").arg(crow.contact.name)
                    onClicked: {
                        dialInput.text = crow.primaryNumber
                        page.placeCall(crow.primaryNumber)
                    }
                }
            }

            HoverHandler { id: contactHover; cursorShape: Qt.PointingHandCursor }
            TapHandler { onTapped: crow.pickContact() }
        }
    }
}
