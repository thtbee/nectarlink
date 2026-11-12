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

    @Test
    fun namesRemovableVolumesWithoutClashing() {
        val sd = File("/storage/1A2B-3C4D")
        val usb = File("/storage/5E6F-7081")
        val other = File("/storage/9999-0000")
        val names = PhoneStorage.volumeNames(
            listOf("SD card" to sd, "SD card" to other, "SanDisk/USB" to usb, "  " to File("/storage/X")),
        ) { it == "SanDiskUSB" }
        assertEquals(listOf("SD card", "SD card (2)", "SanDiskUSB (2)", "SD card (3)"), names.map { it.name })
        assertEquals(listOf(sd, other, usb, File("/storage/X")), names.map { it.dir })
    }

    @Test
    fun locatesPathsOnRemovableVolumesOrInternalStorage() {
        val internal = File("/storage/emulated/0")
        val sd = File("/storage/1A2B-3C4D")
        val volumes = listOf(Volume("SD card", sd))
        assertEquals(internal to "", PhoneStorage.locate(volumes, "", internal))
        assertEquals(internal to "DCIM/Camera", PhoneStorage.locate(volumes, "DCIM/Camera", internal))
        assertEquals(sd to "", PhoneStorage.locate(volumes, "SD card", internal))
        assertEquals(sd to "DCIM/IMG_1.jpg", PhoneStorage.locate(volumes, "SD card/DCIM/IMG_1.jpg", internal))
        // Only a whole first segment names a volume.
        assertEquals(internal to "SD cards/x", PhoneStorage.locate(volumes, "SD cards/x", internal))
    }

    @Test
    fun blocksDeletingOrMovingAncestorsOfBlockedDirectoriesAndSymlinksToBlockedTargets() {
        val temp = Files.createTempDirectory("nectarlink-storage-ancestor-test").toFile()
        try {
            val root = File(temp, "shared").apply { mkdirs() }
            val canonRoot = root.canonicalFile
            val androidDir = File(root, "Android").apply { mkdirs() }
            val androidData = File(androidDir, "data").apply { mkdirs() }
            val appPrivate = File(androidData, "app.nectarlink/secret.txt").apply {
                parentFile?.mkdirs()
                writeText("keep")
            }
            val blocked = listOf(File(root, "Android/data"), File(root, "Android/obb"))

            // `Android` contains `Android/data`, so `containsBlockedDir` is true and `deleteTreeSafely` refuses.
            assertTrue(PhoneStorage.containsBlockedDir(androidDir.canonicalFile, blocked))
            assertFalse(PhoneStorage.deleteTreeSafely(androidDir, canonRoot, blocked))
            assertTrue(appPrivate.exists())

            // If symlink creation is permitted, a symlink to `Android` cannot be used to resolve `link/data/sub`.
            val linkToAndroid = File(root, "link_android")
            val created = runCatching {
                Files.createSymbolicLink(linkToAndroid.toPath(), androidDir.toPath())
                true
            }.getOrDefault(false)
            if (created) {
                try {
                    PhoneStorage.resolveTargetInRoot(root, blocked, "link_android/data/new_folder/file.txt")
                    fail("expected Denied when traversing symlink into non-existent child of blocked dir")
                } catch (_: StorageFailure.Denied) {
                }

                // Deleting the symlink deletes the link itself, not the target directory.
                val resolvedLink = PhoneStorage.resolveEntryInRoot(root, blocked, "link_android")
                assertTrue(PhoneStorage.isSymlink(resolvedLink))
                assertTrue(PhoneStorage.deleteTreeSafely(resolvedLink, canonRoot, blocked))
                assertFalse(PhoneStorage.isSymlink(linkToAndroid))
                assertTrue(androidDir.exists())
                assertTrue(appPrivate.exists())
            }
        } finally {
            temp.deleteRecursively()
        }
    }

    @Test
    fun deletingWhatIsAlreadyGoneIsNotFoundAndARefusalStillFails() {
        val temp = Files.createTempDirectory("nectarlink-storage-delete-test").toFile()
        try {
            val root = File(temp, "shared").apply { mkdirs() }
            val canonRoot = root.canonicalFile
            val blocked = listOf(File(root, "Android/data"))
            val file = File(root, "app.xapk").apply { writeText("x") }

            PhoneStorage.deleteEntry(file, canonRoot, blocked)
            assertFalse(file.exists())
            // A second delete of the same file (the PC asks again) is NotFound, which the PC takes as done.
            try {
                PhoneStorage.deleteEntry(file, canonRoot, blocked)
                fail("expected NotFound for a file that's already gone")
            } catch (_: StorageFailure.NotFound) {
            }

            val secret = File(root, "Android/data/app/secret.txt").apply {
                parentFile?.mkdirs()
                writeText("keep")
            }
            try {
                PhoneStorage.deleteEntry(File(root, "Android"), canonRoot, blocked)
                fail("expected a refusal to stay a failure")
            } catch (_: StorageFailure.Failed) {
            }
            assertTrue(secret.exists())
        } finally {
            temp.deleteRecursively()
        }
    }

    @Test
    fun filterPersistedSafEntriesDropsAllWhenPersistedSetIsEmpty() {
        val raw = setOf("Documents|content://tree/docs", "Music|content://tree/music")
        assertTrue(
            PhoneStorage.filterPersistedSafEntries(
                rawEntries = raw,
                persistedUris = emptySet(),
                parseUri = { it },
            ).isEmpty(),
        )
        val partial = PhoneStorage.filterPersistedSafEntries(
            rawEntries = raw,
            persistedUris = setOf("content://tree/music"),
            parseUri = { it },
        )
        assertEquals(listOf("Music" to "content://tree/music"), partial)
    }
}
