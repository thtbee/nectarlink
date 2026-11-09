// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import app.nectarlink.android.mirror.SendButton.Box
import app.nectarlink.android.mirror.SendButton.Candidate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SendButtonTest {
    private val field = Box(0, 1800, 900, 1900)

    @Test
    fun `send ids are recognized and lookalikes aren't`() {
        assertTrue(SendButton.isSendId("com.whatsapp:id/send"))
        assertTrue(SendButton.isSendId("com.example:id/send_button"))
        assertTrue(SendButton.isSendId("com.example:id/composer_send"))
        assertFalse(SendButton.isSendId("com.example:id/sender_name"))
        assertFalse(SendButton.isSendId("com.example:id/resend_code"))
        assertFalse(SendButton.isSendId(null))
        assertFalse(SendButton.isSendId("send"))
    }

    @Test
    fun `send labels match whole words in the app's languages`() {
        assertTrue(SendButton.isSendLabel("Send"))
        assertTrue(SendButton.isSendLabel(" send message "))
        assertTrue(SendButton.isSendLabel("Enviar"))
        assertTrue(SendButton.isSendLabel("发送"))
        assertFalse(SendButton.isSendLabel("Send feedback"))
        assertFalse(SendButton.isSendLabel(null))
    }

    @Test
    fun `picks the enabled send button closest to the box`() {
        val far = Candidate(null, "Send", true, Box(800, 100, 900, 200))
        val near = Candidate("com.whatsapp:id/send", null, true, Box(910, 1810, 1000, 1890))
        val disabled = Candidate(null, "Send", false, Box(905, 1800, 990, 1900))
        val other = Candidate("com.whatsapp:id/camera", "Camera", true, Box(700, 1810, 790, 1890))
        assertEquals(near, SendButton.pick(field, listOf(far, disabled, other, near)))
    }

    @Test
    fun `no send button means none is pressed`() {
        val other = Candidate("com.example:id/attach", "Attach", true, Box(910, 1810, 1000, 1890))
        assertNull(SendButton.pick(field, listOf(other)))
        assertNull(SendButton.pick(field, emptyList()))
    }
}
