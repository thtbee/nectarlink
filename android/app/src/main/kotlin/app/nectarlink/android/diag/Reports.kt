// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.diag

import android.Manifest
import android.app.usage.UsageStatsManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.os.PowerManager
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.core.Device
import app.nectarlink.core.Link
import app.nectarlink.core.CountInstall
import app.nectarlink.core.CrashCard
import app.nectarlink.core.OutgoingReport
import app.nectarlink.core.ReportDevice
import app.nectarlink.core.ReportOutcome
import app.nectarlink.core.ReportPair
import app.nectarlink.core.crashCardFixedIn
import app.nectarlink.core.dailyCountDue
import app.nectarlink.core.knownCrashesUrl
import app.nectarlink.core.dailyCountSent
import app.nectarlink.core.reportAnswer
import app.nectarlink.core.reportFollowed
import app.nectarlink.core.reportIssueAnswer
import app.nectarlink.core.reportOutgoing
import app.nectarlink.core.reportPending
import app.nectarlink.core.reportPrepare
import app.nectarlink.core.reportQueued
import app.nectarlink.core.reportQueuedAnswer
import app.nectarlink.core.reportService
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.net.HttpURLConnection
import java.net.URL

/**
 * Report a problem on the phone: what the phone says about itself, its
 * checks, and sending a report to the report service
 * (`nectarlink_core::diag::send`). The report itself is built in the core.
 */
object Reports {
    private const val MAX_ANSWER = 16 * 1024

    /** This phone, with the checks that explain most connection trouble. */
    fun phone(context: Context): ReportDevice {
        val power = context.getSystemService(PowerManager::class.java)
        val checks = buildList {
            add(pair("notification_access", onOff(NotificationManagerCompat.getEnabledListenerPackages(context).contains(context.packageName))))
            add(pair("battery_unrestricted", onOff(power?.isIgnoringBatteryOptimizations(context.packageName) == true)))
            if (Build.VERSION.SDK_INT >= 33) {
                add(pair("post_notifications", granted(context, Manifest.permission.POST_NOTIFICATIONS)))
                add(pair("nearby_wifi", granted(context, Manifest.permission.NEARBY_WIFI_DEVICES)))
            }
            add(pair("background_kills", Crashes.backgroundKills(context).toString()))
        }
        val extra = buildList {
            add(pair("battery saver", onOff(power?.isPowerSaveMode == true)))
            if (Build.VERSION.SDK_INT >= 28) {
                val bucket = context.getSystemService(UsageStatsManager::class.java)?.appStandbyBucket
                add(pair("standby bucket", bucketName(bucket)))
            }
        }
        val version = BuildConfig.VERSION_NAME
        return ReportDevice(
            role = "Phone",
            appVersion = version,
            channel = if ("-nightly." in version) "nightly" else "stable",
            install = if (BuildConfig.DEBUG) "debug apk" else "apk",
            os = "Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT})",
            arch = Build.SUPPORTED_ABIS.firstOrNull().orEmpty(),
            hardware = listOf("${Build.MANUFACTURER} ${Build.MODEL}".trim()),
            extra = extra,
            checks = checks,
        )
    }

    /** A paired PC as far as the phone knows it: what it shared when pairing. */
    fun other(device: Device): ReportDevice = ReportDevice(
        id = device.id,
        role = if (device.isAndroid) "Phone" else "PC",
        appVersion = "",
        channel = "",
        install = "",
        os = device.os,
        arch = "",
        hardware = listOfNotNull(device.model),
        extra = listOf(pair("link", linkName(device.link))),
        checks = emptyList(),
    )

    /**
     * Sends a crash report for [card]: the crash, versions, checks and feature health, never
     * the log. True once it's sent or kept to send later. Blocking.
     */
    fun sendCrash(context: Context, card: CrashCard, devices: List<Device>): Boolean {
        reportPrepare(card.area, "", "", false, 15, phone(context), devices.map(::other))
        val outgoing = reportOutgoing(card.sig) ?: return false
        return when (deliver(outgoing, follow = false)) {
            is ReportOutcome.Filed, ReportOutcome.Queued, ReportOutcome.Later -> true
            ReportOutcome.Limited, ReportOutcome.Failed -> false
        }
    }

    /** The version that fixes the crash card's crash, if a newer one does. Blocking. */
    fun knownFix(): String? {
        val (status, body) = request(knownCrashesUrl(), null)
        return if (status.toInt() == 200) crashCardFixedIn(body) else null
    }

