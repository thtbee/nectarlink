// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import android.app.Notification
import android.app.PendingIntent
import android.app.Person
import android.content.Context
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.service.notification.StatusBarNotification
import android.telecom.TelecomManager
import android.util.Log
import app.nectarlink.android.notifications.ActionStarts
import app.nectarlink.android.notifications.NotificationListener
import app.nectarlink.core.Call
import app.nectarlink.core.CallCommand
import app.nectarlink.core.CallPhase

/**
 * Calls in other apps (WhatsApp, Signal, Messenger...) from their call
 * notifications (#434), reported to PCs like the phone's own calls but
 * naming the app. A PC's Answer, Decline and End press the notification's
 * own buttons, and its volume buttons the phone's call volume; the call's
 * sound stays on the phone.
 *
 * One call per app: answering often swaps the ringing notification for a
 * new one, so a call ends only when its app has shown none for [END_AFTER_MS].
 */
internal class AppCalls(private val context: Context, private val onChange: (Call) -> Unit) {
    private class Tracked(
        val pkg: String,
        val id: String,
        var key: String,
        var phase: CallPhase,
        val incoming: Boolean,
        var since: Long?,
        var name: String?,
        val app: String,
        var answered: Boolean = false,
        var declinedHere: Boolean = false,
    )

    private val byPkg = HashMap<String, Tracked>()
    private val endings = HashMap<String, Runnable>()
    private val main = Handler(Looper.getMainLooper())

