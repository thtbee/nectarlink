// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.links

import android.app.KeyguardManager
import android.content.Context
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import app.nectarlink.android.banking.BankingPause
import app.nectarlink.android.clipboard.PhoneClipboard
import app.nectarlink.android.media.PhoneMedia
import app.nectarlink.android.mirror.InputService
import app.nectarlink.core.HandoffKind
import app.nectarlink.core.PulledLink
import app.nectarlink.core.extractHandoffLink
import app.nectarlink.core.withVideoTimestamp

/**
 * Resolves the phone's currently active shareable Handoff URL (`link.pull`),
 * respecting privacy: skips Incognito / Private browsing windows, banking and
 * password-manager apps, and locked screens, and strictly validates `http://` /
 * `https://` URLs.
 */
object ActiveHandoffLink {
    private const val MAX_LINK_BYTES = 4096

    internal data class BrowserSnapshot(
        val packageName: String,
        val urlText: String?,
        val toolbarLabels: List<String>,
        val resourceIds: List<String>,
    )

    private val BROWSER_PACKAGES = setOf(
        "com.android.chrome",
        "com.chrome.beta",
        "com.chrome.dev",
        "com.chrome.canary",
        "com.microsoft.emmx",
        "com.microsoft.emmx.beta",
        "com.microsoft.emmx.dev",
        "com.microsoft.emmx.canary",
        "com.brave.browser",
        "com.brave.browser_beta",
        "com.brave.browser_nightly",
        "org.mozilla.firefox",
        "org.mozilla.firefox_beta",
        "org.mozilla.fenix",
        "io.github.forkmaintainers.iceraven",
        "com.opera.browser",
        "com.opera.mini.native",
        "com.opera.gx",
        "com.vivaldi.browser",
        "com.sec.android.app.sbrowser",
        "com.sec.android.app.sbrowser.beta",
        "com.duckduckgo.mobile.android",
        "com.kiwibrowser.browser",
        "company.thebrowser.arc",
    )

    private val ALWAYS_PRIVATE_PACKAGES = setOf(
        "org.mozilla.focus",
        "org.torproject.torbrowser",
    )

    private val YOUTUBE_PACKAGES = setOf(
        "com.google.android.youtube",
        "app.revanced.android.youtube",
        "app.rvx.android.youtube",
        "com.vanced.android.youtube",
    )

    private val SENSITIVE_PACKAGE_TOKENS = listOf(
        "1password",
        "onepassword",
        "bitwarden",
        "keepass",
        "dashlane",
        "lastpass",
        "proton.pass",
        "authenticator",
        "banking",
        "bank",
        "wallet",
        "paypal",
        "venmo",
        "cashapp",
        "revolut",
        "wise",
        "phonepe",
        "paytm",
        "paisa", // Google Pay (com.google.android.apps.nbu.paisa.user)
    )

    private val PRIVATE_RESOURCE_TOKENS = listOf(
        "incognito",
        "inprivate",
        "private_browsing",
        "private_tab",
        "secret_mode",
    )

    private val PRIVATE_LABEL_MARKERS = listOf(
        "you've gone incognito",
        "close all incognito",
        "incognito tab",
        "(incognito)",
        "inprivate",
        "private browsing",
        "private tab",
        "secret mode",
        "(private)",
    )

    private val YOUTUBE_VIDEO_ID_REGEX = Regex("^[A-Za-z0-9_-]{11}$")

    fun resolve(context: Context, media: PhoneMedia): PulledLink? {
        if (BankingPause.isPaused(context)) return null
        val keyguard = context.getSystemService(KeyguardManager::class.java)
        val locked = keyguard?.isKeyguardLocked == true
        val bankingApps = BankingPause.apps(context)

        if (!locked && InputService.running) {
            val snapshot = InputService.activeBrowserSnapshot()
            if (snapshot != null) {
                if (isSensitiveOrBankingPackage(snapshot.packageName, bankingApps)) {
                    return null
                }
                if (isBrowserPackage(snapshot.packageName)) {
                    if (isPrivateBrowserUi(snapshot.packageName, snapshot.toolbarLabels, snapshot.resourceIds)) {
                        return null
                    }
                    val url = normalizeHttpUrl(snapshot.urlText)
                    if (url != null) {
                        val activeMedia = media.activeMediaLink()
                        val matchingMedia = activeMedia?.takeIf {
                            it.packageName.equals(snapshot.packageName, ignoreCase = true) ||
                                isYoutubePackage(it.packageName)
                        }
                        val finalUrl = attachVideoTimestampIfNeeded(url, matchingMedia?.positionSecs)
                        val title = matchingMedia?.title?.takeIf { it.isNotBlank() }
                        return PulledLink(url = finalUrl, title = title)
                    }
                }
            }
        }

        val activeMedia = media.activeMediaLink()
        if (activeMedia != null && !isSensitiveOrBankingPackage(activeMedia.packageName, bankingApps)) {
            val mediaUrl = mediaLinkToUrl(activeMedia.packageName, activeMedia.mediaUri, activeMedia.mediaId)
            if (mediaUrl != null) {
                val finalUrl = attachVideoTimestampIfNeeded(mediaUrl, activeMedia.positionSecs)
                return PulledLink(
                    url = finalUrl,
                    title = activeMedia.title?.takeIf { it.isNotBlank() },
                )
            }
        }

        if (!locked && isAppInForeground()) {
            val clip = PhoneClipboard.read(context)
            if (clip is PhoneClipboard.Read.Text && !PhoneClipboard.isRecentEcho(clip.text)) {
                val raw = clip.text.trim()
                val extracted = extractHandoffLink(raw)?.url ?: raw
                val url = normalizeHttpUrl(extracted)
                if (url != null) {
                    val finalUrl = attachVideoTimestampIfNeeded(url, activeMedia?.positionSecs)
                    return PulledLink(url = finalUrl, title = null)
                }
            }
        }

        return null
    }

