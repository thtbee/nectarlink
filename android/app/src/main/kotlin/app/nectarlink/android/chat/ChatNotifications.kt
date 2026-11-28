// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.chat

import android.Manifest
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import androidx.core.content.ContextCompat
import app.nectarlink.android.R
import app.nectarlink.android.ui.MainActivity
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

/**
 * A note from a paired device (#698) while its chat isn't on screen: a
 * notification with the device's name and what it wrote, one per device.
 * Tapping opens that chat; opening the chat clears it.
 */
object ChatNotifications {
    private const val CHANNEL = "chat"
    private const val TAG = "chat"

    /** On MainActivity's intent: open the chat with this device. */
    const val EXTRA_OPEN_CHAT = "open_chat"

    /** The notes a notification shows at most, newest last. */
    private const val SHOWN_NOTES = 6

    /** The device whose chat is on screen now, while the app is in front. */
    @Volatile
    var onScreen: String? = null

    private val pending = HashMap<String, ArrayDeque<Pair<String, Long>>>()

    private val _open = MutableStateFlow<String?>(null)

    /** A chat to open, asked by a notification. */
    val open: StateFlow<String?> = _open

    fun requestOpen(deviceId: String) {
        _open.value = deviceId
    }

    fun opened() {
        _open.value = null
    }

    /** Shows [text] from [deviceId], unless its chat is on screen. */
    fun show(context: Context, deviceId: String, deviceName: String, text: String) {
        if (onScreen == deviceId) return
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) !=
            PackageManager.PERMISSION_GRANTED
        ) {
            return
        }
        createChannel(context)
        val name = deviceName.ifBlank { context.getString(R.string.your_pc) }
        val notes = synchronized(pending) {
            val queue = pending.getOrPut(deviceId) { ArrayDeque() }
            queue.addLast(text to System.currentTimeMillis())
            while (queue.size > SHOWN_NOTES) queue.removeFirst()
            queue.toList()
        }
        val device = Person.Builder().setName(name).setKey(deviceId).build()
        val style = NotificationCompat.MessagingStyle(
            Person.Builder().setName(context.getString(R.string.chat_you)).build(),
        )
        notes.forEach { (note, at) -> style.addMessage(note, at, device) }
        val notification = NotificationCompat.Builder(context, CHANNEL)
            .setSmallIcon(R.drawable.ic_notification)
            .setContentTitle(name)
            .setContentText(text)
            .setStyle(style)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setContentIntent(openChat(context, deviceId))
            .setAutoCancel(true)
            .build()
        runCatching { NotificationManagerCompat.from(context).notify(TAG, deviceId.hashCode(), notification) }
    }

    /** Clears the notification for [deviceId]'s chat. */
    fun cancel(context: Context, deviceId: String) {
        synchronized(pending) { pending.remove(deviceId) }
        runCatching { NotificationManagerCompat.from(context).cancel(TAG, deviceId.hashCode()) }
    }

    private fun createChannel(context: Context) {
        context.getSystemService(NotificationManager::class.java)?.createNotificationChannel(
            NotificationChannel(CHANNEL, context.getString(R.string.channel_chat), NotificationManager.IMPORTANCE_DEFAULT),
        )
    }

    private fun openChat(context: Context, deviceId: String): PendingIntent = PendingIntent.getActivity(
        context,
        deviceId.hashCode(),
        Intent(context, MainActivity::class.java)
            .putExtra(EXTRA_OPEN_CHAT, deviceId)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
}
