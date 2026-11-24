// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import app.nectarlink.android.BuildConfig
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class LinksTest {
    @Test
    fun the_links_are_pinned() {
        assertEquals("https://discord.gg/AA2Dp8PYqf", Link.DISCORD.url)
        assertEquals("https://x.com/thtbee_", Link.X.url)
        if (BuildConfig.PLAY) {
            // Play: no donations, no link to the code and its APKs.
            listOf(Link.DONATE, Link.REPO, Link.RELEASES).forEach { assertEquals(it.name, "", it.url) }
        } else {
            assertEquals("https://buymeacoffee.com/thtbee_", Link.DONATE.url)
            assertEquals("https://github.com/thtbee/nectarlink", Link.REPO.url)
            assertEquals("https://github.com/thtbee/nectarlink/releases", Link.RELEASES.url)
        }
    }

    @Test
    fun only_https_on_known_hosts() {
        Link.entries.filter { it.url.isNotEmpty() }.forEach { assertTrue(it.name, Link.allowed(it.url)) }
        assertFalse("an empty link isn't opened", Link.allowed(""))
        assertFalse(Link.allowed("http://discord.gg/AA2Dp8PYqf"))
        assertFalse(Link.allowed("https://discord.gg.evil.example/x"))
        assertFalse(Link.allowed("https://evil.example/?discord.gg"))
        assertFalse(Link.allowed("intent://x#Intent;end"))
    }

    /** Translations come in as pull requests: none may carry a link. */
    @Test
    fun no_string_resource_has_a_link() {
        val files = File("src/main/res").listFiles { f -> f.name.startsWith("values") }.orEmpty()
            .map { File(it, "strings.xml") }.filter { it.exists() }
        assertTrue("found the strings", files.size >= 10)
        for (file in files) {
            val text = file.readText()
            for (word in listOf("://", "www.", "href", "discord.gg")) {
                assertFalse("${file.path} has \"$word\"", text.contains(word))
            }
        }
    }
}
