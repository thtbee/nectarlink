// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.clipboard

import android.app.Person
import android.content.Context
import android.content.Intent
import android.content.pm.ShortcutInfo
import android.content.pm.ShortcutManager
import android.graphics.drawable.Icon
import android.os.Build
import app.nectarlink.android.R
import app.nectarlink.android.core.Device

/**
 * Publishes Direct Share targets (Android 10+ dynamic sharing shortcuts) for each
 * paired PC so paired PCs appear directly in Android's system share sheet.
 */
object DirectShareTargets {
    const val CATEGORY_DIRECT_SHARE = "app.nectarlink.android.category.DIRECT_SHARE"
    private const val SHORTCUT_PREFIX = "pc:"

    private data class Snapshot(
        val id: String,
        val name: String,
        val online: Boolean,
    )

    @Volatile
    private var lastSynced: List<Snapshot>? = null

    fun pcIdFromShortcutId(shortcutId: String?): String? =
        shortcutId?.takeIf { it.startsWith(SHORTCUT_PREFIX) }?.removePrefix(SHORTCUT_PREFIX)?.takeIf { it.isNotBlank() }

    fun sync(context: Context, devices: List<Device>) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) return
        val snapshot = devices.map { Snapshot(id = it.id, name = it.name, online = it.online) }
        if (snapshot == lastSynced) return
        val previousIds = lastSynced?.map { "$SHORTCUT_PREFIX${it.id}" }?.toSet().orEmpty()
        lastSynced = snapshot

        val shortcutManager = context.getSystemService(ShortcutManager::class.java) ?: return
        val maxShortcuts = shortcutManager.maxShortcutCountPerActivity.coerceIn(1, 8)
        val sorted = devices.sortedByDescending { it.online }.take(maxShortcuts)
        val fallbackName = context.getString(R.string.your_pc)

        val shortcuts = sorted.mapIndexed { index, device ->
            val label = device.name.ifBlank { fallbackName }
            val person = Person.Builder()
                .setName(label)
                .setKey(device.id)
                .build()
            val intent = Intent(context, SendActivity::class.java)
                .setAction(Intent.ACTION_SEND)
                .putExtra(SendActivity.EXTRA_PC_ID, device.id)
            ShortcutInfo.Builder(context, "$SHORTCUT_PREFIX${device.id}")
                .setShortLabel(label)
                .setLongLabel(label)
                .setIcon(Icon.createWithResource(context, R.mipmap.ic_launcher))
                .setCategories(setOf(CATEGORY_DIRECT_SHARE))
                .setLongLived(true)
                .setRank(index)
                .setPerson(person)
                .setIntent(intent)
                .build()
        }

        runCatching {
            shortcutManager.dynamicShortcuts = shortcuts
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                val activeIds = shortcuts.map { it.id }.toSet()
                val removed = (previousIds - activeIds).toList()
                if (removed.isNotEmpty()) {
                    shortcutManager.removeLongLivedShortcuts(removed)
                }
            }
        }
    }
}
