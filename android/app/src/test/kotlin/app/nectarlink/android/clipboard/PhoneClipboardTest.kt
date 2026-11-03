// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.clipboard

import app.nectarlink.android.clipboard.PhoneClipboard.ClipCandidate
import app.nectarlink.android.clipboard.PhoneClipboard.ItemChoice
import app.nectarlink.android.files.OutgoingFiles
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PhoneClipboardTest {

    @Test
    fun `selectClipItem prefers item text over URI even when clip description advertises an image`() {
        val choice = PhoneClipboard.selectClipItem(
            items = listOf(
                ClipCandidate(
                    text = "Caption text",
                    uri = "content://media/external/images/1",
                    uriMime = { "image/jpeg" },
                ),
            ),
            descHasImage = true,
        )
        assertEquals(ItemChoice.Text(index = 0, text = "Caption text", coerced = false), choice)
    }

    @Test
    fun `selectClipItem returns Image when item has an image URI and no direct text`() {
        val byResolverMime = PhoneClipboard.selectClipItem(
            items = listOf(
                ClipCandidate(
                    text = null,
                    uri = "content://media/external/images/2",
                    uriMime = { "image/png" },
                ),
            ),
            descHasImage = false,
        )
        assertEquals(ItemChoice.Image("content://media/external/images/2"), byResolverMime)

        val byClipDescription = PhoneClipboard.selectClipItem(
            items = listOf(
                ClipCandidate(
                    text = null,
                    uri = "content://provider/untyped-image",
                    uriMime = { null },
                ),
            ),
            descHasImage = true,
        )
        assertEquals(ItemChoice.Image("content://provider/untyped-image"), byClipDescription)
    }

    @Test
    fun `selectClipItem never coerces a non-image URI item to text`() {
        var coerceCalled = false
        val choice = PhoneClipboard.selectClipItem(
            items = listOf(
                ClipCandidate(
                    text = null,
                    uri = "content://downloads/document.pdf",
                    uriMime = { "application/pdf" },
                    coercedText = {
                        coerceCalled = true
                        "content://downloads/document.pdf"
                    },
                ),
            ),
            descHasImage = false,
        )
        assertEquals(ItemChoice.Empty, choice)
        assertFalse("coerceToText must not be called for URI-backed items", coerceCalled)

        val withImageDesc = PhoneClipboard.selectClipItem(
            items = listOf(
                ClipCandidate(
                    text = null,
                    uri = "content://downloads/document.pdf",
                    uriMime = { "application/pdf" },
                ),
            ),
            descHasImage = true,
        )
        assertEquals(ItemChoice.Empty, withImageDesc)
    }

    @Test
    fun `selectClipItem falls back to coercedText only for items without a URI`() {
        val choice = PhoneClipboard.selectClipItem(
            items = listOf(
                ClipCandidate<String>(
                    text = null,
                    uri = null,
                    coercedText = { "coerced intent text" },
                ),
            ),
            descHasImage = false,
        )
        assertEquals(ItemChoice.Text(index = 0, text = "coerced intent text", coerced = true), choice)

        val echoed = PhoneClipboard.selectClipItem(
            items = listOf(
                ClipCandidate<String>(
                    text = null,
                    uri = null,
                    coercedText = { "coerced intent text" },
                ),
            ),
            descHasImage = false,
            isTextEcho = { it == "coerced intent text" },
        )
        assertEquals(ItemChoice.Empty, echoed)
    }

    @Test
    fun `SendActivity and OutgoingFiles reject file URIs, blocked authorities, and ungranted streams`() {
        assertFalse(OutgoingFiles.isAllowedShareUri("file", "", "app.nectarlink"))
        assertFalse(OutgoingFiles.isAllowedShareUri("content", "sms", "app.nectarlink"))
        assertFalse(OutgoingFiles.isAllowedShareUri("content", "call_log", "app.nectarlink"))
        assertFalse(OutgoingFiles.isAllowedShareUri("content", "com.android.contacts", "app.nectarlink"))
        assertFalse(OutgoingFiles.isAllowedShareUri("content", "app.nectarlink.shizuku", "app.nectarlink"))
        assertTrue(OutgoingFiles.isAllowedShareUri("content", "media", "app.nectarlink"))

        data class FakeUri(val raw: String, val scheme: String?, val auth: String?)
        val good = FakeUri("content://media/external/images/1", "content", "media")
        val ungranted = FakeUri("content://media/external/images/2", "content", "media")
        val sms = FakeUri("content://sms/inbox", "content", "sms")
        val fileUri = FakeUri("file:///data/data/app.nectarlink/no_backup/adb/adb.key", "file", null)

        assertTrue(
            SendActivity.filterGrantedStreamUris(
                hasReadGrantFlag = false,
                streams = listOf(good),
                clipUris = setOf(good),
                schemeOf = { it.scheme },
                authorityOf = { it.auth },
                appPackage = "app.nectarlink",
            ).isEmpty(),
        )

        val filtered = SendActivity.filterGrantedStreamUris(
            hasReadGrantFlag = true,
            streams = listOf(good, ungranted, sms, fileUri),
            clipUris = setOf(good, sms, fileUri),
            schemeOf = { it.scheme },
            authorityOf = { it.auth },
            appPackage = "app.nectarlink",
        )
        assertEquals(listOf(good), filtered)

        assertTrue(SendActivity.isValidInternalToken("a1b2c3", "a1b2c3"))
        assertFalse(SendActivity.isValidInternalToken("a1b2c3", "a1b2c4"))
        assertFalse(SendActivity.isValidInternalToken("a1b2c3", null))
        assertFalse(SendActivity.isValidInternalToken("", ""))
    }

    @Test
    fun `isAppPrivatePath blocks app-private directories while allowing clipboard cache`() {
        val priv = listOf(File("/data/user/0/app.nectarlink"))
        val clipCache = listOf(File("/data/user/0/app.nectarlink/cache/clipboard"))

        assertTrue(OutgoingFiles.isAppPrivatePath("/data/user/0/app.nectarlink/no_backup/adb/adb.key", priv))
        assertTrue(OutgoingFiles.isAppPrivatePath("/data/user/0/app.nectarlink/cache/other.png", priv, clipCache))
        assertFalse(OutgoingFiles.isAppPrivatePath("/data/user/0/app.nectarlink/cache/clipboard/Image 1.png", priv, clipCache))
        assertFalse(OutgoingFiles.isAppPrivatePath("/storage/emulated/0/Download/report.pdf", priv, clipCache))
    }
}
