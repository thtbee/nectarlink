// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.theme

import app.nectarlink.android.ui.Preferences
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.w3c.dom.Element

class ThemeAndLocalizationTest {

    @Test
    fun `sanitizeAppearance accepts valid themes, modes, and seeds and falls back on unknown or null values`() {
        val themes = listOf("hive", "bloom", "graphite", "glass")
        val modes = listOf("system", "light", "dark")
        val seeds = Tokens.bloomSeeds.keys + "artwork"

        for (theme in themes) {
            for (mode in modes) {
                for (seed in seeds) {
                    val sanitized = Preferences.sanitizeAppearance(
                        Appearance(theme = theme, mode = mode, seed = seed),
                    )
                    assertEquals(theme, sanitized.theme)
                    assertEquals(mode, sanitized.mode)
                    assertEquals(seed, sanitized.seed)
                }
            }
        }

        val defaults = Appearance()

        val fromUnknown = Preferences.sanitizeAppearance(
            Appearance(
                theme = "unknown_theme",
                mode = "unknown_mode",
                seed = "unknown_seed",
                dynamicColor = false,
                amoled = true,
                combMotion = false,
            ),
        )
        assertEquals(defaults.theme, fromUnknown.theme)
        assertEquals(defaults.mode, fromUnknown.mode)
        assertEquals(defaults.seed, fromUnknown.seed)
        assertFalse(fromUnknown.dynamicColor)
        assertTrue(fromUnknown.amoled)
        assertFalse(fromUnknown.combMotion)

        val fromNulls = Preferences.sanitizeAppearance(
            theme = null,
            mode = null,
            seed = null,
        )
        assertEquals(defaults, fromNulls)
    }

    @Test
    fun `tokens define distinct light and dark palettes for Hive, Graphite, and all Bloom seeds`() {
        assertNotEquals(Tokens.hiveWax.surface, Tokens.hiveComb.surface)
        assertNotEquals(Tokens.hiveWax.onSurface, Tokens.hiveComb.onSurface)
        assertNotEquals(Tokens.graphitePaper.surface, Tokens.graphiteSlate.surface)
        assertNotEquals(Tokens.graphitePaper.onSurface, Tokens.graphiteSlate.onSurface)

        assertTrue(Tokens.bloomSeeds.containsKey(Tokens.DEFAULT_SEED))
        for ((name, seed) in Tokens.bloomSeeds) {
            assertNotNull("seed '$name' missing light palette", seed.light)
            assertNotNull("seed '$name' missing dark palette", seed.dark)
            assertNotEquals("seed '$name' light and dark surfaces must differ", seed.light.surface, seed.dark.surface)
            assertNotEquals("seed '$name' light and dark onSurface must differ", seed.light.onSurface, seed.dark.onSurface)
        }
    }

    @Test
    fun `Bloom Home card color follows the theme by default and resolves a saved preset in both modes`() {
        val default = Appearance(theme = "bloom")
        assertEquals(null, default.bloomHomeCardPalette(dark = false))
        assertEquals(null, Preferences.sanitizeAppearance(default.copy(homeCardSeed = "artwork")).homeCardSeed)

        for ((name, preset) in Tokens.bloomSeeds) {
            val appearance = Preferences.sanitizeAppearance(default.copy(homeCardSeed = name))
            assertEquals(name, appearance.homeCardSeed)
            assertEquals(preset.light, appearance.bloomHomeCardPalette(dark = false))
            assertEquals(preset.dark, appearance.bloomHomeCardPalette(dark = true))
            assertEquals(null, appearance.copy(theme = "glass").bloomHomeCardPalette(dark = true))
        }
    }

    private fun resolveResDir(): File {
        val candidates = listOf(
            File("src/main/res"),
            File("app/src/main/res"),
            File("android/app/src/main/res"),
        )
        return candidates.firstOrNull { it.isDirectory }
            ?: error("Could not locate src/main/res from working directory ${File(".").absolutePath}")
    }

    private fun parsePluralsXml(file: File): Map<String, Map<String, String>> {
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)
        doc.documentElement.normalize()

        val plurals = linkedMapOf<String, Map<String, String>>()
        val pluralNodes = doc.getElementsByTagName("plurals")
        for (i in 0 until pluralNodes.length) {
            val el = pluralNodes.item(i) as? Element ?: continue
            if (el.getAttribute("translatable") == "false") continue
            val name = el.getAttribute("name")
            val items = linkedMapOf<String, String>()
            val itemNodes = el.getElementsByTagName("item")
            for (j in 0 until itemNodes.length) {
                val item = itemNodes.item(j) as? Element ?: continue
                items[item.getAttribute("quantity")] = item.textContent
            }
            if (name.isNotEmpty()) {
                plurals[name] = items
            }
        }
        return plurals
    }

    private val placeholderRegex = Regex("""%(?:\d+\$)?(?:\.\d+)?[sdf]""")

    private fun extractPlaceholders(text: String): Set<String> =
        placeholderRegex.findAll(text.replace("%%", "")).map { it.value }.toSet()

    @Test
    fun `Simplified Chinese localization covers all translatable plurals with matching placeholders`() {
        val resDir = resolveResDir()
        val enFile = File(resDir, "values/strings.xml")
        val zhFile = File(resDir, "values-zh-rCN/strings.xml")
        assertTrue("values/strings.xml must exist", enFile.isFile)
        assertTrue("values-zh-rCN/strings.xml must exist", zhFile.isFile)

        val enPlurals = parsePluralsXml(enFile)
        val zhPlurals = parsePluralsXml(zhFile)

        assertTrue("Expected plurals in values/strings.xml", enPlurals.isNotEmpty())

        val missingPlurals = enPlurals.keys - zhPlurals.keys
        assertTrue("Missing zh-rCN plural translations: $missingPlurals", missingPlurals.isEmpty())

        for ((key, enItems) in enPlurals) {
            val zhItems = zhPlurals.getValue(key)
            assertTrue("Empty zh-rCN plural items for '$key'", zhItems.isNotEmpty())
            val expectedPlaceholders = enItems.values.flatMap { extractPlaceholders(it) }.toSet()
            for ((qty, zhText) in zhItems) {
                assertFalse("Blank zh-rCN plural '$key' quantity '$qty'", zhText.isBlank())
                assertEquals(
                    "Placeholder mismatch in plural '$key' quantity '$qty'",
                    expectedPlaceholders,
                    extractPlaceholders(zhText),
                )
            }
        }
    }
}
