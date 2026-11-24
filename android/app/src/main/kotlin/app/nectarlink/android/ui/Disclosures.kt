// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui

import android.Manifest
import android.content.Context
import androidx.activity.result.ActivityResultLauncher
import androidx.annotation.StringRes
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import androidx.core.content.edit
import app.nectarlink.android.BuildConfig
import app.nectarlink.android.R
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * What NectarLink does with personal data, said in the app before Android's
 * own prompt or settings page for it, with Continue and Not now (Google Play's
 * prominent disclosure). Once per kind: Continue is remembered.
 *
 * Only in the Google Play build; the GitHub build goes straight to Android's
 * prompt, as each feature's own screen already says what it's for.
 */
object Disclosures {
    enum class Kind(@StringRes val title: Int, @StringRes val text: Int) {
        Messages(R.string.disclosure_messages_title, R.string.disclosure_messages_text),
        Calls(R.string.disclosure_calls_title, R.string.disclosure_calls_text),
        Contacts(R.string.disclosure_contacts_title, R.string.disclosure_contacts_text),
        Photos(R.string.disclosure_photos_title, R.string.disclosure_photos_text),
        Notifications(R.string.disclosure_notifications_title, R.string.disclosure_notifications_text),
        Control(R.string.disclosure_control_title, R.string.disclosure_control_text),
        Elevated(R.string.disclosure_elevated_title, R.string.disclosure_elevated_text),
    }

    /** Kinds waiting for an answer, and what runs on Continue. */
    class Ask(val kinds: List<Kind>, val then: () -> Unit)

    private const val PREFS = "disclosures"
    private val _asking = MutableStateFlow<Ask?>(null)
    val asking: StateFlow<Ask?> = _asking.asStateFlow()

    /** Runs [then] once the user has read and continued past each of [kinds] (at once, when they have). */
    fun before(context: Context, vararg kinds: Kind, then: () -> Unit) {
        val prefs = context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        val unread = if (BuildConfig.PLAY) kinds.distinct().filterNot { prefs.getBoolean(it.name, false) } else emptyList()
        if (unread.isEmpty()) then() else _asking.value = Ask(unread, then)
    }

    /** [before], for the kinds Android's runtime [permissions] are about. */
    fun beforePermissions(context: Context, permissions: Array<String>, then: () -> Unit) =
        before(context, *kindsOf(permissions).toTypedArray(), then = then)

    internal fun kindsOf(permissions: Array<String>): List<Kind> = permissions.mapNotNull { permission ->
        when (permission) {
            Manifest.permission.READ_SMS, Manifest.permission.SEND_SMS, Manifest.permission.RECEIVE_SMS -> Kind.Messages
            Manifest.permission.READ_CALL_LOG, Manifest.permission.READ_PHONE_STATE,
            Manifest.permission.CALL_PHONE, Manifest.permission.ANSWER_PHONE_CALLS,
            Manifest.permission.READ_PHONE_NUMBERS -> Kind.Calls
            Manifest.permission.READ_CONTACTS -> Kind.Contacts
            Manifest.permission.READ_MEDIA_IMAGES, Manifest.permission.READ_MEDIA_VIDEO,
            Manifest.permission.READ_MEDIA_VISUAL_USER_SELECTED, Manifest.permission.READ_EXTERNAL_STORAGE -> Kind.Photos
            else -> null
        }
    }.distinct()

    internal fun answer(context: Context, ask: Ask, continued: Boolean) {
        if (_asking.value === ask) _asking.value = null
        if (!continued) return
        context.applicationContext.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit {
            ask.kinds.forEach { putBoolean(it.name, true) }
        }
        ask.then()
    }
}

/** [ActivityResultLauncher.launch] for runtime permissions, after their disclosure. */
fun ActivityResultLauncher<Array<String>>.launchDisclosed(context: Context, permissions: Array<String>) =
    Disclosures.beforePermissions(context, permissions) { launch(permissions) }

/** Shows the disclosure waiting for an answer, if any. Once, at the app's root. */
@Composable
fun DisclosureHost() {
    val ask by Disclosures.asking.collectAsState()
    val current = ask ?: return
    val context = LocalContext.current
    AlertDialog(
        onDismissRequest = { Disclosures.answer(context, current, continued = false) },
        title = { Text(stringResource(current.kinds.first().title)) },
        text = {
            Column(
                verticalArrangement = Arrangement.spacedBy(12.dp),
                modifier = androidx.compose.ui.Modifier.verticalScroll(rememberScrollState()),
            ) {
                current.kinds.forEach { kind ->
                    if (current.kinds.size > 1) {
                        Text(stringResource(kind.title), style = MaterialTheme.typography.titleSmall)
                    }
                    Text(stringResource(kind.text), style = MaterialTheme.typography.bodyMedium)
                }
                Text(
                    stringResource(R.string.disclosure_private),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        },
        confirmButton = {
            Button(onClick = { Disclosures.answer(context, current, continued = true) }) {
                Text(stringResource(R.string.disclosure_continue))
            }
        },
        dismissButton = {
            TextButton(onClick = { Disclosures.answer(context, current, continued = false) }) {
                Text(stringResource(R.string.action_not_now))
            }
        },
    )
}
