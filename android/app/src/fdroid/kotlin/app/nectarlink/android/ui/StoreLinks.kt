// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

/** The links the F-Droid build keeps alongside the GitHub build ([Link]). */
internal object StoreLinks {
    const val DONATE = "https://buymeacoffee.com/thtbee_"
    const val REPO = "https://github.com/thtbee/nectarlink"
    /** The hosts these point at. */
    val HOSTS: Set<String> = setOf("buymeacoffee.com", "github.com")
    const val RELEASES = "https://github.com/thtbee/nectarlink/releases"
}
