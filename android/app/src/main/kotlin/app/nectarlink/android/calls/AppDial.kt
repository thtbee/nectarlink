// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.ContentUris
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.provider.ContactsContract
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.ProcessLifecycleOwner
import app.nectarlink.android.R
import app.nectarlink.core.CallSim

/**
 * Calls through a calling app from the PC (#434): WhatsApp and WhatsApp
 * Business add a "voice call" row to each contact they know, and opening that
 * row starts the call in the app, as tapping it in Contacts does. They offer
 * no other way for another app to start a call.
 *
 * The PC sees these apps next to the SIMs in "Call with" (`call.sims`), with
 * an id of `app:<package>`; an older PC just shows them as one more choice.
 * When Android won't let NectarLink open an app from the background, a
 * notification on the phone starts the call with a tap instead.
 */
internal object AppDial {
    private const val TAG = "AppDial"
    const val PREFIX = "app:"
    private const val CHANNEL = "app_calls"

    /** A calling app: its package, its call rows' MIME type, and its name. */
    internal data class App(val pkg: String, val mime: String, val label: String)

    internal val APPS = listOf(
        App("com.whatsapp", "vnd.android.cursor.item/vnd.com.whatsapp.voip.call", "WhatsApp"),
        App("com.whatsapp.w4b", "vnd.android.cursor.item/vnd.com.whatsapp.w4b.voip.call", "WhatsApp Business"),
    )

    /** The calling apps to offer: installed, with contacts readable to find the person. */
    fun choices(context: Context): List<CallSim> {
        if (!granted(context)) return emptyList()
        return APPS.filter { installed(context, it.pkg) }
            .map { CallSim(id = PREFIX + it.pkg, label = it.label, isDefault = false) }
    }

    /** Whether the app [sim] (`app:<package>`) is one of these and installed. */
    fun available(context: Context, sim: String): Boolean =
        APPS.any { PREFIX + it.pkg == sim && installed(context, it.pkg) }

    /**
     * Starts a call to [number] in the app [sim] (`app:<package>`). False when
     * the app isn't there, or the number isn't one contact of it for sure.
     */
    fun dial(context: Context, number: String, sim: String): Boolean {
        val app = APPS.firstOrNull { PREFIX + it.pkg == sim } ?: return false
        if (!granted(context) || !installed(context, app.pkg)) return false
        val row = findRow(context, app, number) ?: return false
        val intent = Intent(Intent.ACTION_VIEW)
            .setDataAndType(ContentUris.withAppendedId(ContactsContract.Data.CONTENT_URI, row.id), app.mime)
            .setPackage(app.pkg)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        // Opened right away only while NectarLink is on screen: from the
        // background Android may block it without saying, and a locked phone
        // would open the app behind the lock. Otherwise a tap starts it.
        if (inForeground() && runCatching { context.startActivity(intent) }.isSuccess) return true
        return notify(context, app, row.name ?: number, intent)
    }

    internal data class Row(val id: Long, val name: String?)

    /** The app's call row for [number], when exactly one of its numbers is that one ([pick]). */
    private fun findRow(context: Context, app: App, number: String): Row? {
        val wanted = digits(number)
        if (wanted.length < MIN_DIGITS) return null
        return runCatching {
            context.contentResolver.query(
                ContactsContract.Data.CONTENT_URI,
                arrayOf(ContactsContract.Data._ID, ContactsContract.Data.DATA1, ContactsContract.Data.DISPLAY_NAME),
                "${ContactsContract.Data.MIMETYPE} = ?",
                arrayOf(app.mime),
                null,
            )?.use { cursor ->
                val rows = buildList {
                    while (cursor.moveToNext()) {
                        add(Candidate(cursor.getLong(0), digits(cursor.getString(1).orEmpty().substringBefore('@')), cursor.getString(2)))
                    }
                }
                pick(rows, wanted)
            }
        }.onFailure { Log.w(TAG, "can't read ${app.label}'s contacts", it) }.getOrNull()
    }

    internal data class Candidate(val id: Long, val digits: String, val name: String?)

    /**
     * The row to call for [wanted]: the one with exactly those digits, or else
     * the only number that ends the same way. Two different numbers that
     * both could be it (the same national number in two countries) call
     * nobody, rather than maybe the wrong person.
     */
    internal fun pick(rows: List<Candidate>, wanted: String): Row? {
        rows.firstOrNull { it.digits == wanted }?.let { return Row(it.id, it.name) }
        val matches = rows.filter { sameNumber(it.digits, wanted) }
        if (matches.map { it.digits }.distinct().size != 1) return null
        return matches.first().let { Row(it.id, it.name) }
    }

    /**
     * Whether a call row's ID ("919876543210@s.whatsapp.net") is [wanted]
     * (digits only): the same last digits, as the PC may send a local number.
     */
    internal fun sameNumber(rowId: String, wanted: String): Boolean {
        val have = digits(rowId.substringBefore('@'))
        if (have == wanted && have.length >= MIN_DIGITS) return true
        // A local number has to be long enough to tell people apart.
        val n = minOf(have.length, wanted.length, COMPARED_DIGITS)
        return n >= SUFFIX_DIGITS && have.takeLast(n) == wanted.takeLast(n)
    }

    internal fun digits(text: String) = text.filter { it in '0'..'9' }

    private fun notify(context: Context, app: App, who: String, intent: Intent): Boolean {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return false
        }
        context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.app_call_channel), NotificationManager.IMPORTANCE_HIGH),
        )
        val open = PendingIntent.getActivity(
            context,
            NOTIFICATION_ID,
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(context.getString(R.string.app_call_title, who, app.label))
            .setContentText(context.getString(R.string.app_call_text))
            .setContentIntent(open)
            .setAutoCancel(true)
            .setTimeoutAfter(TIMEOUT_MS)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .build()
        return runCatching { NotificationManagerCompat.from(context).notify(TAG, NOTIFICATION_ID, notification) }.isSuccess
    }

    private fun inForeground(): Boolean =
        ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)

    private fun granted(context: Context) =
        ContextCompat.checkSelfPermission(context, Manifest.permission.READ_CONTACTS) == PackageManager.PERMISSION_GRANTED

    private fun installed(context: Context, pkg: String) =
        runCatching { context.packageManager.getPackageInfo(pkg, 0) }.isSuccess

    /** Fewer digits than this aren't a phone number to match on. */
    private const val MIN_DIGITS = 7
    /** Matching on the end of a number needs at least this much of it. */
    private const val SUFFIX_DIGITS = 9
    /** The national part of a number: enough to tell contacts apart. */
    private const val COMPARED_DIGITS = 10
    private const val NOTIFICATION_ID = 0x0A11
    /** A call nobody started in a minute is stale. */
    private const val TIMEOUT_MS = 60_000L
}
