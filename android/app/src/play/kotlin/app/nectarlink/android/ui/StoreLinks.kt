// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

/**
 * The Google Play build links neither donations (Play's payments policy) nor
 * the code with its APKs: these stay empty, and [Link.open] opens nothing empty.
 */
internal object StoreLinks {
    const val DONATE = ""
    const val REPO = ""
    /** The hosts these point at. */
    val HOSTS: Set<String> = emptySet<String>()
    const val RELEASES = ""
}
