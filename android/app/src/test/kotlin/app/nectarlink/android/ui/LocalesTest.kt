// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.w3c.dom.Element
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory

/**
 * The languages the app offers (res/xml/locales_config.xml) are the ones its own picker
 * lists, and each is fully translated. Languages still being translated may sit in res/
 * partly done (lint's MissingTranslation is off for that); they just aren't offered yet.
 */
class LocalesTest {

    private val res: File = listOf(File("src/main/res"), File("app/src/main/res"), File("android/app/src/main/res"))
        .firstOrNull { it.isDirectory }
        ?: error("Could not locate src/main/res from ${File(".").absolutePath}")

    private fun offered(): List<String> {
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(File(res, "xml/locales_config.xml"))
        val nodes = doc.getElementsByTagName("locale")
        return (0 until nodes.length).map { (nodes.item(it) as Element).getAttribute("android:name") }
    }

    /** A BCP 47 tag's resource folder: "zh-CN" is values-zh-rCN, "de" is values-de. */
    private fun valuesFolder(tag: String): String {
        val parts = tag.split('-')
        return if (parts.size == 1) "values-${parts[0]}" else "values-${parts[0]}-r${parts[1]}"
    }

    /** Translatable string and plural names in a strings.xml. */
    private fun names(file: File): Pair<Map<String, String>, Set<String>> {
        val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(file)
        val strings = linkedMapOf<String, String>()
        val stringNodes = doc.getElementsByTagName("string")
        for (i in 0 until stringNodes.length) {
            val el = stringNodes.item(i) as Element
            if (el.getAttribute("translatable") == "false") continue
            strings[el.getAttribute("name")] = el.textContent
        }
        val plurals = linkedSetOf<String>()
        val pluralNodes = doc.getElementsByTagName("plurals")
        for (i in 0 until pluralNodes.length) {
            val el = pluralNodes.item(i) as Element
            if (el.getAttribute("translatable") == "false") continue
            plurals += el.getAttribute("name")
        }
        return strings to plurals
    }

    @Test
    fun `the app's language picker offers exactly the languages Android is told about`() {
        assertEquals(offered(), AppLanguage.languages.map { it.first })
        assertEquals("de", AppLanguage.normalize("de"))
        assertEquals("de", AppLanguage.normalize("de-DE"))
        assertEquals("de", AppLanguage.normalize("de-AT"))
        assertEquals("de", AppLanguage.normalize("de-CH"))
        assertEquals("tr", AppLanguage.normalize("tr"))
        assertEquals("tr", AppLanguage.normalize("tr-TR"))
    }

    @Test
    fun `every offered language is fully translated`() {
        val (englishStrings, englishPlurals) = names(File(res, "values/strings.xml"))
        for (tag in offered().filter { it != "en" }) {
            val file = File(res, "${valuesFolder(tag)}/strings.xml")
            assertTrue("$tag is offered but has no ${file.path}", file.isFile)
            val (strings, plurals) = names(file)
            val missing = englishStrings.keys.filter { strings[it].isNullOrBlank() }
            assertTrue("$tag is offered but misses ${missing.size} strings: ${missing.take(20)}", missing.isEmpty())
            val missingPlurals = englishPlurals - plurals
            assertTrue("$tag is offered but misses plurals: $missingPlurals", missingPlurals.isEmpty())
        }
    }
}
