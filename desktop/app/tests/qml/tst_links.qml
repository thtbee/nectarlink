// SPDX-License-Identifier: GPL-3.0-or-later
import QtQuick
import QtTest
import app.nectarlink

// The app's own links: fixed addresses, https on known hosts, by key only.
TestCase {
    name: "Links"

    function test_the_addresses_are_pinned() {
        compare(Links.url("discord"), "https://discord.gg/AA2Dp8PYqf")
        compare(Links.url("x"), "https://x.com/thtbee_")
        compare(Links.url("donate"), "https://buymeacoffee.com/thtbee_")
        compare(Links.url("repo"), "https://github.com/thtbee/nectarlink")
    }

    function test_an_unknown_key_opens_nothing() {
        compare(Links.url("https://evil.example"), "")
        ignoreWarning(/Links: not opening/)
        verify(!Links.open("somewhere"))
    }

    function test_only_https_on_known_hosts() {
        verify(Links.allowed("https://discord.gg/AA2Dp8PYqf"))
        verify(!Links.allowed("http://discord.gg/AA2Dp8PYqf"))
        verify(!Links.allowed("https://discord.gg.evil.example/x"))
        verify(!Links.allowed("https://evil.example/?discord.gg"))
        verify(!Links.allowed("javascript:alert(1)"))
        for (const key of ["discord", "x", "donate", "repo", "releases", "latestRelease", "privacy", "androidSdkTerms", "oemUsb"])
            verify(Links.allowed(Links.url(key)), key)
    }
}
