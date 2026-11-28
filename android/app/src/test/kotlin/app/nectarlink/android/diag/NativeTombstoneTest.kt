// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.diag

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayOutputStream

class NativeTombstoneTest {
    private fun varint(out: ByteArrayOutputStream, value: Long) {
        var v = value
        while (v and 0x7fL.inv() != 0L) {
            out.write(((v and 0x7f) or 0x80).toInt())
            v = v ushr 7
        }
        out.write(v.toInt())
    }

    private fun msg(vararg fields: Pair<Int, Any>): ByteArray {
        val out = ByteArrayOutputStream()
        for ((field, value) in fields) {
            when (value) {
                is Long -> {
                    varint(out, (field shl 3).toLong())
                    varint(out, value)
                }
                is ByteArray -> {
                    varint(out, ((field shl 3) or 2).toLong())
                    varint(out, value.size.toLong())
                    out.write(value)
                }
                is String -> {
                    val bytes = value.toByteArray()
                    varint(out, ((field shl 3) or 2).toLong())
                    varint(out, bytes.size.toLong())
                    out.write(bytes)
                }
            }
        }
        return out.toByteArray()
    }

    private fun frame(file: String, function: String = "", relPc: Long = 0) =
        msg(1 to relPc, 4 to function, 6 to file, 8 to "buildid")

    private fun thread(id: Long, vararg frames: ByteArray) =
        msg(1 to id, 2 to "worker", *frames.map { 4 to it }.toTypedArray(), 5 to ByteArray(32) { 7 })

    private val tombstone = msg(
        5 to 1234L,
        6 to 7L,
        10 to msg(1 to 6L, 2 to "SIGABRT"),
        14 to "memory allocation of 1234567 bytes failed",
        16 to msg(1 to 3L, 2 to thread(3, frame("/system/lib64/libc.so", "epoll_wait"))),
        16 to msg(
            1 to 7L,
            2 to thread(
                7,
                frame("/apex/com.android.runtime/lib64/bionic/libc.so", "abort"),
                frame("/apex/com.android.runtime/lib64/bionic/libc.so", "raise"),
                frame("/data/app/x/lib/arm64/libnectarlink_ffi.so", "_ZN15nectarlink_core12linux_unlock7respond17h0123456789abcdefE"),
                frame("/data/app/x/lib/arm64/libnectarlink_ffi.so", relPc = 0x1234),
                frame("/data/app/x/lib/arm64/libnectarlink_ffi.so", "uniffi_call"),
                frame("/system/lib64/libart.so", "art::Monitor::Wait(art::Thread*, long, int, bool, art::ThreadState)"),
            ),
        ),
    )

    @Test
    fun theCrashingThreadsFramesPastLibcsAbort() {
        assertEquals(
            listOf(
                "libnectarlink_ffi.so!nectarlink_core::linux_unlock::respond",
                "libnectarlink_ffi.so+0x1234",
                "libnectarlink_ffi.so!uniffi_call",
                "libart.so!art::Monitor::Wait",
            ),
            NativeTombstone.frames(tombstone),
        )
    }

    @Test
    fun neverTheAbortMessage() {
        // It can hold the values the code was working on.
        assertTrue(NativeTombstone.frames(tombstone).none { "1234567" in it })
    }

    @Test
    fun anythingElseGivesNoFrames() {
        assertEquals(emptyList<String>(), NativeTombstone.frames(ByteArray(0)))
        assertEquals(emptyList<String>(), NativeTombstone.frames("not a tombstone at all".toByteArray()))
        // Cut off in the middle: what can't be read is left out, never a crash.
        NativeTombstone.frames(tombstone.copyOf(tombstone.size / 2))
    }

    @Test
    fun rustNamesLoseTheirHash() {
        assertEquals("core::panicking::panic", NativeTombstone.demangle("_ZN4core9panicking5panic17h0123456789abcdefE"))
        assertEquals("_RNvCs1234_7mycrate3foo", NativeTombstone.demangle("_RNvCs1234_7mycrate3foo"))
        assertEquals("abort", NativeTombstone.demangle("abort"))
    }
}
