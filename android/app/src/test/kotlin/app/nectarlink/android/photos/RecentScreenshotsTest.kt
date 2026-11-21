// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.photos

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class RecentScreenshotsTest {
    @Test
    fun onlyRecognizesFreshScreenshotsFromStandardAlbums() {
        val now = 100_000L
        assertTrue(isFreshScreenshot("Screenshots", "Screenshot_20261121.png", now - 1, now - 4_000, now))
        assertFalse(isFreshScreenshot("Camera", "Screenshot_20261121.png", now - 1, 0, now))
        assertFalse(isFreshScreenshot("Screenshots", "Screenshot_imported.png", null, 0, now))
        assertFalse(isFreshScreenshot("Screenshots", "IMG_20261121.jpg", now - 1, 0, now))
        assertFalse(isFreshScreenshot("Screenshots", "Screenshot_20261121.jpg", now - 6_000, now - 5_000, now))
        assertFalse(isFreshScreenshot("Screenshots", "Screenshot_20261121.jpg", now - 30_000, 0, now))
    }
}
