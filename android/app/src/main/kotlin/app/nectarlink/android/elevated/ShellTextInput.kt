// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.elevated

import android.accessibilityservice.AccessibilityServiceInfo
import android.annotation.SuppressLint
import android.app.UiAutomation
import android.os.Build
import android.os.Bundle
import android.os.HandlerThread
import android.os.Looper
import android.os.SystemClock
import android.view.accessibility.AccessibilityNodeInfo

/**
 * Text in an Elevated app display, without the device-wide clipboard. Android
 * blocks clipboard reads while the physical phone is locked, even though our
 * ALWAYS_UNLOCKED app display still takes keys. The shell may edit its focused
 * field through accessibility, without requiring a separate enabled service.
 */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi", "BlockedPrivateApi")
internal object ShellTextInput {
    private var retryAfter = 0L
    private var automation: UiAutomation? = null
    private var callbacks: HandlerThread? = null

    /** Only an explicitly targeted virtual display; never the phone's lock screen. */
    fun insert(display: Int, text: String): Boolean {
        if (display <= 0 || Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return false
        val ui = connection() ?: return false
        return runCatching {
            val windows = ui.windowsOnAllDisplays[display].orEmpty()
            val focused = windows.firstNotNullOfOrNull { window ->
                window.root?.findFocus(AccessibilityNodeInfo.FOCUS_INPUT)
            } ?: return false
            if (!focused.refresh() || focused.isPassword || !focused.isEditable) return false
            val old = if (focused.isShowingHintText) "" else focused.text?.toString().orEmpty()
            val anchor = focused.textSelectionStart.takeIf { it in 0..old.length } ?: old.length
            val extent = focused.textSelectionEnd.takeIf { it in 0..old.length } ?: anchor
            val start = minOf(anchor, extent)
            val end = maxOf(anchor, extent)
            val replacement = old.replaceRange(start, end, text)
            val changed = focused.performAction(
                AccessibilityNodeInfo.ACTION_SET_TEXT,
                Bundle().apply { putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, replacement) },
            )
            if (!changed) return false
            val cursor = start + text.length
            focused.performAction(
                AccessibilityNodeInfo.ACTION_SET_SELECTION,
                Bundle().apply {
                    putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, cursor)
                    putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, cursor)
                },
            )
            true
        }.onFailure {
            close()
            retryAfter = SystemClock.uptimeMillis() + 5_000
            System.err.println("can't insert text on app display $display: ${it.javaClass.simpleName}")
        }.getOrDefault(false)
    }

    private fun connection(): UiAutomation? {
        automation?.let { return it }
        if (SystemClock.uptimeMillis() < retryAfter) return null
        retryAfter = SystemClock.uptimeMillis() + 5_000
        val thread = HandlerThread("app-text-input").apply { start() }
        return runCatching {
            val binderType = Class.forName("android.app.IUiAutomationConnection")
            val binder = Class.forName("android.app.UiAutomationConnection").getDeclaredConstructor().newInstance()
            val ui = UiAutomation::class.java.getConstructor(Looper::class.java, binderType)
                .newInstance(thread.looper, binder) as UiAutomation
            // Keep the user's existing accessibility services running.
            UiAutomation::class.java.getMethod("connect", Int::class.javaPrimitiveType)
                .invoke(ui, UiAutomation.FLAG_DONT_SUPPRESS_ACCESSIBILITY_SERVICES)
            callbacks = thread
            automation = ui
            ui.serviceInfo = ui.serviceInfo.apply {
                flags = flags or AccessibilityServiceInfo.FLAG_RETRIEVE_INTERACTIVE_WINDOWS
            }
            ui
        }.onFailure {
            close()
            thread.quitSafely()
            System.err.println("app text accessibility unavailable: ${it.javaClass.simpleName}")
        }.getOrNull()
    }

    fun close() {
        automation?.let { runCatching { UiAutomation::class.java.getMethod("disconnect").invoke(it) } }
        automation = null
        callbacks?.quitSafely()
        callbacks = null
    }
}
