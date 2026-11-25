// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri

/**
 * Every link the app itself opens, in one place and nowhere else: never in a
 * string resource (translations come in as pull requests), a setting or
 * anything from the network, so nothing can repoint one. Opened by [open],
 * which checks each is https on a known host; `LinksTest` pins them.
 */
internal enum class Link(val url: String) {
    DISCORD("https://discord.gg/AA2Dp8PYqf"),
    X("https://x.com/thtbee_"),
    // Per build (src/github, src/play, src/fdroid: StoreLinks): empty in the Google Play build,
    // which links neither donations (Play's payments policy) nor the code and its APKs. Nothing
    // opens an empty link.
    DONATE(StoreLinks.DONATE),
    REPO(StoreLinks.REPO),
    RELEASES(StoreLinks.RELEASES),
    ;

    companion object {
        /** The hosts these may point at; anything else isn't opened. */
        val HOSTS = setOf("discord.gg", "x.com") + StoreLinks.HOSTS

        /** Whether [address] is https on one of [HOSTS]. */
        fun allowed(address: String): Boolean {
            val match = Regex("^https://([^/:?#]+)(?:[/?#]|$)").find(address) ?: return false
            return match.groupValues[1] in HOSTS
        }

        /** Opens [link] in the browser (or the app that handles it). False when nothing could. */
        fun open(context: Context, link: Link): Boolean {
            if (!allowed(link.url)) return false
            val intent = Intent(Intent.ACTION_VIEW, Uri.parse(link.url))
                .addCategory(Intent.CATEGORY_BROWSABLE)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            return try {
                context.startActivity(intent)
                true
            } catch (_: ActivityNotFoundException) {
                false
            }
        }
    }
}
