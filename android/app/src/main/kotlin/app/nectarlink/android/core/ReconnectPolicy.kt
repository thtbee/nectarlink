// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.core

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.NetworkCapabilities
import java.net.Inet4Address
import java.security.MessageDigest

/**
 * When the phone may look for its PCs by itself (#397), past having a
 * network at all:
 *
 * - **Known networks only**: the phone learns the networks where it reached a
 *   PC directly and, with this on, only reconnects in the background on those.
 *   A network is told apart by its router's address and its address range,
 *   which needs no location permission (a Wi-Fi network's name would).
 * - **Stop trying after** a while with no PC reachable: it waits until the
 *   network changes or NectarLink is opened.
 *
 * A PC can still reach the phone either way; this only stops the phone's own
 * dialing, which is what costs battery.
 */
internal class ReconnectPolicy(context: Context) {
    private val app = context.applicationContext
    private val prefs = app.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
    private val connectivity = app.getSystemService(ConnectivityManager::class.java)

    /** When no PC has been reachable since, while trying (ms since boot clock). */
    @Volatile private var failingSince: Long? = null

    /** Stopped trying after [giveUpMinutes], until the network changes or the app opens. */
    @Volatile var gaveUp: Boolean = false
        private set

    var knownOnly: Boolean
        get() = prefs.getBoolean(KNOWN_ONLY, false)
        set(value) = prefs.edit().putBoolean(KNOWN_ONLY, value).apply()

    /** 0: keep trying. */
    var giveUpMinutes: Int
        get() = prefs.getInt(GIVE_UP_MINUTES, 0)
        set(value) = prefs.edit().putInt(GIVE_UP_MINUTES, value.coerceAtLeast(0)).apply()

    private val known: Set<String>
        get() = prefs.getStringSet(KNOWN_NETWORKS, emptySet()).orEmpty()

    /**
     * The network the phone is on, as an ID, or null when it can't be told
     * apart or doesn't need to be: a VPN or USB link always counts as known.
     */
    fun currentNetwork(): String? = runCatching {
        val manager = connectivity ?: return null
        val network = manager.activeNetwork ?: return null
        val caps = manager.getNetworkCapabilities(network) ?: return null
        if (caps.hasTransport(NetworkCapabilities.TRANSPORT_VPN)) return null
        if (!caps.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) &&
            !caps.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET)
        ) {
            return null
        }
        fingerprint(manager.getLinkProperties(network) ?: return null)
    }.getOrNull()

    /** Whether the phone may dial PCs on the network it's on now. */
    fun allows(network: String?): Boolean {
        if (gaveUp) return false
        if (!knownOnly || network == null) return true
        val known = known
        // Nothing learned yet: no network can be told from another.
        return known.isEmpty() || network in known
    }

    /** Whether the phone holds back only because it's on a network it doesn't know. */
    fun unknownNetwork(network: String?): Boolean =
        !gaveUp && knownOnly && network != null && known.let { it.isNotEmpty() && network !in it }

    /** A PC was reached directly on [network]: it's known from now on. */
    @Synchronized
    fun learn(network: String?) {
        if (network == null) return
        val known = known
        if (network in known) return
        // Newest last; the oldest go past the limit (a set keeps no order,
        // so the order is kept in a second list).
        val order = prefs.getString(KNOWN_ORDER, "").orEmpty().split(',').filter { it.isNotEmpty() } + network
        val kept = order.takeLast(MAX_KNOWN)
        prefs.edit()
            .putStringSet(KNOWN_NETWORKS, kept.toSet())
            .putString(KNOWN_ORDER, kept.joinToString(","))
            .apply()
    }

    /** Forgets the networks learned so far. */
    fun forgetNetworks() {
        prefs.edit().remove(KNOWN_NETWORKS).remove(KNOWN_ORDER).apply()
    }

    val knownCount: Int get() = known.size

    /**
     * Notes whether a PC is reachable now, while the phone is trying to
     * reach its PCs. Returns true when it just gave up.
     */
    @Synchronized
    fun note(anyOnline: Boolean, trying: Boolean, now: Long): Boolean {
        if (anyOnline) {
            // A PC is back, even one that reached the phone: when it goes, look again.
            gaveUp = false
        }
        if (anyOnline || !trying) {
            failingSince = null
            return false
        }
        val since = failingSince ?: now.also { failingSince = it }
        if (!gaveUp && overdue(since, now, giveUpMinutes)) {
            gaveUp = true
            return true
        }
        return false
    }

    /** How long until it gives up, or null when it isn't counting. */
    @Synchronized
    fun remainingMs(now: Long): Long? {
        val since = failingSince ?: return null
        val minutes = giveUpMinutes
        if (minutes == 0 || gaveUp) return null
        return (since + minutes * 60_000L - now).coerceAtLeast(0L)
    }

    /** The network changed or NectarLink was opened: try again. */
    @Synchronized
    fun reset() {
        gaveUp = false
        failingSince = null
    }

    companion object {
        private const val PREFS = "reconnect"
        private const val KNOWN_ONLY = "known_only"
        private const val GIVE_UP_MINUTES = "give_up_minutes"
        private const val KNOWN_NETWORKS = "known_networks"
        private const val KNOWN_ORDER = "known_order"
        private const val MAX_KNOWN = 30

        /** The choices offered for [giveUpMinutes]. */
        val GIVE_UP_CHOICES = listOf(0, 15, 60, 240)

        /**
         * A network's ID: its IPv4 address range and router, and its DNS
         * domain. Two homes can both be 192.168.1.0/24 behind 192.168.1.1, so
         * this tells most networks apart, not all of them; it never needs the
         * Wi-Fi network's name, which Android only gives with location access.
         */
        internal fun fingerprint(link: LinkProperties): String? {
            val range = link.linkAddresses.firstNotNullOfOrNull { address ->
                val ip = address.address as? Inet4Address ?: return@firstNotNullOfOrNull null
                val bits = address.prefixLength
                val raw = ip.address
                val masked = ByteArray(4) { i ->
                    val keep = (bits - i * 8).coerceIn(0, 8)
                    (raw[i].toInt() and (0xFF shl (8 - keep))).toByte()
                }
                masked.joinToString(".") { (it.toInt() and 0xFF).toString() } + "/" + bits
            } ?: return null
            val router = link.routes
                .firstOrNull { it.isDefaultRoute && it.gateway is Inet4Address }
                ?.gateway?.hostAddress.orEmpty()
            return idOf(range, router, link.domains.orEmpty())
        }

        /** Whether trying since [since] has gone on past [minutes] (0: never). */
        internal fun overdue(since: Long, now: Long, minutes: Int): Boolean =
            minutes > 0 && now - since >= minutes * 60_000L

        internal fun idOf(range: String, router: String, domains: String): String {
            val digest = MessageDigest.getInstance("SHA-256").digest("$range|$router|$domains".toByteArray())
            return digest.take(8).joinToString("") { "%02x".format(it) }
        }
    }
}
