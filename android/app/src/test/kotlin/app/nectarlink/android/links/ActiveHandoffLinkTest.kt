// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.links

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ActiveHandoffLinkTest {
    @Test
    fun identifiesSupportedBrowsersAndAlwaysPrivateExclusions() {
        assertTrue(ActiveHandoffLink.isBrowserPackage("com.android.chrome"))
        assertTrue(ActiveHandoffLink.isBrowserPackage("com.microsoft.emmx"))
        assertTrue(ActiveHandoffLink.isBrowserPackage("com.brave.browser"))
        assertTrue(ActiveHandoffLink.isBrowserPackage("org.mozilla.firefox"))
        assertTrue(ActiveHandoffLink.isBrowserPackage("com.opera.browser"))
        assertTrue(ActiveHandoffLink.isBrowserPackage("com.sec.android.app.sbrowser"))

        assertFalse(ActiveHandoffLink.isBrowserPackage("org.mozilla.focus"))
        assertFalse(ActiveHandoffLink.isBrowserPackage("org.torproject.torbrowser"))
        assertFalse(ActiveHandoffLink.isBrowserPackage("com.whatsapp"))
        assertFalse(ActiveHandoffLink.isBrowserPackage(null))
    }

    @Test
    fun skipsSensitiveAndBankingApps() {
        val userBanking = setOf("com.example.mybank")
        assertTrue(ActiveHandoffLink.isSensitiveOrBankingPackage("com.example.mybank", userBanking))
        assertTrue(ActiveHandoffLink.isSensitiveOrBankingPackage("com.bitwarden.mobile", userBanking))
        assertTrue(ActiveHandoffLink.isSensitiveOrBankingPackage("com.agilebits.onepassword", userBanking))
        assertTrue(ActiveHandoffLink.isSensitiveOrBankingPackage("org.mozilla.focus", userBanking))
        assertTrue(ActiveHandoffLink.isSensitiveOrBankingPackage("com.paypal.android.p2pmobile", userBanking))

        assertFalse(ActiveHandoffLink.isSensitiveOrBankingPackage("com.android.chrome", userBanking))
        assertFalse(ActiveHandoffLink.isSensitiveOrBankingPackage("com.google.android.youtube", userBanking))
    }

    @Test
    fun detectsPrivateAndIncognitoBrowserModes() {
        assertTrue(
            ActiveHandoffLink.isPrivateBrowserUi(
                "com.android.chrome",
                listOf("You've gone incognito"),
                emptyList(),
            ),
        )
        assertTrue(
            ActiveHandoffLink.isPrivateBrowserUi(
                "com.android.chrome",
                emptyList(),
                listOf("com.android.chrome:id/incognito_tabs_count"),
            ),
        )
        assertTrue(
            ActiveHandoffLink.isPrivateBrowserUi(
                "com.microsoft.emmx",
                listOf("InPrivate tab"),
                emptyList(),
            ),
        )
        assertTrue(
            ActiveHandoffLink.isPrivateBrowserUi(
                "org.mozilla.firefox",
                listOf("Private Browsing"),
                emptyList(),
            ),
        )
        assertTrue(
            ActiveHandoffLink.isPrivateBrowserUi(
                "com.sec.android.app.sbrowser",
                listOf("Secret mode"),
                emptyList(),
            ),
        )
        assertFalse(
            ActiveHandoffLink.isPrivateBrowserUi(
                "com.android.chrome",
                listOf("NectarLink Docs", "Home"),
                listOf("com.android.chrome:id/url_bar"),
            ),
        )
    }

    @Test
    fun normalizesAndStrictlyValidatesHttpUrls() {
        assertEquals(
            "https://example.com/article?id=1",
            ActiveHandoffLink.normalizeHttpUrl("  https://example.com/article?id=1  "),
        )
        assertEquals(
            "http://localhost:8080/test",
            ActiveHandoffLink.normalizeHttpUrl("http://localhost:8080/test"),
        )
        assertEquals(
            "https://youtube.com/watch?v=dQw4w9WgXcQ",
            ActiveHandoffLink.normalizeHttpUrl("youtube.com/watch?v=dQw4w9WgXcQ"),
        )

        assertNull(ActiveHandoffLink.normalizeHttpUrl(null))
        assertNull(ActiveHandoffLink.normalizeHttpUrl(""))
        assertNull(ActiveHandoffLink.normalizeHttpUrl("search query with spaces"))
        assertNull(ActiveHandoffLink.normalizeHttpUrl("chrome://settings"))
        assertNull(ActiveHandoffLink.normalizeHttpUrl("about:blank"))
        assertNull(ActiveHandoffLink.normalizeHttpUrl("javascript:alert(1)"))
        assertNull(ActiveHandoffLink.normalizeHttpUrl("file:///sdcard/Download/a.pdf"))
        assertNull(ActiveHandoffLink.normalizeHttpUrl("geo:37.7749,-122.4194"))
        assertNull(ActiveHandoffLink.normalizeHttpUrl("https:///path"))
        assertNull(ActiveHandoffLink.normalizeHttpUrl("http://"))
    }

    @Test
    fun resolvesMediaUrlsAndYoutubeIdsSafely() {
        assertEquals(
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            ActiveHandoffLink.mediaLinkToUrl("com.google.android.youtube", null, "dQw4w9WgXcQ"),
        )
        assertEquals(
            "https://vimeo.com/123456",
            ActiveHandoffLink.mediaLinkToUrl("com.vimeo.android", "https://vimeo.com/123456", null),
        )
        assertNull(ActiveHandoffLink.mediaLinkToUrl("com.spotify.music", null, "dQw4w9WgXcQ"))
        assertNull(ActiveHandoffLink.mediaLinkToUrl("com.google.android.youtube", "file:///local.mp4", "not-an-id"))
    }
}
