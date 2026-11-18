// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.app.Activity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.BatteryManager
import android.os.SystemClock
import android.view.MotionEvent
import android.view.Window
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalView
import androidx.core.content.ContextCompat
import kotlinx.coroutines.delay

/**
 * How long a screen that's in use only while it's touched (the touchpad, Deck, Desk)
 * keeps the display on after the last touch. Then it sleeps as the phone's own
 * timeout says, unless something else keeps it on (charging, for Deck and Desk).
 */
const val KEEP_AWAKE_AFTER_TOUCH_MS = 10 * 60_000L

/** Keeps the display on while [on] and this screen is shown. */
@Composable
fun KeepScreenOn(on: Boolean) {
    val view = LocalView.current
    DisposableEffect(view, on) {
        if (!on) return@DisposableEffect onDispose {}
        val before = view.keepScreenOn
        view.keepScreenOn = true
        onDispose { view.keepScreenOn = before }
    }
}

/**
 * Whether the screen was touched within [timeoutMs]: true when it opens, and again
 * from each touch anywhere in the window. Touches are only watched, never consumed.
 */
@Composable
fun rememberTouchedRecently(timeoutMs: Long = KEEP_AWAKE_AFTER_TOUCH_MS): Boolean {
    val context = LocalContext.current
    var lastTouch by remember { mutableLongStateOf(SystemClock.uptimeMillis()) }
    var recent by remember { mutableStateOf(true) }
    DisposableEffect(context) {
        val window = (context as? Activity)?.window ?: return@DisposableEffect onDispose {}
        val original = window.callback
        val watching = object : Window.Callback by original {
            override fun dispatchTouchEvent(event: MotionEvent): Boolean {
                if (event.actionMasked == MotionEvent.ACTION_DOWN) lastTouch = SystemClock.uptimeMillis()
                return original.dispatchTouchEvent(event)
            }
        }
        window.callback = watching
        onDispose {
            // Only if nothing replaced it since: then it's ours to undo.
            if (window.callback === watching) window.callback = original
        }
    }
    LaunchedEffect(lastTouch, timeoutMs) {
        recent = true
        delay(timeoutMs)
        recent = false
    }
    return recent
}

/** Whether the phone is on a charger, kept up to date while this screen is shown. */
@Composable
fun rememberCharging(): Boolean {
    val context = LocalContext.current
    var charging by remember { mutableStateOf(false) }
    DisposableEffect(context) {
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context, intent: Intent) {
                charging = intent.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) != 0
            }
        }
        // Sticky: registering also delivers the current state.
        val current = ContextCompat.registerReceiver(
            context,
            receiver,
            IntentFilter(Intent.ACTION_BATTERY_CHANGED),
            ContextCompat.RECEIVER_NOT_EXPORTED,
        )
        if (current != null) charging = current.getIntExtra(BatteryManager.EXTRA_PLUGGED, 0) != 0
        onDispose { runCatching { context.unregisterReceiver(receiver) } }
    }
    return charging
}