    private fun dialers(): Set<String> = runCatching {
        val telecom = context.getSystemService(TelecomManager::class.java)
        setOfNotNull(
            telecom?.defaultDialerPackage,
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) telecom?.systemDialerPackage else null,
        )
    }.getOrDefault(emptySet())

    private fun callType(n: Notification): Int =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            n.extras.getInt(EXTRA_CALL_TYPE, AppCallRules.CALL_TYPE_NONE)
        } else {
            AppCallRules.CALL_TYPE_NONE
        }

    /** Whether [sbn] is a call in another app, which this reports instead of mirroring it as a notification. */
    fun isAppCall(sbn: StatusBarNotification): Boolean {
        val n = sbn.notification
        val ongoing = n.flags and (Notification.FLAG_ONGOING_EVENT or Notification.FLAG_FOREGROUND_SERVICE) != 0
        return AppCallRules.isAppCall(
            sbn.packageName,
            n.category,
            callType(n),
            ongoing || n.fullScreenIntent != null,
            context.packageName,
            dialers(),
        )
    }

    /**
     * A call notification was posted or updated. [caller] and [photo] are read
     * by the notification reader; [app] is the app's name.
     */
    @Synchronized
    fun posted(sbn: StatusBarNotification, caller: String?, photo: () -> ByteArray?, app: String) {
        lastSeen[sbn.packageName] = android.os.SystemClock.elapsedRealtime()
        val n = sbn.notification
        val showsTimer = n.extras.getBoolean(Notification.EXTRA_SHOW_CHRONOMETER, false) && n.`when` > 0L
        val phase = AppCallRules.phase(callType(n), showsTimer)
        endings.remove(sbn.packageName)?.let(main::removeCallbacks)
        val known = byPkg[sbn.packageName]
        if (known == null) {
            val call = Tracked(
                pkg = sbn.packageName,
                id = "app:" + Integer.toHexString(sbn.packageName.hashCode()) + "-" + (sbn.postTime % 1_000_000),
                key = sbn.key,
                phase = phase,
                incoming = phase == CallPhase.RINGING,
                since = if (phase == CallPhase.ACTIVE) startOf(n) else null,
                name = caller,
                app = app,
                answered = phase == CallPhase.ACTIVE,
            )
            byPkg[sbn.packageName] = call
            report(call, if (phase == CallPhase.RINGING) photo() else null)
            return
        }
        known.key = sbn.key
        if (caller != null) known.name = caller
        if (phase != known.phase) {
            known.phase = phase
            if (phase == CallPhase.ACTIVE) {
                known.answered = true
                known.since = startOf(n)
            }
            report(known, null)
        }
    }

    /** A call notification went away: the call ends unless its app shows another soon. */
    @Synchronized
    fun removed(sbn: StatusBarNotification) {
        val call = byPkg[sbn.packageName]?.takeIf { it.key == sbn.key } ?: return
        val end = Runnable {
            synchronized(this) {
                if (byPkg[call.pkg] !== call) return@Runnable
                byPkg.remove(call.pkg)
                endings.remove(call.pkg)
                report(call, null, ended = true)
            }
        }
        endings.put(call.pkg, end)?.let(main::removeCallbacks)
        main.postDelayed(end, END_AFTER_MS)
    }

    private fun startOf(n: Notification): Long =
        if (n.`when` in 1..System.currentTimeMillis()) n.`when` else System.currentTimeMillis()

    private fun report(call: Tracked, photo: ByteArray?, ended: Boolean = false) {
        onChange(
            Call(
                id = call.id,
                state = if (ended) CallPhase.ENDED else call.phase,
                incoming = call.incoming,
                number = null,
                name = call.name?.let { PhoneCalls.truncateUtf8(it, MAX_TEXT_BYTES) }?.takeIf { it.isNotEmpty() },
                photo = photo,
                missed = ended && call.incoming && !call.answered && !call.declinedHere,
                since = call.since.takeIf { !ended && call.phase == CallPhase.ACTIVE },
                controls = null,
                app = PhoneCalls.truncateUtf8(call.app, MAX_TEXT_BYTES),
            ),
        )
    }

    /**
     * A PC's Answer, Decline or End for an app call; `null` when [id] isn't
     * one (it's the phone's own call).
     */
    fun command(id: String, command: CallCommand): Boolean? {
        if (!id.startsWith("app:")) return null
        val call = synchronized(this) { byPkg.values.firstOrNull { it.id == id } } ?: return false
        // These apps talk on the call stream too, so the PC's volume buttons work as for phone calls.
        if (command is CallCommand.Volume) {
            context.getSystemService(android.media.AudioManager::class.java).adjustStreamVolume(
                android.media.AudioManager.STREAM_VOICE_CALL,
                if (command.up) android.media.AudioManager.ADJUST_RAISE else android.media.AudioManager.ADJUST_LOWER,
                0,
            )
            return true
        }
        val sbn = NotificationListener.instance?.find(call.key) ?: return false
        val n = sbn.notification
        val titles = n.actions.orEmpty().map { it.title?.toString().orEmpty() }
        fun action(index: Int?): PendingIntent? = index?.let { n.actions?.getOrNull(it)?.actionIntent }
        val ringing = call.phase == CallPhase.RINGING
        val intent = when (command) {
            CallCommand.Answer -> style(n, EXTRA_ANSWER_INTENT) ?: action(AppCallRules.answerIndex(titles))
            CallCommand.Decline -> if (ringing) {
                style(n, EXTRA_DECLINE_INTENT) ?: action(AppCallRules.endIndex(titles, ringing = true))
            } else {
                style(n, EXTRA_HANG_UP_INTENT) ?: action(AppCallRules.endIndex(titles, ringing = false))
            }
            else -> null
        } ?: return false
        if (command == CallCommand.Decline && ringing) synchronized(this) { call.declinedHere = true }
        return runCatching {
            intent.send(context, 0, null, null, null, null, ActionStarts.options())
            true
        }.getOrElse {
            Log.w(TAG, "an app call's button didn't work", it)
            false
        }
    }

    /** CallStyle's own answer, decline or hang-up intent (Android 12+). */
    private fun style(n: Notification, key: String): PendingIntent? =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            @Suppress("DEPRECATION")
            n.extras.getParcelable(key) as? PendingIntent
        } else {
            null
        }

    companion object {
        /** When each app last showed a call notification ([android.os.SystemClock.elapsedRealtime]). */
        private val lastSeen = java.util.concurrent.ConcurrentHashMap<String, Long>()

        /** Whether [pkg] has shown a call notification since [since]: a call it started is under way. */
        fun seenSince(pkg: String, since: Long): Boolean = (lastSeen[pkg] ?: Long.MIN_VALUE) >= since

        private const val TAG = "AppCalls"
        private const val END_AFTER_MS = 1_500L
        /** `calls::MAX_TEXT_BYTES` in the protocol. */
        private const val MAX_TEXT_BYTES = 256

        // CallStyle's extras (Android 12+), by key: the constants are newer than this app's minimum.
        private const val EXTRA_CALL_TYPE = "android.callType"
        private const val EXTRA_CALL_PERSON = "android.callPerson"
        private const val EXTRA_ANSWER_INTENT = "android.answerIntent"
        private const val EXTRA_DECLINE_INTENT = "android.declineIntent"
        private const val EXTRA_HANG_UP_INTENT = "android.hangUpIntent"

        private fun callPerson(n: Notification): Person? =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                @Suppress("DEPRECATION")
                n.extras.getParcelable(EXTRA_CALL_PERSON) as? Person
            } else {
                null
            }

        /** The caller CallStyle names (Android 12+). */
        fun callerName(n: Notification): String? =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) callPerson(n)?.name?.toString()?.trim()?.ifEmpty { null } else null

        /** The caller's picture CallStyle carries (Android 12+). */
        fun callerIcon(n: Notification): android.graphics.drawable.Icon? =
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) callPerson(n)?.icon else null
    }
}
