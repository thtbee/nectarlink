// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.storage

import app.nectarlink.core.StorageFailure
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.File
import java.nio.file.Files

class PhoneStorageTest {
    @Test
    fun validatesRelativeNamesAndPaths() {
        assertTrue(PhoneStorage.isValidName("notes.txt"))
        assertTrue(PhoneStorage.isValidName(".hidden"))
        assertFalse(PhoneStorage.isValidName(""))
        assertFalse(PhoneStorage.isValidName("."))
        assertFalse(PhoneStorage.isValidName(".."))
        assertFalse(PhoneStorage.isValidName("a/b"))
        assertFalse(PhoneStorage.isValidName("a\\b"))
        assertFalse(PhoneStorage.isValidName("bad\u0000name"))

        assertTrue(PhoneStorage.isValidDirPath(""))
        assertTrue(PhoneStorage.isValidDirPath("DCIM/Camera"))
        assertFalse(PhoneStorage.isValidPath(""))
        assertTrue(PhoneStorage.isValidPath("DCIM/Camera/IMG_001.jpg"))
        assertFalse(PhoneStorage.isValidPath("/etc/passwd"))
        assertFalse(PhoneStorage.isValidPath("DCIM/../secret"))
        assertFalse(PhoneStorage.isValidPath("DCIM/Camera/"))
        assertFalse(PhoneStorage.isValidPath("DCIM//Camera"))
    }

    @Test
    fun resolvesInsideRootAndBlocksPrivateAndTraversalPaths() {
        val temp = Files.createTempDirectory("nectarlink-storage-test").toFile()
        try {
            val root = File(temp, "shared").apply { mkdirs() }
            val outside = File(temp, "outside").apply { mkdirs() }
            val outsideSecret = File(outside, "secret.txt").apply { writeText("secret") }
            val androidData = File(root, "Android/data/app.nectarlink").apply { mkdirs() }
            File(androidData, "private.db").writeText("private")
            val docs = File(root, "Documents").apply { mkdirs() }
            val note = File(docs, "note.txt").apply { writeText("hello") }

            val blocked = listOf(
                File("/data"),
                File(root, "Android/data"),
                File(root, "Android/obb"),
            )

            val resolvedRoot = PhoneStorage.resolveExistingInRoot(root, blocked, "", allowRoot = true)
            assertEquals(root.canonicalFile, resolvedRoot)

            val resolvedNote = PhoneStorage.resolveExistingInRoot(
                root,
                blocked,
                "Documents/note.txt",
                allowRoot = false,
            )
            assertEquals(note.canonicalFile, resolvedNote)

            // Reject traversal & absolute paths
            for (bad in listOf("../outside/secret.txt", "/data/system", "Documents/../../outside/secret.txt")) {
                try {
                    PhoneStorage.resolveExistingInRoot(root, blocked, bad, allowRoot = false)
                    fail("expected Invalid for $bad")
                } catch (_: StorageFailure.Invalid) {
                }
            }

            // Reject blocked Android/data directory
            try {
                PhoneStorage.resolveExistingInRoot(
                    root,
                    blocked,
                    "Android/data/app.nectarlink/private.db",
                    allowRoot = false,
                )
                fail("expected Denied for Android/data")
            } catch (_: StorageFailure.Denied) {
            }

            try {
                PhoneStorage.resolveTargetInRoot(
                    root,
                    blocked,
                    "Android/data/app.nectarlink/new.db",
                )
                fail("expected Denied for target in Android/data")
            } catch (_: StorageFailure.Denied) {
            }

            // If the OS allows creating a symlink, verify symlink escape is rejected
            val linkFile = File(docs, "escape.txt")
            val symlinkCreated = runCatching {
                Files.createSymbolicLink(linkFile.toPath(), outsideSecret.toPath())
                true
            }.getOrDefault(false)
            if (symlinkCreated) {
                try {
                    PhoneStorage.resolveExistingInRoot(
                        root,
                        blocked,
                        "Documents/escape.txt",
                        allowRoot = false,
                    )
                    fail("expected Denied for symlink escaping root")
                } catch (_: StorageFailure.Denied) {
                }
            }
        } finally {
            temp.deleteRecursively()
        }
    }
}
