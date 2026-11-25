// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.update

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AppUpdaterTest {
    @Test
    fun versionsCompare() {
        assertTrue(AppUpdater.isNewer("0.1.0", "0.0.1"))
        assertTrue(AppUpdater.isNewer("1.0", "0.9.9"))
        assertTrue(AppUpdater.isNewer("0.2.0", "0.2.0-beta.1"))
        assertTrue(AppUpdater.isNewer("0.2.0-beta.2", "0.2.0-beta.1"))
        assertFalse(AppUpdater.isNewer("0.0.1", "0.0.1"))
        assertFalse(AppUpdater.isNewer("0.0.1", "0.1.0"))
        assertFalse(AppUpdater.isNewer("garbage", "0.1.0"))
        assertFalse(AppUpdater.isNewer("0.1.0-", "0.0.1"))
    }

    @Test
    fun nightliesSortByNumberBetweenReleases() {
        // Numbers, not text: 10 comes after 9.
        assertTrue(AppUpdater.isNewer("0.1.5-nightly.10", "0.1.5-nightly.9"))
        assertFalse(AppUpdater.isNewer("0.1.5-nightly.9", "0.1.5-nightly.10"))
        assertTrue(AppUpdater.isNewer("0.1.5-nightly.1", "0.1.4"))
        assertTrue(AppUpdater.isNewer("0.1.5", "0.1.5-nightly.42"))
        assertFalse(AppUpdater.isNewer("0.1.5-nightly.42", "0.1.5"))
        assertEquals(0, AppUpdater.compareVersions("0.1.5+abc", "0.1.5+def"))
        assertEquals(42, AppUpdater.nightlyBuild("0.1.5-nightly.42"))
        assertNull(AppUpdater.nightlyBuild("0.1.5"))
        assertNull(AppUpdater.nightlyBuild("0.2.0-beta.1"))
    }

    private fun feed(channel: String, version: String, files: String): org.json.JSONObject = org.json.JSONObject(
        """{"schema":1,"channel":"$channel","version":"$version","name":"Amber Clover",
            "notes":"https://github.com/thtbee/nectarlink/releases/tag/v$version","files":[$files]}""",
    )

    private fun apk(version: String, url: String = "https://github.com/thtbee/nectarlink/releases/download/v$version/Nectarlink-$version-android.apk") =
        """{"name":"Nectarlink-$version-android.apk","url":"$url","sha256":"${"A".repeat(64)}","size":1}"""

    @Test
    fun aGoodFeedGivesAnUpdateWithItsHashAndName() {
        val checked = AppUpdater.checkedFeed(feed("nightly", "0.1.5-nightly.7", apk("0.1.5-nightly.7")), AppUpdater.Channel.Nightly)
        val update = AppUpdater.feedUpdate(checked, "0.1.5-nightly.6")!!
        assertEquals("0.1.5-nightly.7", update.version)
        assertEquals("a".repeat(64), update.sha256)
        assertEquals("Amber Clover", update.name)
        assertNull(update.sumsUrl)
        // Not newer: nothing to install.
        assertNull(AppUpdater.feedUpdate(checked, "0.1.5-nightly.7"))
    }

    @Test
    fun feedsMustMatchTheirChannel() {
        fun fails(block: () -> Unit) = assertTrue(runCatching(block).isFailure)
        fails { AppUpdater.checkedFeed(feed("stable", "0.1.5-nightly.7", ""), AppUpdater.Channel.Nightly) }
        fails { AppUpdater.checkedFeed(feed("nightly", "0.1.5", ""), AppUpdater.Channel.Nightly) }
        fails { AppUpdater.checkedFeed(feed("stable", "latest", ""), AppUpdater.Channel.Stable) }
        fails { AppUpdater.checkedFeed(feed("stable", "0.1.5", "").put("schema", 2), AppUpdater.Channel.Stable) }
    }

    @Test
    fun filesFromElsewhereAreDropped() {
        val files = listOf(
            apk("0.1.5", url = "https://evil.example/Nectarlink-0.1.5-android.apk"),
            apk("0.1.5", url = "https://github.com/thtbee/nectarlink/releases/download/../../x.apk"),
        ).joinToString(",")
        val checked = AppUpdater.checkedFeed(feed("stable", "0.1.5", files), AppUpdater.Channel.Stable)
        assertEquals(0, checked.getJSONArray("files").length())
        assertNull(AppUpdater.feedUpdate(checked, "0.1.4"))
    }

    @Test
    fun theChannelComesFromTheVersion() {
        assertEquals(AppUpdater.Channel.Nightly, AppUpdater.channelOf("0.1.5-nightly.3"))
        assertEquals(AppUpdater.Channel.Stable, AppUpdater.channelOf("0.1.5"))
        assertFalse(AppUpdater.isPlainName("../x.apk"))
        assertTrue(AppUpdater.isPlainName("Nectarlink-0.1.5-android.apk"))
    }

    @Test
    fun findsTheApkHash() {
        val hash = "a".repeat(64)
        val sums = "$hash  Nectarlink-9.0.0-android.apk\n${"b".repeat(64)}  Nectarlink-9.0.0-x64-setup.exe\n"
        assertEquals(hash, AppUpdater.listedHash(sums, "Nectarlink-9.0.0-android.apk"))
        assertNull(AppUpdater.listedHash(sums, "missing.apk"))
        assertEquals("c".repeat(64), AppUpdater.listedHash("${"C".repeat(64)} *x.apk", "x.apk"))
        assertNull(AppUpdater.listedHash("${"z".repeat(64)}  x.apk", "x.apk"))
        assertNull(AppUpdater.listedHash("${"../".repeat(21)}a  x.apk", "x.apk"))
    }

    @Test
    fun readsTheFullSizeFromContentRange() {
        assertEquals(49_362_916L, AppUpdater.rangeTotal("bytes 1048576-49362915/49362916"))
        assertNull(AppUpdater.rangeTotal("bytes 0-99/*"))
        assertNull(AppUpdater.rangeTotal(null))
        assertNull(AppUpdater.rangeTotal("garbage"))
    }
}
