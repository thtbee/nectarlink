// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.view.KeyEvent
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class DeviceControlTest {
    @Test
    fun theKeyboardsNamedKeysMapToTheTabletsKeys() {
        assertEquals(KeyEvent.KEYCODE_ENTER, DeviceControl.keyCode("enter"))
        assertEquals(KeyEvent.KEYCODE_DEL, DeviceControl.keyCode("backspace"))
        assertEquals(KeyEvent.KEYCODE_FORWARD_DEL, DeviceControl.keyCode("delete"))
        assertEquals(KeyEvent.KEYCODE_MOVE_HOME, DeviceControl.keyCode("home"))
        assertEquals(KeyEvent.KEYCODE_PAGE_DOWN, DeviceControl.keyCode("page_down"))
        assertEquals(KeyEvent.KEYCODE_PASTE, DeviceControl.keyCode("paste"))
        // The Windows key goes to the tablet's own home screen.
        assertEquals(KeyEvent.KEYCODE_HOME, DeviceControl.keyCode("win"))
    }

    @Test
    fun functionKeysOneToTwelve() {
        assertEquals(KeyEvent.KEYCODE_F1, DeviceControl.keyCode("f1"))
        assertEquals(KeyEvent.KEYCODE_F12, DeviceControl.keyCode("f12"))
        assertNull(DeviceControl.keyCode("f13"))
        assertNull(DeviceControl.keyCode("fx"))
    }

    @Test
    fun unknownKeysAreLeftAlone() {
        assertNull(DeviceControl.keyCode("volume_up"))
        assertNull(DeviceControl.keyCode(""))
    }
}
