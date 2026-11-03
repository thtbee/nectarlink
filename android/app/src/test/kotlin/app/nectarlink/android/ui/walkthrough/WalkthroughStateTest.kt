// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.walkthrough

import app.nectarlink.android.core.CoreState
import app.nectarlink.android.ui.guide.FeatureArea
import app.nectarlink.android.ui.guide.FeatureLiveBadge
import app.nectarlink.android.ui.guide.FeaturesGuideCatalog
import app.nectarlink.android.ui.guide.GuideFixAction
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class WalkthroughStateTest {
    @Test
    fun walkthroughStepsFollowExpectedOrderAndStorageKeys() {
        val steps = WalkthroughStep.entries
        assertEquals(5, steps.size)
        assertEquals(WalkthroughStep.Welcome, steps.first())
        assertEquals(WalkthroughStep.Done, steps.last())

        assertNull(WalkthroughStep.Welcome.previous)
        assertEquals(WalkthroughStep.Permissions, WalkthroughStep.Welcome.next)
        assertEquals(WalkthroughStep.Pair, WalkthroughStep.Permissions.next)
        assertEquals(WalkthroughStep.PowerLevel, WalkthroughStep.Pair.next)
        assertEquals(WalkthroughStep.Done, WalkthroughStep.PowerLevel.next)
        assertNull(WalkthroughStep.Done.next)

        for (step in steps) {
            assertEquals(step, WalkthroughStep.fromIndex(step.index))
        }
        assertEquals(WalkthroughStep.Welcome, WalkthroughStep.fromIndex(-1))
        assertEquals(WalkthroughStep.Done, WalkthroughStep.fromIndex(99))
    }

    @Test
    fun resolveStartupWalkthroughHandlesFreshInstallResumeAndExistingUsers() {
        // 1. Fresh install with no paired devices starts at Welcome.
        val fresh = resolveStartupWalkthrough(
            persistence = WalkthroughPersistence(
                completed = false,
                currentStepIndex = WalkthroughStep.Welcome.index,
                whatsNewGuideDismissed = false,
            ),
            hasPairedDevices = false,
        )
        assertTrue(fresh.showWalkthrough)
        assertEquals(WalkthroughStep.Welcome, fresh.initialStep)
        assertFalse(fresh.markCompletedAutomatically)
        assertFalse(fresh.showWhatsNewGuideBanner)

        // 2. Resuming an interrupted walkthrough restores the saved step.
        val resumed = resolveStartupWalkthrough(
            persistence = WalkthroughPersistence(
                completed = false,
                currentStepIndex = WalkthroughStep.Pair.index,
                whatsNewGuideDismissed = false,
            ),
            hasPairedDevices = false,
        )
        assertTrue(resumed.showWalkthrough)
        assertEquals(WalkthroughStep.Pair, resumed.initialStep)
        assertFalse(resumed.markCompletedAutomatically)

        // 3. Existing user upgrading with paired devices silently completes and shows What's New banner.
        val upgradeWithDevices = resolveStartupWalkthrough(
            persistence = WalkthroughPersistence(
                completed = false,
                currentStepIndex = WalkthroughStep.Welcome.index,
                whatsNewGuideDismissed = false,
            ),
            hasPairedDevices = true,
        )
        assertFalse(upgradeWithDevices.showWalkthrough)
        assertTrue(upgradeWithDevices.markCompletedAutomatically)
        assertTrue(upgradeWithDevices.showWhatsNewGuideBanner)

        // 3b. Active first-run walkthrough stays open at Welcome even if a PC pairs via deep link.
        val pairedDuringActiveWelcome = resolveStartupWalkthrough(
            persistence = WalkthroughPersistence(
                completed = false,
                currentStepIndex = WalkthroughStep.Welcome.index,
                whatsNewGuideDismissed = false,
            ),
            hasPairedDevices = true,
            walkthroughAlreadyActive = true,
        )
        assertTrue(pairedDuringActiveWelcome.showWalkthrough)
        assertEquals(WalkthroughStep.Welcome, pairedDuringActiveWelcome.initialStep)
        assertFalse(pairedDuringActiveWelcome.markCompletedAutomatically)

        // 4. Completed walkthrough shows What's New banner until dismissed.
        val completed = resolveStartupWalkthrough(
            persistence = WalkthroughPersistence(
                completed = true,
                currentStepIndex = WalkthroughStep.Done.index,
                whatsNewGuideDismissed = false,
            ),
            hasPairedDevices = true,
        )
        assertFalse(completed.showWalkthrough)
        assertFalse(completed.markCompletedAutomatically)
        assertTrue(completed.showWhatsNewGuideBanner)

        val dismissed = resolveStartupWalkthrough(
            persistence = WalkthroughPersistence(
                completed = true,
                currentStepIndex = WalkthroughStep.Done.index,
                whatsNewGuideDismissed = true,
            ),
            hasPairedDevices = true,
        )
        assertFalse(dismissed.showWalkthrough)
        assertFalse(dismissed.showWhatsNewGuideBanner)
    }

    @Test
    fun featuresGuideCatalogCoversAllAreasAndRespectsWritingRules() {
        val entries = FeaturesGuideCatalog.entries
        assertTrue(entries.size >= 22)

        val coveredAreas = entries.map { it.area }.toSet()
        for (area in FeatureArea.entries.filter { it != FeatureArea.All }) {
            assertTrue("Missing guide entries for area ${area.id}", area in coveredAreas)
        }

        val bannedWords = listOf(
            "seamless",
            "seamlessly",
            "effortless",
            "effortlessly",
            "magic",
            "magical",
            "unleash",
            "supercharge",
            "elevate",
            "game-changer",
            "revolutionary",
            "next-level",
            "delve",
            "robust",
            "cutting-edge",
            "in today's world",
            "whether you're",
        )

        fun assertCleanCopy(context: String, text: String) {
            assertFalse("$context contains em dash: $text", text.contains('\u2014'))
            assertFalse("$context contains en dash: $text", text.contains('\u2013'))
            assertFalse("$context contains exclamation mark: $text", text.contains('!'))
            assertFalse("$context contains ASCII arrow: $text", text.contains("->"))
            for (word in bannedWords) {
                val pattern = Regex("\\b${Regex.escape(word)}\\b", RegexOption.IGNORE_CASE)
                assertFalse("$context contains banned word '$word': $text", pattern.containsMatchIn(text))
            }
        }

        for (step in WalkthroughStep.entries) {
            assertCleanCopy("WalkthroughStep.${step.name}", step.title)
        }
        for (area in FeatureArea.entries) {
            assertNotEquals(0, area.labelRes)
        }
        for (fix in GuideFixAction.entries) {
            assertNotEquals(0, fix.buttonLabelRes)
        }

        val emptyState = CoreState()
        for (entry in entries) {
            assertTrue(entry.id.isNotBlank())
            assertNotEquals(0, entry.titleRes)
            assertNotEquals(0, entry.summaryRes)
            assertNotEquals(0, entry.whereTextRes)
            assertNotEquals(0, entry.needsSummaryRes)
            assertTrue(entry.whatYouNeedResIds.isNotEmpty())
            assertTrue(entry.stepByStepPhoneResIds.isNotEmpty())
            assertTrue(entry.stepByStepPcResIds.isNotEmpty())
            assertTrue(entry.optionsExplainedResIds.isNotEmpty())
            assertNotEquals(0, entry.dataAndPrivacyRes)
            assertNotEquals(0, entry.limitsRes)
            assertTrue(entry.keywords.isNotEmpty())

            val badge = entry.evaluateBadge(emptyState)
            assertNotEquals(0, badge.labelRes)
        }

        // LocalSend does not require a paired PC, so its badge is Available on default state.
        val localSendEntry = entries.first { it.id == "localsend_lan" }
        assertEquals(FeatureLiveBadge.Available, localSendEntry.evaluateBadge(emptyState))

        // Verify walkthrough and guide string resources in strings.xml obey all writing rules
        val stringsXmlCandidates = listOf(
            File("src/main/res/values/strings.xml"),
            File("app/src/main/res/values/strings.xml"),
            File("android/app/src/main/res/values/strings.xml"),
        )
        val stringsXml = stringsXmlCandidates.firstOrNull { it.exists() }
        if (stringsXml != null) {
            val doc = DocumentBuilderFactory.newInstance().newDocumentBuilder().parse(stringsXml)
            val nodeList = doc.getElementsByTagName("string")
            val stringMap = mutableMapOf<String, String>()
            for (i in 0 until nodeList.length) {
                val node = nodeList.item(i)
                val name = node.attributes?.getNamedItem("name")?.nodeValue ?: continue
                val value = node.textContent ?: ""
                stringMap[name] = value
                if (name.startsWith("walkthrough_") || name.startsWith("guide_")) {
                    assertCleanCopy("strings.xml[$name]", value)
                }
                if (name.endsWith("_needs") && name.startsWith("guide_")) {
                    assertFalse(
                        "strings.xml[$name] should not list baseline Standard power level: $value",
                        value.contains("Standard power level", ignoreCase = true),
                    )
                }
            }

            val expectedTitles = listOf(
                "Notifications",
                "Clipboard",
                "Send files",
                "Send queue",
                "Now playing",
                "Messages",
                "Calls",
                "Photos",
                "Screen mirroring",
                "App streams",
                "Phone as webcam",
                "Continuity camera",
                "Voice recorder",
                "Control PC",
                "Deck",
                "Desk Mode",
                "Quick actions",
                "Browse PC files",
                "Wake PC",
                "Elevated",
                "Assist",
                "Connection Doctor",
            )
            val actualGuideTitles = stringMap
                .filterKeys { it.startsWith("guide_") && it.endsWith("_title") }
                .values
                .toSet()
            for (expected in expectedTitles) {
                assertTrue(
                    "Missing exact in-app feature title '$expected' in guide_*_title strings",
                    expected in actualGuideTitles,
                )
            }
        }
    }
}
