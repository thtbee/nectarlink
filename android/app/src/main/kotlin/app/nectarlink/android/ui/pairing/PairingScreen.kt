// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.ui.pairing

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import app.nectarlink.android.R
import app.nectarlink.android.core.CoreState
import app.nectarlink.android.core.PairingState
import app.nectarlink.android.ui.theme.LocalAppFonts
import app.nectarlink.android.ui.theme.LocalReducedMotion
import app.nectarlink.core.DiscoveredDevice
import app.nectarlink.core.PairingFailure
import kotlinx.coroutines.delay

/** What the user does on the pairing screen. */
interface PairingActions {
    /** The screen is shown or hidden (discovery needs a multicast lock). */
    fun visible(open: Boolean)
    fun join(link: String)
    fun pairNearby(id: String)
    fun confirm(matches: Boolean)
    fun reset()
}

/**
 * Pairing with a PC: scan the QR code it shows, or pick it from the PCs
 * nearby and compare a 6-digit code. `onDone` runs after success (or when
 * the user backs out, if `cancellable`).
 */
@Composable
fun PairingScreen(
    state: CoreState,
    actions: PairingActions,
    cancellable: Boolean,
    onDone: () -> Unit,
) {
    var nearby by rememberSaveable { mutableStateOf(false) }
    val pairing = state.pairing
    val reducedMotion = LocalReducedMotion.current
    DisposableEffect(Unit) {
        actions.visible(true)
        onDispose { actions.visible(false) }
    }

    LaunchedEffect(pairing) {
        if (pairing is PairingState.Paired) {
            delay(1_400)
            actions.reset()
            onDone()
        }
    }

    Column(
        Modifier.fillMaxSize().safeDrawingPadding().padding(horizontal = 24.dp, vertical = 16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Spacer(Modifier.height(24.dp))
        Text(
            text = when (pairing) {
                is PairingState.Comparing, is PairingState.Confirmed -> stringResource(R.string.pair_check_code)
                is PairingState.Paired -> stringResource(R.string.pair_done_title)
                is PairingState.Failed -> stringResource(R.string.pair_failed_title)
                else -> if (nearby) stringResource(R.string.pair_nearby_title) else stringResource(R.string.pair_title)
            },
            style = MaterialTheme.typography.headlineMedium,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(20.dp))

        AnimatedContent(
            targetState = pairing to nearby,
            transitionSpec = {
                if (reducedMotion) {
                    EnterTransition.None togetherWith ExitTransition.None
                } else {
                    fadeIn() togetherWith fadeOut()
                }
            },
            modifier = Modifier.weight(1f),
            label = "pairing",
        ) { (current, showNearby) ->
            when (current) {
                PairingState.Idle -> if (showNearby) {
                    NearbyList(state.discovered, actions::pairNearby)
                } else {
                    Scan(actions::join)
                }
                PairingState.Joining -> Working(stringResource(R.string.pair_joining))
                is PairingState.Connecting ->
                    Working(stringResource(R.string.pair_connecting, state.nameOf(current.peer) ?: ""))
                is PairingState.Comparing -> Compare(state.nameOf(current.peer), current.code, actions::confirm)
                is PairingState.Confirmed -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    CodeDigits(current.code)
                    Spacer(Modifier.height(24.dp))
                    Working(stringResource(R.string.pair_waiting, state.nameOf(current.peer) ?: ""))
                }
                is PairingState.Paired -> Outcome(true, stringResource(R.string.pair_done, current.name))
                is PairingState.Failed -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
                    Outcome(false, failureText(current.failure))
                    Spacer(Modifier.height(16.dp))
                    Button(onClick = actions::reset) { Text(stringResource(R.string.action_try_again)) }
                }
            }
        }

        if (pairing == PairingState.Idle) {
            FilledTonalButton(onClick = { nearby = !nearby }) {
                Text(stringResource(if (nearby) R.string.action_scan_instead else R.string.action_pair_nearby))
            }
        }
        if (cancellable && pairing !is PairingState.Paired) {
            TextButton(onClick = { actions.reset(); onDone() }) { Text(stringResource(R.string.action_cancel)) }
        }
    }
}

@Composable
private fun Scan(onLink: (String) -> Unit) {
    Column(
        modifier = Modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(
            stringResource(R.string.pair_scan_hint),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(16.dp))
        QrScanner(
            onLink = onLink,
            modifier = Modifier
                .weight(1f, fill = false)
                .fillMaxWidth()
                .aspectRatio(1f)
                .clip(MaterialTheme.shapes.extraLarge)
                .background(MaterialTheme.colorScheme.surfaceContainerHigh),
        )
    }
}

