// SPDX-License-Identifier: GPL-3.0-or-later
pragma Singleton
import QtQuick

// Every link the app itself opens, in one place and nowhere else: never in
// a translation, a setting or anything that arrives over the network, so
// nothing can repoint one. Open them by key with open(); a test pins each
// address (src/app_links.rs).
QtObject {
    id: links

    readonly property string discord: "https://discord.gg/AA2Dp8PYqf"
    readonly property string x: "https://x.com/thtbee_"
    readonly property string donate: "https://buymeacoffee.com/thtbee_"
    readonly property string repo: "https://github.com/thtbee/nectarlink"
    readonly property string releases: "https://github.com/thtbee/nectarlink/releases"
    readonly property string latestRelease: "https://github.com/thtbee/nectarlink/releases/latest"
    readonly property string privacy: "https://github.com/thtbee/nectarlink/blob/main/docs/guide/privacy-and-security.md"
    readonly property string androidSdkTerms: "https://developer.android.com/studio/terms"
    readonly property string oemUsb: "https://developer.android.com/studio/run/oem-usb"

    // The hosts these may point at; anything else isn't opened.
    readonly property var hosts: ["discord.gg", "x.com", "buymeacoffee.com", "github.com", "developer.android.com"]

    // The address for `key`, or "" for a key that isn't one of the above.
    function url(key) {
        switch (key) {
        case "discord": return discord
        case "x": return x
        case "donate": return donate
        case "repo": return repo
        case "releases": return releases
        case "latestRelease": return latestRelease
        case "privacy": return privacy
        case "androidSdkTerms": return androidSdkTerms
        case "oemUsb": return oemUsb
        default: return ""
        }
    }

    // Whether `address` is https on one of the hosts above.
    function allowed(address) {
        const match = /^https:\/\/([^\/:?#]+)(?:[\/?#]|$)/.exec(address)
        return match !== null && hosts.indexOf(match[1]) >= 0
    }

    // Opens the link `key`; an unknown key opens nothing.
    function open(key) {
        const address = url(key)
        if (address.length === 0 || !allowed(address)) {
            console.warn("Links: not opening", key)
            return false
        }
        return Qt.openUrlExternally(address)
    }
}