    internal fun isBrowserPackage(pkg: String?): Boolean {
        val clean = pkg?.trim()?.lowercase() ?: return false
        return clean in BROWSER_PACKAGES
    }

    internal fun isYoutubePackage(pkg: String?): Boolean {
        val clean = pkg?.trim()?.lowercase() ?: return false
        return clean in YOUTUBE_PACKAGES
    }

    internal fun isSensitiveOrBankingPackage(pkg: String?, bankingApps: Set<String>): Boolean {
        val clean = pkg?.trim()?.lowercase() ?: return false
        if (clean in ALWAYS_PRIVATE_PACKAGES) return true
        if (bankingApps.any { it.equals(clean, ignoreCase = true) }) return true
        return SENSITIVE_PACKAGE_TOKENS.any { token ->
            if (token.contains('.')) {
                clean.contains(token)
            } else {
                clean.split('.').any { seg -> seg == token || seg.contains(token) }
            }
        }
    }

    internal fun isPrivateBrowserUi(
        pkg: String?,
        toolbarLabels: List<String>,
        resourceIds: List<String>,
    ): Boolean {
        val cleanPkg = pkg?.trim()?.lowercase()
        if (cleanPkg in ALWAYS_PRIVATE_PACKAGES) return true
        if (resourceIds.any { id ->
                val lower = id.lowercase()
                PRIVATE_RESOURCE_TOKENS.any { token -> lower.contains(token) }
            }
        ) {
            return true
        }
        return toolbarLabels.any { label ->
            val lower = label.trim().lowercase()
            PRIVATE_LABEL_MARKERS.any { marker -> lower.contains(marker) }
        }
    }

    internal fun normalizeHttpUrl(raw: String?): String? {
        val trimmed = raw?.trim() ?: return null
        if (trimmed.isEmpty() || trimmed.length > MAX_LINK_BYTES) return null
        if (trimmed.any { it.isISOControl() || it.isWhitespace() }) return null
        val lower = trimmed.lowercase()
        val candidate = when {
            lower.startsWith("https://") || lower.startsWith("http://") -> trimmed
            lower.contains("://") || lower.startsWith("about:") || lower.startsWith("javascript:") ||
                lower.startsWith("data:") || lower.startsWith("intent:") || lower.startsWith("geo:") -> return null
            else -> {
                val host = lower.substringBefore('/').substringBefore('?').substringBefore('#').substringBefore(':')
                if (host.contains('.') || host == "localhost") "https://$trimmed" else return null
            }
        }
        val afterScheme = candidate.substringAfter("://", "")
        if (afterScheme.isEmpty() || afterScheme.startsWith('/')) return null
        val authority = afterScheme.substringBefore('/').substringBefore('?').substringBefore('#')
        val host = authority.substringAfterLast('@').substringBefore(':')
        if (host.isEmpty()) return null
        return candidate
    }

    internal fun mediaLinkToUrl(packageName: String, mediaUri: String?, mediaId: String?): String? {
        normalizeHttpUrl(mediaUri)?.let { return it }
        if (isYoutubePackage(packageName)) {
            val id = mediaId?.trim().orEmpty()
            if (YOUTUBE_VIDEO_ID_REGEX.matches(id)) {
                return "https://www.youtube.com/watch?v=$id"
            }
        }
        return null
    }

    internal fun attachVideoTimestampIfNeeded(url: String, positionSecs: UInt?): String {
        val secs = positionSecs ?: return url
        if (secs == 0u) return url
        val handoff = runCatching { extractHandoffLink(url) }.getOrNull() ?: return url
        if (handoff.kind == HandoffKind.VIDEO_LINK && handoff.timestampSecs == null) {
            return runCatching { withVideoTimestamp(handoff.url, secs) }.getOrDefault(handoff.url)
        }
        return url
    }

    private fun isAppInForeground(): Boolean =
        runCatching {
            ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)
        }.getOrDefault(false)
}
