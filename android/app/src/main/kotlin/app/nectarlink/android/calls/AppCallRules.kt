// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.calls

import app.nectarlink.core.CallPhase

/**
 * What a call notification from another app (WhatsApp, Signal, Messenger...)
 * means (#434): whether it's a call at all, ringing or answered, and which
 * of its buttons answers it and which declines or ends it. Kept apart from
 * Android's types so it can be tested on its own.
 */
internal object AppCallRules {
    /** `Notification.EXTRA_CALL_TYPE` values (CallStyle, Android 12+). */
    const val CALL_TYPE_NONE = 0
    const val CALL_TYPE_INCOMING = 1
    const val CALL_TYPE_ONGOING = 2
    const val CALL_TYPE_SCREENING = 3

    /** `Notification.CATEGORY_CALL`. */
    private const val CATEGORY_CALL = "call"

    /** Packages that are the phone's own calls: [PhoneCalls] reports those already. */
    private val TELEPHONY = setOf("com.android.server.telecom", "com.android.phone")

    /**
     * Whether a notification is a call in another app. The dialer's own
     * call notifications aren't: those calls reach PCs as phone calls. A
     * call is CallStyle, or in the call category and ongoing or full-screen
     * (a missed-call notice some apps file under "call" is neither).
     */
    fun isAppCall(
        pkg: String,
        category: String?,
        callType: Int,
        ongoingOrFullScreen: Boolean,
        own: String,
        dialers: Set<String>,
    ): Boolean {
        if (pkg == own || pkg in dialers || pkg in TELEPHONY || pkg.contains("incallui")) return false
        return callType != CALL_TYPE_NONE || (category == CATEGORY_CALL && ongoingOrFullScreen)
    }

    /**
     * Ringing or answered. CallStyle says so; other call notifications show
     * a running timer once the call is answered.
     */
    fun phase(callType: Int, showsTimer: Boolean): CallPhase = when (callType) {
        CALL_TYPE_INCOMING, CALL_TYPE_SCREENING -> CallPhase.RINGING
        CALL_TYPE_ONGOING -> CallPhase.ACTIVE
        else -> if (showsTimer) CallPhase.ACTIVE else CallPhase.RINGING
    }

    private val ANSWER = listOf(
        "answer", "accept", "pick up", "aceptar", "contestar", "responder", "atender", "annehmen",
        "принять", "ответить", "yanıtla", "cevapla", "kabul", "trả lời", "nghe", "接听", "接受",
    )
    private val END = listOf(
        "decline", "reject", "hang up", "end", "dismiss", "ignore", "rechazar", "colgar", "recusar",
        "desligar", "encerrar", "ablehnen", "auflegen", "beenden", "отклонить", "завершить", "сбросить",
        "reddet", "kapat", "từ chối", "kết thúc", "拒绝", "挂断", "结束",
    )

    /** Whole words, so "end" isn't found in "Send"; scripts without spaces (Chinese) by substring. */
    private fun matches(title: String, words: List<String>): Boolean {
        val t = title.lowercase()
        return words.any { w ->
            if (w.all { it.code < 0x2E80 }) Regex("(^|[^\\p{L}])" + Regex.escape(w) + "($|[^\\p{L}])").containsMatchIn(t)
            else t.contains(w)
        }
    }

    /**
     * The button that answers a ringing call, for a notification without
     * CallStyle's: by its title, else the last of two or more (these apps
     * put Decline first and Answer last).
     */
    fun answerIndex(titles: List<String>): Int? =
        titles.indexOfFirst { matches(it, ANSWER) }.takeIf { it >= 0 }
            ?: titles.lastIndex.takeIf { titles.size >= 2 }

    /**
     * The button that declines a ringing call or ends an answered one: by its
     * title, else the first of a ringing call's two or more, or an answered
     * call's only button.
     */
    fun endIndex(titles: List<String>, ringing: Boolean): Int? {
        val answer = if (ringing) answerIndex(titles) else null
        titles.indices.firstOrNull { it != answer && matches(titles[it], END) }?.let { return it }
        return when {
            ringing && titles.size >= 2 -> 0.takeIf { it != answer }
            !ringing && titles.size == 1 -> 0
            else -> null
        }
    }
}
