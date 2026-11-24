// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.sms

/** Telling which of the phone's notifications is a text conversation's, and its "Mark as read" button. */
internal object SmsNotificationMatch {
    /** The digits of a number, "+1 (555) 010-0" as "15550100". */
    private fun digits(text: String): String = text.filter { it in '0'..'9' }

    /** Fewest digits that tell two numbers apart when one lacks the other's country code. */
    private const val MIN_DIGITS = 7

    /** Whether two numbers are the same, one perhaps with a country code the other leaves out. */
    private fun sameNumber(a: String, b: String): Boolean {
        val (short, long) = if (a.length <= b.length) a to b else b to a
        return short.length >= MIN_DIGITS && long.endsWith(short)
    }

    /**
     * Whether a notification titled [title] is from a conversation with [people] (their
     * contact names and numbers): the title is a name, or a number that is one of theirs.
     */
    fun titleMatches(title: String, people: List<String>): Boolean {
        val wanted = title.trim()
        if (wanted.isEmpty()) return false
        if (people.any { it.trim().equals(wanted, ignoreCase = true) }) return true
        // A number as a title is only digits and the usual punctuation.
        if (wanted.any { it !in "0123456789 +-().".toSet() }) return false
        val number = digits(wanted)
        return people.any { sameNumber(digits(it), number) }
    }

    /** Labels of a "Mark as read" button, for apps that don't tag it as one. */
    private val MARK_AS_READ_LABELS = setOf("mark as read", "mark read")

    /** Whether a button called [title], tagged [semantic] (Android's own kind of action), marks the conversation read. */
    fun marksRead(semantic: Int, title: CharSequence?, markAsReadSemantic: Int): Boolean =
        semantic == markAsReadSemantic || title?.toString()?.trim()?.lowercase() in MARK_AS_READ_LABELS
}
