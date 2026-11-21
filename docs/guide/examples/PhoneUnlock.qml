// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Services.Pam

// A separate authentication context for an already locked graphical session.
Scope {
    id: root

    required property bool locked
    property bool available: false
    readonly property bool pending: requestPending
    property string message: ""
    property bool requestPending: false

    signal authenticated()

    function start() {
        if (!locked || !available || phone.active || requestPending)
            return;
        requestPending = true;
        message = "";
        if (!phone.start()) {
            requestPending = false;
            message = qsTr("Could not start phone unlock");
        }
    }

    function cancel() {
        requestPending = false;
        message = "";
        if (phone.active)
            phone.abort();
    }

    onLockedChanged: {
        if (locked)
            check.running = true;
        else {
            available = false;
            cancel();
        }
    }

    Process {
        id: check
        command: ["sh", "-c",
            'test -r /etc/pam.d/nectarlink-phone-lock && test -r "/etc/nectarlink/phone-unlock/$(id -u).json"']
        onExited: code => root.available = root.locked && code === 0
    }

    PamContext {
        id: phone
        config: "nectarlink-phone-lock"
        onPamMessage: {
            if (responseRequired)
                root.cancel();
        }
        onCompleted: result => {
            if (!root.requestPending || !root.locked)
                return;
            root.requestPending = false;
            if (result === PamResult.Success)
                root.authenticated();
            else
                root.message = qsTr("Phone unlock failed");
        }
    }
}
