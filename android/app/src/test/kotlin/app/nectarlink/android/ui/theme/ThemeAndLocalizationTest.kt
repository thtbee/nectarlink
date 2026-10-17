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
    fun `theme switching supports all themes, color modes, seeds, and modifiers`() {
        val themes = listOf("hive", "bloom", "graphite", "glass")
        val modes = listOf("system", "light", "dark")
        val seeds = Tokens.bloomSeeds.keys + "artwork"

        for (theme in themes) {
            for (mode in modes) {
                for (seed in seeds) {
                    for (dynamicColor in listOf(false, true)) {
                        for (amoled in listOf(false, true)) {
                            for (combMotion in listOf(false, true)) {
                                val sanitized = Preferences.sanitizeAppearance(
                                    Appearance(
                                        theme = theme,
                                        mode = mode,
                                        seed = seed,
                                        dynamicColor = dynamicColor,
                                        amoled = amoled,
                                        combMotion = combMotion,
                                    ),
                                )
                                assertEquals(theme, sanitized.theme)
                                assertEquals(mode, sanitized.mode)
                                assertEquals(seed, sanitized.seed)
                                assertEquals(dynamicColor, sanitized.dynamicColor)
                                assertEquals(amoled, sanitized.amoled)
                                assertEquals(combMotion, sanitized.combMotion)
                            }
                        }
                    }
                }
            }
        }
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

    private fun resolveResDir(): File {
        val candidates = listOf(
            File("src/main/res"),
            File("app/src/main/res"),
            File("android/app/src/main/res"),
        )
        return candidates.firstOrNull { it.isDirectory }
            ?: error("Could not locate src/main/res from working directory ${File(".").absolutePath}")
    }

    private fun parseStringsXml(file: File): Pair<Map<String, String>, Map<String, Map<String, String>>> {
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)
        doc.documentElement.normalize()

        val strings = linkedMapOf<String, String>()
        val stringNodes = doc.getElementsByTagName("string")
        for (i in 0 until stringNodes.length) {
            val el = stringNodes.item(i) as? Element ?: continue
            if (el.parentNode != doc.documentElement) continue
            if (el.getAttribute("translatable") == "false") continue
            val name = el.getAttribute("name")
            if (name.isNotEmpty()) {
                strings[name] = el.textContent
            }
        }

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
        return strings to plurals
    }

    private val placeholderRegex = Regex("""%(?:\d+\$)?(?:\.\d+)?[sdf]""")

    private fun extractPlaceholders(text: String): Set<String> =
        placeholderRegex.findAll(text.replace("%%", "")).map { it.value }.toSet()

    @Test
    fun `Simplified Chinese localization covers all translatable strings and plurals with matching placeholders`() {
        val resDir = resolveResDir()
        val enFile = File(resDir, "values/strings.xml")
        val zhFile = File(resDir, "values-zh-rCN/strings.xml")
        assertTrue("values/strings.xml must exist", enFile.isFile)
        assertTrue("values-zh-rCN/strings.xml must exist", zhFile.isFile)

        val (enStrings, enPlurals) = parseStringsXml(enFile)
        val (zhStrings, zhPlurals) = parseStringsXml(zhFile)

        assertTrue("Expected at least 1000 strings in values/strings.xml", enStrings.size >= 1000)
        assertTrue("Expected plurals in values/strings.xml", enPlurals.isNotEmpty())

        val missingStrings = enStrings.keys - zhStrings.keys
        assertTrue("Missing zh-rCN string translations: $missingStrings", missingStrings.isEmpty())

        for ((key, enVal) in enStrings) {
            val zhVal = zhStrings.getValue(key)
            assertFalse("Empty zh-rCN string for key '$key'", zhVal.isBlank())
            assertEquals(
                "Placeholder mismatch in string '$key'",
                extractPlaceholders(enVal),
                extractPlaceholders(zhVal),
            )
        }

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
