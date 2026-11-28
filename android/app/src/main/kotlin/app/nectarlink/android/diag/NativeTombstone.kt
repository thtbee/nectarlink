// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.diag

/**
 * Where a native crash happened, from the tombstone Android keeps for it
 * (`ApplicationExitInfo.getTraceInputStream()`, a protobuf on Android 12+;
 * AOSP `debuggerd/proto/tombstone.proto`). Only the crashing thread's top
 * frames are read: library and function names, never the abort message,
 * registers or memory, which can hold the values the code was working on.
 */
internal object NativeTombstone {
    private const val TOMBSTONE_TID = 6
    private const val TOMBSTONE_THREADS = 16
    private const val ENTRY_KEY = 1
    private const val ENTRY_VALUE = 2
    private const val THREAD_BACKTRACE = 4
    private const val FRAME_REL_PC = 1
    private const val FRAME_FUNCTION = 4
    private const val FRAME_FILE = 6

    /** libc's own frames at the top of an abort (abort, raise, the signal itself). */
    private val ABORT_LIBRARIES = setOf("libc.so", "libdl.so")

    /**
     * The crashing thread's frames, innermost first, as `library!function`
     * (or `library+0x<offset>` without a symbol), past libc's abort frames.
     * Empty when [bytes] isn't a tombstone this can read.
     */
    fun frames(bytes: ByteArray, max: Int = 4): List<String> = runCatching {
        var tid = -1L
        val threads = HashMap<Long, Pair<Int, Int>>()
        forEachField(bytes, 0, bytes.size) { field, value, start, end ->
            when (field) {
                TOMBSTONE_TID -> tid = value
                TOMBSTONE_THREADS -> {
                    var key = -1L
                    var thread: Pair<Int, Int>? = null
                    forEachField(bytes, start, end) { f, v, s, e ->
                        if (f == ENTRY_KEY) key = v
                        if (f == ENTRY_VALUE) thread = s to e
                    }
                    thread?.let { threads[key] = it }
                }
            }
        }
        val (start, end) = threads[tid] ?: return emptyList()
        val all = mutableListOf<Pair<String, String>>()
        forEachField(bytes, start, end) { field, _, s, e ->
            if (field == THREAD_BACKTRACE) all += frame(bytes, s, e)
        }
        all.dropWhile { (library, _) -> library in ABORT_LIBRARIES }
            .take(max)
            .map { (library, symbol) -> "$library$symbol" }
    }.getOrDefault(emptyList())

    private fun frame(bytes: ByteArray, start: Int, end: Int): Pair<String, String> {
        var function = ""
        var file = ""
        var relPc = 0L
        forEachField(bytes, start, end) { field, value, s, e ->
            when (field) {
                FRAME_FUNCTION -> function = String(bytes, s, e - s, Charsets.UTF_8)
                FRAME_FILE -> file = String(bytes, s, e - s, Charsets.UTF_8)
                FRAME_REL_PC -> relPc = value
            }
        }
        val library = file.substringAfterLast('/').ifEmpty { "?" }
        val symbol = if (function.isNotEmpty()) "!" + demangle(function).substringBefore('(').take(MAX_SYMBOL) else "+0x" + java.lang.Long.toHexString(relPc)
        return library to symbol
    }

    private const val MAX_SYMBOL = 90

    /**
     * Rust's legacy mangling (`_ZN4core9panicking5panic17h0123456789abcdefE`)
     * as `core::panicking::panic`, without the hash. Anything else as it is.
     */
    fun demangle(name: String): String {
        if (!name.startsWith("_ZN") || !name.endsWith("E")) return name
        val parts = mutableListOf<String>()
        var i = 3
        while (i < name.length - 1) {
            var j = i
            while (j < name.length && name[j].isDigit()) j++
            if (j == i) return name
            val length = name.substring(i, j).toInt()
            if (j + length > name.length) return name
            parts += name.substring(j, j + length)
            i = j + length
        }
        if (parts.lastOrNull()?.matches(Regex("h[0-9a-f]{16}")) == true) parts.removeAt(parts.lastIndex)
        return parts.joinToString("::") { it.replace("\$LT\$", "<").replace("\$GT\$", ">").replace("..", "::") }
    }

    /**
     * Calls [each] with each field's number in `bytes[start, end)`: the
     * value of a varint, or the bounds of a length-delimited field.
     */
    private inline fun forEachField(bytes: ByteArray, start: Int, end: Int, each: (field: Int, value: Long, start: Int, end: Int) -> Unit) {
        var at = start
        while (at < end) {
            val (key, afterKey) = varint(bytes, at)
            at = afterKey
            val field = (key ushr 3).toInt()
            when ((key and 7).toInt()) {
                0 -> {
                    val (value, next) = varint(bytes, at)
                    each(field, value, 0, 0)
                    at = next
                }
                1 -> at += 8
                2 -> {
                    val (length, next) = varint(bytes, at)
                    val stop = next + length.toInt()
                    // A field cut off at the end of what was read: keep what came before it.
                    if (length < 0 || stop > end) return
                    each(field, 0, next, stop)
                    at = stop
                }
                5 -> at += 4
                else -> throw IllegalArgumentException("unsupported wire type")
            }
        }
    }

    private fun varint(bytes: ByteArray, from: Int): Pair<Long, Int> {
        var result = 0L
        var shift = 0
        var at = from
        while (true) {
            require(at < bytes.size && shift < 64)
            val b = bytes[at++].toInt()
            result = result or ((b and 0x7f).toLong() shl shift)
            if (b and 0x80 == 0) return result to at
            shift += 7
        }
    }
}
