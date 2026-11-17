// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.widget

import android.content.Context
import android.content.Intent
import android.os.Bundle
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.NectarlinkApplication
import app.nectarlink.android.R
import app.nectarlink.android.core.Core
import app.nectarlink.android.core.Device
import app.nectarlink.android.ui.Preferences
import app.nectarlink.android.ui.theme.NectarlinkTheme

/**
 * What a widget opens over the home screen, since widgets can't show menus
 * themselves: a PC's power menu, with its actions named as that PC's OS
 * names them, or the list of PCs to pick the one a widget shows.
 */
class WidgetMenuActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val core = (application as NectarlinkApplication).core
        core.start()
        val action = intent?.action
        val widgetId = intent?.getIntExtra(EXTRA_WIDGET_ID, -1) ?: -1
        val pcId = intent?.getStringExtra(EXTRA_PC_ID)
        if (action == ACTION_PICK && widgetId < 0 || action == ACTION_POWER && pcId == null || action == null) {
            finish()
            return
        }
        val preferences = Preferences(this)
        setContent {
            val appearance by preferences.appearance.collectAsStateWithLifecycle()
            val state by core.state.collectAsStateWithLifecycle()
            val pcs = Widgets.pcs(state)
            NectarlinkTheme(appearance = appearance) {
                Dialog(onDismissRequest = ::finish) {
                    Surface(
                        shape = RoundedCornerShape(28.dp),
                        color = MaterialTheme.colorScheme.surfaceContainerHigh,
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        Column(Modifier.padding(20.dp)) {
                            if (action == ACTION_PICK) {
                                PcPicker(pcs) { pc ->
                                    Widgets.choosePc(this@WidgetMenuActivity, widgetId, pc.id)
                                    finish()
                                }
                            } else {
                                PowerMenu(core, pcs, pcId!!)
                            }
                        }
                    }
                }
            }
        }
    }

    @Composable
    private fun PcPicker(pcs: List<Device>, onPick: (Device) -> Unit) {
        Text(
            stringResource(R.string.widget_choose_pc),
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(start = 4.dp, bottom = 12.dp),
        )
        Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
            pcs.forEach { pc -> PcRow(pc, onClick = { onPick(pc) }) }
        }
    }

    @Composable
    private fun PowerMenu(core: Core, pcs: List<Device>, initialPc: String) {
        var pcId by rememberSaveable { mutableStateOf(initialPc) }
        var choosing by remember { mutableStateOf(false) }
        var confirmOff by rememberSaveable { mutableStateOf(false) }
        val pc = pcs.firstOrNull { it.id == pcId } ?: pcs.firstOrNull()
        if (pc == null) {
            finish()
            return
        }
        val linux = pc.os == "linux"

        // The PC, which opens the others when there are more than one.
        PcRow(
            pc,
            chevron = if (pcs.size > 1) choosing else null,
            onClick = if (pcs.size > 1) ({ choosing = !choosing }) else null,
        )
        AnimatedVisibility(choosing) {
            Column(Modifier.padding(top = 6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                pcs.filter { it.id != pc.id }.forEach { other ->
                    PcRow(other, onClick = {
                        pcId = other.id
                        choosing = false
                        confirmOff = false
                    })
                }
            }
        }
        Spacer(Modifier.height(16.dp))

        val name = pc.name.ifBlank { stringResource(R.string.your_pc) }
        when {
            !pc.online -> {
                // The row above already says it's offline: say what that means here.
                Hint(stringResource(R.string.widget_power_offline, name))
                if (pc.canWake) {
                    Button(onClick = { core.wake(pc.id); done(R.string.widget_pc_waking_toast, name) }, modifier = Modifier.fillMaxWidth()) {
                        Text(stringResource(R.string.widget_pc_wake))
                    }
                }
            }
            !pc.has("device.pc_actions") -> Hint(stringResource(R.string.pc_actions_off, name))
            confirmOff -> {
                Text(
                    stringResource(if (linux) R.string.widget_power_off_title else R.string.shutdown_pc_title, name),
                    style = MaterialTheme.typography.titleMedium,
                )
                Spacer(Modifier.height(6.dp))
                Hint(stringResource(R.string.shutdown_pc_text))
                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                    TextButton(onClick = { confirmOff = false }) { Text(stringResource(R.string.action_cancel)) }
                    Spacer(Modifier.width(8.dp))
                    Button(onClick = { core.pcShutdown(pc.id); done(R.string.pc_shutting_down, name) }) {
                        Text(stringResource(if (linux) R.string.widget_power_off else R.string.action_shutdown_pc))
                    }
                }
            }
            else -> Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                PowerRow(stringResource(R.string.widget_power_lock)) {
                    core.pcPower(pc.id, sleep = false)
                    done(R.string.pc_locked, name)
                }
                PowerRow(stringResource(if (linux) R.string.widget_power_suspend else R.string.action_sleep_pc)) {
                    core.pcPower(pc.id, sleep = true)
                    done(R.string.pc_sleeping, name)
                }
                if (pc.has("device.pc_hibernate")) {
                    PowerRow(stringResource(R.string.action_hibernate_pc)) {
                        core.pcHibernate(pc.id)
                        done(R.string.pc_hibernating, name)
                    }
                }
                if (pc.has("device.pc_shutdown")) {
                    PowerRow(stringResource(if (linux) R.string.widget_power_off else R.string.action_shutdown_pc)) {
                        confirmOff = true
                    }
                }
            }
        }
    }

    private fun done(message: Int, name: String) {
        Toast.makeText(this, getString(message, name), Toast.LENGTH_SHORT).show()
        finish()
    }

    @Composable
    private fun Hint(text: String) {
        Text(
            text,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(start = 4.dp, bottom = 12.dp),
        )
    }

    /** A PC: its OS logo, name and whether it's connected. [chevron] is null without one, else whether it points up. */
    @Composable
    private fun PcRow(pc: Device, chevron: Boolean? = null, onClick: (() -> Unit)?) {
        val colors = MaterialTheme.colorScheme
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(18.dp))
                .background(colors.surfaceContainer)
                .then(if (onClick != null) Modifier.clickable(role = Role.Button, onClick = onClick) else Modifier)
                .padding(horizontal = 12.dp, vertical = 10.dp),
        ) {
            Box(
                contentAlignment = Alignment.Center,
                modifier = Modifier.size(40.dp).clip(CircleShape).background(colors.primaryContainer),
            ) {
                Icon(
                    painterResource(Widgets.osIcon(pc)),
                    contentDescription = null,
                    tint = colors.onPrimaryContainer,
                    modifier = Modifier.size(20.dp),
                )
            }
            Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
                Text(
                    pc.name.ifBlank { stringResource(R.string.your_pc) },
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(
                    stringResource(if (pc.online) R.string.widget_pc_connected else R.string.widget_pc_offline),
                    style = MaterialTheme.typography.bodySmall,
                    color = if (pc.online) colors.primary else colors.onSurfaceVariant,
                )
            }
            if (chevron != null) {
                Icon(
                    painterResource(R.drawable.ic_widget_chevron),
                    contentDescription = stringResource(R.string.widget_choose_pc),
                    tint = colors.onSurfaceVariant,
                    modifier = Modifier.size(22.dp).rotate(if (chevron) 180f else 0f),
                )
            }
        }
    }

    @Composable
    private fun PowerRow(label: String, onClick: () -> Unit) {
        Box(
            contentAlignment = Alignment.CenterStart,
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 52.dp)
                .clip(RoundedCornerShape(18.dp))
                .background(MaterialTheme.colorScheme.surfaceContainer)
                .clickable(role = Role.Button, onClick = onClick)
                .padding(horizontal = 18.dp),
        ) {
            Text(label, style = MaterialTheme.typography.bodyLarge)
        }
    }

    companion object {
        private const val ACTION_POWER = "app.nectarlink.widget.POWER_MENU"
        private const val ACTION_PICK = "app.nectarlink.widget.PICK_PC"
        private const val EXTRA_PC_ID = "pc"
        private const val EXTRA_WIDGET_ID = "widget"

        fun powerIntent(context: Context, pcId: String): Intent = Intent(context, WidgetMenuActivity::class.java)
            .setAction(ACTION_POWER)
            .putExtra(EXTRA_PC_ID, pcId)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)

        fun pickIntent(context: Context, widgetId: Int): Intent = Intent(context, WidgetMenuActivity::class.java)
            .setAction(ACTION_PICK)
            .putExtra(EXTRA_WIDGET_ID, widgetId)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
    }
}
