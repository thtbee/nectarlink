// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.banking

/**
 * Android's list of enabled accessibility services, as stored in Settings: names
 * joined by `:`. One service can be written two ways, `pkg/pkg.Service` or the
 * short `pkg/.Service`, and both mean the same to Android, so matching them as
 * plain text misses it. Kept apart from Android so it can be tested.
 */
internal object AccessibilityServices {
    private const val SEPARATOR = ':'

    fun parse(setting: String?): List<String> =
        setting?.split(SEPARATOR)?.filter { it.isNotBlank() }.orEmpty()

    fun contains(setting: String?, service: String): Boolean =
        parse(setting).any { sameService(it, service) }

    /** The list with [service] taken out, however it was written. */
    fun without(setting: String?, service: String): List<String> =
        parse(setting).filterNot { sameService(it, service) }

    fun join(services: List<String>): String = services.joinToString(SEPARATOR.toString())

    private fun sameService(a: String, b: String): Boolean = longForm(a).equals(longForm(b), ignoreCase = true)

    private fun longForm(name: String): String {
        val slash = name.indexOf('/')
        if (slash <= 0 || name.getOrNull(slash + 1) != '.') return name
        val pkg = name.substring(0, slash)
        return "$pkg/$pkg${name.substring(slash + 1)}"
    }
}