@Composable
private fun NearbyList(devices: List<DiscoveredDevice>, onPair: (String) -> Unit) {
    val reducedMotion = LocalReducedMotion.current
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Text(
            stringResource(R.string.pair_nearby_hint),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.fillMaxWidth(),
        )
        Spacer(Modifier.height(6.dp))
        if (devices.isEmpty()) {
            Surface(
                shape = MaterialTheme.shapes.large,
                color = MaterialTheme.colorScheme.surfaceContainer,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Row(
                    Modifier.fillMaxWidth().padding(20.dp),
                    horizontalArrangement = Arrangement.Center,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    if (reducedMotion) {
                        CircularProgressIndicator(
                            progress = { 0.75f },
                            modifier = Modifier.size(18.dp),
                            strokeWidth = 2.dp,
                        )
                    } else {
                        CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp)
                    }
                    Spacer(Modifier.width(12.dp))
                    Text(
                        stringResource(R.string.pair_looking),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
        devices.forEach { device ->
            Surface(
                shape = MaterialTheme.shapes.large,
                color = MaterialTheme.colorScheme.surfaceContainerHigh,
                modifier = Modifier.fillMaxWidth(),
            ) {
                Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        device.name ?: stringResource(R.string.your_pc),
                        style = MaterialTheme.typography.titleMedium,
                        modifier = Modifier.weight(1f),
                    )
                    Button(onClick = { onPair(device.id) }) { Text(stringResource(R.string.action_pair)) }
                }
            }
        }
    }
}

@Composable
private fun Compare(peerName: String?, code: String, onConfirm: (Boolean) -> Unit) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Text(
            stringResource(R.string.pair_compare_hint, peerName ?: stringResource(R.string.your_pc)),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(24.dp))
        CodeDigits(code)
        Spacer(Modifier.height(32.dp))
        Button(onClick = { onConfirm(true) }, modifier = Modifier.fillMaxWidth().height(52.dp)) {
            Text(stringResource(R.string.action_codes_match))
        }
        Spacer(Modifier.height(8.dp))
        OutlinedButton(onClick = { onConfirm(false) }, modifier = Modifier.fillMaxWidth().height(52.dp)) {
            Text(stringResource(R.string.action_codes_differ))
        }
    }
}

@Composable
private fun CodeDigits(code: String) {
    Row(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        modifier = Modifier.semantics { contentDescription = code.toList().joinToString(" ") },
    ) {
        code.forEach { digit ->
            Surface(
                shape = MaterialTheme.shapes.medium,
                color = MaterialTheme.colorScheme.surfaceContainerHigh,
                modifier = Modifier.size(width = 44.dp, height = 56.dp),
            ) {
                Box(contentAlignment = Alignment.Center) {
                    Text(digit.toString(), fontFamily = LocalAppFonts.current.mono, fontSize = 26.sp, fontWeight = FontWeight.Bold)
                }
            }
        }
    }
}

@Composable
private fun Working(text: String) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        if (LocalReducedMotion.current) {
            CircularProgressIndicator(
                progress = { 0.75f },
                modifier = Modifier.size(20.dp),
                strokeWidth = 2.dp,
            )
        } else {
            CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp)
        }
        Spacer(Modifier.width(12.dp))
        Text(text, style = MaterialTheme.typography.bodyLarge)
    }
}

@Composable
private fun Outcome(success: Boolean, text: String) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Surface(
            shape = CircleShape,
            color = if (success) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh,
            modifier = Modifier.size(64.dp),
        ) {
            Box(contentAlignment = Alignment.Center) {
                Text(
                    if (success) "✓" else "!",
                    fontSize = 28.sp,
                    fontWeight = FontWeight.Bold,
                    color = if (success) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurface,
                )
            }
        }
        Spacer(Modifier.height(16.dp))
        Text(text, style = MaterialTheme.typography.bodyLarge, textAlign = TextAlign.Center)
    }
}

@Composable
private fun failureText(failure: PairingFailure): String = when (failure) {
    PairingFailure.Rejected -> stringResource(R.string.pair_failed_rejected)
    PairingFailure.Declined -> stringResource(R.string.pair_failed_declined)
    PairingFailure.Expired -> stringResource(R.string.pair_failed_expired)
    PairingFailure.Unreachable -> stringResource(R.string.pair_failed_unreachable)
    is PairingFailure.Other -> stringResource(R.string.pair_failed_other)
}
