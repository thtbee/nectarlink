// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.mirror

import android.graphics.Rect
import android.view.accessibility.AccessibilityNodeInfo
import java.util.Locale

/**
 * Finds a chat app's Send button. Message boxes (WhatsApp, Telegram,
 * Messenger) take the keyboard's action without doing anything, so Enter
 * sends by pressing the button next to the box instead.
 */
internal object SendButton {
    /** What a button the screen reader calls Send is labeled, in the app's languages. */
    private val LABELS = setOf(
        "send", "send message", "senden", "enviar", "enviar mensaje", "enviar mensagem",
        "gönder", "gửi", "发送",
    )

    /** Where something is on the screen. */
    data class Box(val left: Int, val top: Int, val right: Int, val bottom: Int)

    /** One clickable node, as far as choosing it goes. */
    class Candidate(
        val id: String?,
        val label: String?,
        val enabled: Boolean,
        val bounds: Box,
    )

    fun isSendId(id: String?): Boolean {
        val name = id?.substringAfter(":id/", "")?.lowercase(Locale.ROOT) ?: return false
        return name == "send" || name.startsWith("send_") || name.endsWith("_send") ||
            name.contains("send_button") || name.contains("sendbutton")
    }

    fun isSendLabel(label: String?): Boolean =
        label?.trim()?.lowercase(Locale.ROOT)?.let { it in LABELS } == true

    /**
     * The Send button for the box at [field], or null: enabled, named Send by
     * its id or label, and of those the closest to the box.
     */
    fun pick(field: Box, candidates: List<Candidate>): Candidate? =
        candidates
            .filter { it.enabled && (isSendId(it.id) || isSendLabel(it.label)) }
            .minByOrNull { distance(field, it.bounds) }

    /** Gap between two rectangles, 0 when they touch or overlap. */
    private fun distance(a: Box, b: Box): Long {
        val dx = maxOf(0, maxOf(a.left - b.right, b.left - a.right)).toLong()
        val dy = maxOf(0, maxOf(a.top - b.bottom, b.top - a.bottom)).toLong()
        return dx * dx + dy * dy
    }

    /** Presses the Send button by the box [field]; false if there's none. */
    fun press(field: AccessibilityNodeInfo, root: AccessibilityNodeInfo?): Boolean {
        if (root == null) return false
        val nodes = ArrayList<AccessibilityNodeInfo>()
        val found = ArrayList<Candidate>()
        collect(root, nodes, found, depth = 0)
        val box = boxOf(field)
        val chosen = pick(box, found) ?: return false
        return nodes[found.indexOf(chosen)].performAction(AccessibilityNodeInfo.ACTION_CLICK)
    }

    /**
     * Every node named like a button, with what a tap on it presses: itself,
     * or the clickable view around it (apps often label the icon inside).
     */
    private fun collect(
        node: AccessibilityNodeInfo?,
        nodes: MutableList<AccessibilityNodeInfo>,
        found: MutableList<Candidate>,
        depth: Int,
        clickable: AccessibilityNodeInfo? = null,
    ) {
        if (node == null || depth > MAX_DEPTH || nodes.size >= MAX_NODES || !node.isVisibleToUser) return
        val target = if (node.isClickable) node else clickable
        val label = node.contentDescription?.toString() ?: node.text?.toString()
        if (target != null && (isSendId(node.viewIdResourceName) || isSendLabel(label))) {
            nodes += target
            found += Candidate(node.viewIdResourceName, label, node.isEnabled && target.isEnabled, boxOf(target))
        }
        for (i in 0 until node.childCount) collect(node.getChild(i), nodes, found, depth + 1, target)
    }

    private fun boxOf(node: AccessibilityNodeInfo): Box {
        val r = Rect().also(node::getBoundsInScreen)
        return Box(r.left, r.top, r.right, r.bottom)
    }

    private const val MAX_DEPTH = 40
    private const val MAX_NODES = 400
}