    /** Posts [outgoing] and says what became of it. Blocking: call it off the main thread. */
    fun deliver(outgoing: OutgoingReport, follow: Boolean = true): ReportOutcome {
        val (status, body) = try {
            post("${reportService()}/v1/report", outgoing)
        } catch (_: IOException) {
            0.toUShort() to ByteArray(0)
        }
        return reportAnswer(outgoing, status, body, follow)
    }

    /**
     * At an update check: sends the report that couldn't go before, and asks
     * which issue queued ones became. Blocking.
     */
    fun followUp(context: Context) {
        dailyCount(context)
        reportPending()?.let { deliver(it) }
        // Replies and closed reports: one GET per followed report, numbers only.
        for (followed in reportFollowed()) {
            runCatching {
                val (status, body) = request(followed.url, null)
                reportIssueAnswer(followed.rid, status, body)?.let { ReportNotices.show(context, it) }
            }
        }
        for (rid in reportQueued()) {
            runCatching {
                val (status, body) = request("${reportService()}/v1/report/$rid", null)
                reportQueuedAnswer(rid, status, body)
            }
        }
    }

    /**
     * Sends today's count (`nectarlink_core::diag::usage`) once a day: version, Android version
     * and phone model, and how features did yesterday. No ID. Blocking.
     */
    private fun dailyCount(context: Context) {
        // Debug builds and test runs aren't installs.
        if (BuildConfig.DEBUG) return
        val version = BuildConfig.VERSION_NAME
        val install = CountInstall(
            app = version,
            channel = if ("-nightly." in version) "nightly" else "stable",
            platform = "android",
            os = "Android ${Build.VERSION.RELEASE}",
            hardware = "${Build.MANUFACTURER} ${Build.MODEL}".trim(),
        )
        val count = dailyCountDue(install) ?: return
        val (status, _) = try {
            request("${reportService()}/v1/ping", count.body, count.proof)
        } catch (_: IOException) {
            return
        }
        // Counted, or refused for good: either way, not again today.
        if (status.toInt() < 500 && status.toInt() != 429) dailyCountSent()
    }

    private fun post(url: String, outgoing: OutgoingReport): Pair<UShort, ByteArray> =
        request(url, outgoing.body, outgoing.proof)

    private fun request(url: String, outgoing: OutgoingReport?): Pair<UShort, ByteArray> =
        request(url, outgoing?.body, outgoing?.proof)

    private fun request(url: String, body: String?, proof: String?): Pair<UShort, ByteArray> {
        if (!url.startsWith("https://")) throw IOException("only https")
        val connection = (URL(url).openConnection() as HttpURLConnection).apply {
            connectTimeout = 15_000
            readTimeout = 30_000
            instanceFollowRedirects = false
            setRequestProperty("User-Agent", "Nectarlink/${BuildConfig.VERSION_NAME} (Android)")
            setRequestProperty("Accept", "application/json")
        }
        try {
            if (body != null) {
                val bytes = body.toByteArray(Charsets.UTF_8)
                connection.requestMethod = "POST"
                connection.doOutput = true
                connection.setFixedLengthStreamingMode(bytes.size)
                connection.setRequestProperty("Content-Type", "application/json")
                connection.setRequestProperty("X-Proof", proof.orEmpty())
                connection.outputStream.use { it.write(bytes) }
            }
            val status = connection.responseCode
            val stream = if (status >= 400) connection.errorStream else connection.inputStream
            val body = stream?.use { input ->
                val out = ByteArrayOutputStream()
                val buffer = ByteArray(4096)
                while (out.size() <= MAX_ANSWER) {
                    val read = input.read(buffer)
                    if (read < 0) break
                    out.write(buffer, 0, read)
                }
                out.toByteArray()
            } ?: ByteArray(0)
            return status.toUShort() to body
        } finally {
            connection.disconnect()
        }
    }

    private fun pair(name: String, value: String) = ReportPair(name, value)

    private fun onOff(on: Boolean) = if (on) "on" else "off"

    private fun granted(context: Context, permission: String) =
        if (ContextCompat.checkSelfPermission(context, permission) == PackageManager.PERMISSION_GRANTED) "granted" else "denied"

    internal fun bucketName(bucket: Int?): String = when (bucket) {
        UsageStatsManager.STANDBY_BUCKET_ACTIVE -> "active"
        UsageStatsManager.STANDBY_BUCKET_WORKING_SET -> "working_set"
        UsageStatsManager.STANDBY_BUCKET_FREQUENT -> "frequent"
        UsageStatsManager.STANDBY_BUCKET_RARE -> "rare"
        45 -> "restricted"
        null -> "unknown"
        else -> "other"
    }

    private fun linkName(link: Link): String = when (link) {
        is Link.Online -> "online"
        else -> link.javaClass.simpleName.lowercase()
    }
}
