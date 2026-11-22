// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.settings

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class SettingsSearchTest {
    private val entries = listOf(
        SettingsSearchEntry("clipboard", 1),
        SettingsSearchEntry("clipboard", 2),
        SettingsSearchEntry("remote", 3),
        SettingsSearchEntry("control", 4),
        SettingsSearchEntry("control", 5),
    )
    private val labels = mapOf(
        1 to "Clipboard",
        2 to "Sync copied passwords",
        3 to "Volume buttons control your PC",
        4 to "Control from your PC",
        5 to "Clipboard",
    )
    private val sections = mapOf("clipboard" to "Clipboard", "remote" to "Remote", "control" to "Control your PC")

    private fun search(query: String) =
        searchSettings(query, entries, { sections[it].orEmpty() }, { labels.getValue(it) }).map { it.title }

    @Test
    fun findsByEveryWordInTitleOrSectionAnyCase() {
        assertEquals(listOf(2), search("PASSWORDS"))
        // 5 is in the "Control your PC" section, so the section name finds it too.
        assertEquals(listOf(3, 4, 5), search("control pc"))
        // A section's name finds what's in it; a title shows once.
        assertEquals(listOf(1, 2), search("clipboard"))
        assertTrue(search("   ").isEmpty())
        assertTrue(search("bluetooth").isEmpty())
    }

    @Test
    fun everySectionHasAPlaceInItsCategory() {
        SETTINGS_SEARCH_INDEX.map { it.section }.toSet().forEach { section ->
            val (category, index) = settingsSectionPlace(section)
            assertTrue(section, category != "all" && index > 0)
        }
    }
}
