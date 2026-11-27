// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.pairing

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingLayoutTest {
    @Test
    fun layoutModeKeepsPortraitAndAdaptsLandscapeAndCompactHeight() {
        assertEquals(PairingLayoutMode.Portrait, choosePairingLayout(360f, 740f))
        assertEquals(PairingLayoutMode.Portrait, choosePairingLayout(320f, 568f))
        assertEquals(PairingLayoutMode.TwoPaneLandscape, choosePairingLayout(740f, 340f))
        assertEquals(PairingLayoutMode.TwoPaneLandscape, choosePairingLayout(544f, 290f))
        assertEquals(PairingLayoutMode.TwoPaneLandscape, choosePairingLayout(1200f, 760f))
        assertEquals(PairingLayoutMode.CompactScrollable, choosePairingLayout(360f, 440f))
        assertEquals(PairingLayoutMode.CompactScrollable, choosePairingLayout(440f, 380f))
    }

    @Test
    fun squareSideFitsBothPortraitAndLandscapeConstraints() {
        // Portrait preserves 100% width for scanner and 80% width for shown QR code.
        assertEquals(312f, fitSquareSideDp(312f, 388f), 0.01f)
        assertEquals(249.6f, fitSquareSideDp(312f, 400f, widthFraction = 0.8f), 0.01f)

        // Landscape caps the square to the available height instead of clipping.
        assertEquals(280f, fitSquareSideDp(350f, 280f), 0.01f)
        assertEquals(280f, fitSquareSideDp(350f, 280f, widthFraction = 1f), 0.01f)

        // Unbounded height (inside scroll) falls back to width fraction cleanly.
        assertEquals(240f, fitSquareSideDp(300f, Float.POSITIVE_INFINITY, widthFraction = 0.8f), 0.01f)
    }

    @Test
    fun codeDigitsKeepDefaultSizeWhenRoomyAndShrinkOnNarrowPanes() {
        assertEquals(44f, codeDigitWidthDp(312f), 0.01f)
        val narrow = codeDigitWidthDp(250f)
        assertTrue("Expected narrow digit width < 44dp, got $narrow", narrow < 44f)
        assertTrue("6 digits + 5*8dp gaps must fit in 250dp", narrow * 6f + 8f * 5f <= 250.01f)
    }
}
