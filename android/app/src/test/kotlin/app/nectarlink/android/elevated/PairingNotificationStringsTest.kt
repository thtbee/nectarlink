// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test

/**
 * A collapsed pairing notification shows only its hint, so the hint has to
 * name the button exactly as that language's notification labels it.
 */
class PairingNotificationStringsTest {
    private val res = listOf(File("src/main/res"), File("app/src/main/res")).first { it.isDirectory }

    private fun string(xml: String, name: String): String? =
        Regex("""<string name="$name">(.*?)</string>""").find(xml)?.groupValues?.get(1)
            ?.replace("\\'", "'")

    @Test
    fun `every language's collapsed hint names its Enter code button`() {
        val folders = res.listFiles { f -> f.isDirectory && f.name.startsWith("values") }.orEmpty()
            .filter { File(it, "strings.xml").let { s -> s.isFile && s.readText().contains("name=\"elevated_pair_action\"") } }
        assertTrue("no strings found under $res", folders.size > 1)
        for (folder in folders) {
            val xml = File(folder, "strings.xml").readText()
            val action = string(xml, "elevated_pair_action")!!
            val hint = string(xml, "elevated_pair_collapsed") ?: fail("${folder.name} has no elevated_pair_collapsed")
            assertTrue("${folder.name}: \"$hint\" doesn't mention \"$action\"", hint.toString().contains(action))
        }
    }
}
