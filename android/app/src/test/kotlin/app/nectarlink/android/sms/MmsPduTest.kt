// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.sms

import app.nectarlink.android.notifications.NotificationReader
import app.nectarlink.core.NotificationChatMessage
import app.nectarlink.core.SmsAttachment
import java.io.ByteArrayOutputStream
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MmsPduTest {

    @Test
    fun `writeUintvar encodes single and multi-byte values per WAP-230-WSP`() {
        fun encode(v: Int): ByteArray = ByteArrayOutputStream().also {
            PhoneSms.writeUintvar(it, v)
        }.toByteArray()

        assertArrayEquals(byteArrayOf(0x00), encode(0))
        assertArrayEquals(byteArrayOf(0x02), encode(2))
        assertArrayEquals(byteArrayOf(0x7F), encode(127))
        assertArrayEquals(byteArrayOf(0x81.toByte(), 0x00), encode(128))
        assertArrayEquals(byteArrayOf(0x82.toByte(), 0x2C), encode(300))
        assertArrayEquals(byteArrayOf(0x81.toByte(), 0x80.toByte(), 0x00), encode(16384))
    }

    @Test
    fun `buildMmsSendReqPdu encodes headers, text part, and image attachment`() {
        val imageBytes = ByteArray(300) { (it and 0xFF).toByte() }
        val pdu = PhoneSms.buildMmsSendReqPdu(
            to = listOf(" +15550100 ", "user@example.com"),
            body = "Look at this!",
            attachments = listOf(SmsAttachment(mime = "image/jpeg", data = imageBytes)),
            transactionId = "nl-1700000000",
        )

        var pos = 0
        // X-Mms-Message-Type (0x8C): m-send-req (0x80)
        assertEquals(0x8C, pdu[pos++].toInt() and 0xFF)
        assertEquals(0x80, pdu[pos++].toInt() and 0xFF)

        // X-Mms-Transaction-Id (0x98): "nl-1700000000" + NUL
        assertEquals(0x98, pdu[pos++].toInt() and 0xFF)
        val txEnd = pdu.indexOf(0, pos)
        assertEquals("nl-1700000000", String(pdu, pos, txEnd - pos, Charsets.US_ASCII))
        pos = txEnd + 1

        // X-Mms-MMS-Version (0x8D): v1.2 (0x92)
        assertEquals(0x8D, pdu[pos++].toInt() and 0xFF)
        assertEquals(0x92, pdu[pos++].toInt() and 0xFF)

        // From (0x89): length 1, Insert-address-token (0x81)
        assertEquals(0x89, pdu[pos++].toInt() and 0xFF)
        assertEquals(0x01, pdu[pos++].toInt() and 0xFF)
        assertEquals(0x81, pdu[pos++].toInt() and 0xFF)

        // First To (0x97): "+15550100/TYPE=PLMN" + NUL
        assertEquals(0x97, pdu[pos++].toInt() and 0xFF)
        val to1End = pdu.indexOf(0, pos)
        assertEquals("+15550100/TYPE=PLMN", String(pdu, pos, to1End - pos, Charsets.US_ASCII))
        pos = to1End + 1

        // Second To (0x97): "user@example.com" + NUL
        assertEquals(0x97, pdu[pos++].toInt() and 0xFF)
        val to2End = pdu.indexOf(0, pos)
        assertEquals("user@example.com", String(pdu, pos, to2End - pos, Charsets.US_ASCII))
        pos = to2End + 1

        // Content-Type (0x84): application/vnd.wap.multipart.mixed (0xA3)
        assertEquals(0x84, pdu[pos++].toInt() and 0xFF)
        assertEquals(0xA3, pdu[pos++].toInt() and 0xFF)

        // Multipart count: 2 (text + image)
        assertEquals(2, pdu[pos++].toInt() and 0xFF)

        // Part 1: text/plain (0x83)
        val textExpected = "Look at this!".toByteArray(Charsets.UTF_8)
        assertEquals(1, pdu[pos++].toInt() and 0xFF) // headersLen
        assertEquals(textExpected.size, pdu[pos++].toInt() and 0xFF) // dataLen
        assertEquals(0x83, pdu[pos++].toInt() and 0xFF) // Content-Type: text/plain
        assertArrayEquals(textExpected, pdu.copyOfRange(pos, pos + textExpected.size))
        pos += textExpected.size

        // Part 2: image/jpeg + NUL (11 bytes) and 300-byte payload (uintvar 0x82, 0x2C)
        assertEquals(11, pdu[pos++].toInt() and 0xFF) // headersLen
        assertEquals(0x82, pdu[pos++].toInt() and 0xFF) // dataLen high byte (300 = 2*128 + 44)
        assertEquals(0x2C, pdu[pos++].toInt() and 0xFF) // dataLen low byte
        val mimeEnd = pdu.indexOf(0, pos)
        assertEquals("image/jpeg", String(pdu, pos, mimeEnd - pos, Charsets.US_ASCII))
        pos = mimeEnd + 1

        assertArrayEquals(imageBytes, pdu.copyOfRange(pos, pos + imageBytes.size))
        pos += imageBytes.size
        assertEquals(pdu.size, pos)
    }

    @Test
    fun `buildMmsSendReqPdu omits text part when body is blank`() {
        val pngBytes = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47)
        val pdu = PhoneSms.buildMmsSendReqPdu(
            to = listOf("5550199"),
            body = "   ",
            attachments = listOf(SmsAttachment(mime = "image/png", data = pngBytes)),
            transactionId = "nl-42",
        )

        // Find Content-Type (0x84, 0xA3) at the end of the headers
        var idx = 0
        while (idx < pdu.size - 1 && !((pdu[idx].toInt() and 0xFF) == 0x84 && (pdu[idx + 1].toInt() and 0xFF) == 0xA3)) {
            idx++
        }
        assertTrue(idx < pdu.size - 2)
        var pos = idx + 2

        // Multipart count: 1 (image only)
        assertEquals(1, pdu[pos++].toInt() and 0xFF)
        assertEquals(10, pdu[pos++].toInt() and 0xFF) // "image/png\0" length
        assertEquals(pngBytes.size, pdu[pos++].toInt() and 0xFF)
        val mimeEnd = pdu.indexOf(0, pos)
        assertEquals("image/png", String(pdu, pos, mimeEnd - pos, Charsets.US_ASCII))
        pos = mimeEnd + 1
        assertArrayEquals(pngBytes, pdu.copyOfRange(pos, pdu.size))
    }

    @Test
    fun `buildConversation extracts 1-to-1 and group MessagingStyle metadata with bounds`() {
        val smallAvatar = ByteArray(1024) { 1 }
        val oversizedAvatar = ByteArray(20 * 1024) { 2 }

        val msgs = (1..30).map { i ->
            NotificationChatMessage(
                sender = if (i % 2 == 0) "Me" else "Alice",
                text = "Message $i",
                time = 1_700_000_000_000L + i * 1000L,
                selfSent = i % 2 == 0,
                avatar = if (i % 2 == 0) oversizedAvatar else smallAvatar,
            )
        }

        val conv = NotificationReader.buildConversation(
            styleTitle = null,
            fallbackTitle = "Fallback",
            isGroup = false,
            convAvatar = smallAvatar,
            messages = msgs,
        )
        assertNotNull(conv)
        assertEquals("Alice", conv!!.title)
        assertFalse(conv.group)
        assertEquals(25, conv.messages.size)
        assertEquals("Message 6", conv.messages.first().text)
        assertEquals("Message 30", conv.messages.last().text)
        // Self-sent messages drop sender and oversized avatars
        assertTrue(conv.messages.last().selfSent)
        assertNull(conv.messages.last().sender)
        assertNull(conv.messages.last().avatar)

        val groupConv = NotificationReader.buildConversation(
            styleTitle = "Family Chat",
            fallbackTitle = "Alice",
            isGroup = true,
            convAvatar = oversizedAvatar,
            messages = msgs.take(2),
        )
        assertNotNull(groupConv)
        assertEquals("Family Chat", groupConv!!.title)
        assertTrue(groupConv.group)
        assertNull(groupConv.avatar)
    }

    private fun ByteArray.indexOf(byte: Byte, startIndex: Int): Int {
        for (i in startIndex until size) {
            if (this[i] == byte) return i
        }
        return -1
    }
}
