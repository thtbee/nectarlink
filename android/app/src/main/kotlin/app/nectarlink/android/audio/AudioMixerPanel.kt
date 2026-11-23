// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.audio

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Card
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import app.nectarlink.android.R
import app.nectarlink.android.mic.MicService
import kotlin.math.roundToInt

/** The audio controls shared by the app and the Quick Settings volume panel. */
@Composable
fun AudioMixerPanel(
    modifier: Modifier = Modifier,
    onExpand: (() -> Unit)? = null,
    onLaneSelected: ((String) -> Unit)? = null,
) {
    val state by AudioRouting.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val sources = state.peers.filter { it.canSource }
    val sinks = state.peers.filter { it.online && it.canSink }
    val sink = state.outputPeer()
    val phoneLabel = stringResource(R.string.audio_mixer_phone)
    val offLabel = stringResource(R.string.audio_mixer_off)
    val outputChoices = listOf(AudioRouting.Output.Phone to phoneLabel) +
        sinks.map { AudioRouting.Output.Pc(it.deviceId) to state.nameOf(it.deviceId) }
    Column(modifier, verticalArrangement = Arrangement.spacedBy(12.dp)) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text(stringResource(R.string.audio_mixer_title), Modifier.weight(1f), style = MaterialTheme.typography.titleLarge)
            if (onExpand != null) TextButton(onClick = onExpand) { Text(stringResource(R.string.audio_mixer_expand)) }
        }
        MixerToggle(stringResource(R.string.audio_mixer_listening), state.listening) {
            if (it) AudioListeningService.start(context) else AudioListeningService.stop(context)
        }
        Choice(
            label = stringResource(R.string.audio_mixer_output),
            selected = state.output,
            choices = outputChoices,
            onSelect = AudioRouting::selectOutput,
        )
        if (sink != null) {
            val devices: List<Pair<String?, String>> = listOf(null to stringResource(R.string.audio_mixer_system_default)) +
                sink.outputs.map { it.id to it.name }
            if (devices.size > 1) {
                Choice(stringResource(R.string.audio_mixer_output_device), state.outputDeviceId, devices, AudioRouting::selectOutputDevice)
            }
        } else {
            state.outputLatencyMs?.let {
                Text(stringResource(R.string.audio_mixer_output_latency, it.roundToInt()), style = MaterialTheme.typography.bodySmall)
            }
        }
        MixerGain(stringResource(R.string.audio_mixer_master_volume), state.masterGain, AudioRouting::setMasterGain)
        MixerToggle(stringResource(R.string.audio_mixer_mute_output), state.masterMuted, AudioRouting::setMasterMuted)

        for (peer in sources) key(peer.deviceId) {
            val id = peer.deviceId
            val lane = state.lanes[id] ?: AudioRouting.Lane()
            MixerCard {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    val laneSelect = if (onLaneSelected == null || !peer.online) Modifier else {
                        Modifier.clickable { onLaneSelected(id) }
                    }
                    Text(state.nameOf(id), Modifier.weight(1f).then(laneSelect), style = MaterialTheme.typography.titleMedium)
                    val latency = peer.rttMs?.takeIf { it >= 0f && it.isFinite() }
                    val badge = when {
                        !peer.online -> stringResource(R.string.audio_mixer_offline)
                        latency != null -> stringResource(R.string.audio_mixer_rtt, latency.roundToInt())
                        else -> null
                    }
                    if (badge != null) Text(badge, style = MaterialTheme.typography.labelMedium)
                }
                Meter(lane.peak, stringResource(R.string.audio_mixer_pc_level))
                MixerGain(stringResource(R.string.audio_mixer_pc_volume), lane.slider, { AudioRouting.setLaneGain(id, it) })
                MixerToggle(stringResource(R.string.audio_mixer_mute_pc), lane.muted) { AudioRouting.setLaneMuted(id, it) }
                if (peer.canSilenceSpeaker) {
                    MixerToggle(stringResource(R.string.audio_mixer_silence_pc), lane.silenceLocalSpeaker) {
                        AudioRouting.setSilenceLocalSpeaker(id, it)
                    }
                }
                peer.media?.let { media ->
                    if (media.title.isNotBlank()) Text(media.title, style = MaterialTheme.typography.bodyMedium)
                    if (media.artist.isNotBlank()) Text(media.artist, style = MaterialTheme.typography.bodySmall)
                    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                        MediaButton(id, AudioRouting.MediaAction.PREVIOUS, stringResource(R.string.audio_mixer_previous), peer.online)
                        MediaButton(
                            id, AudioRouting.MediaAction.PLAY_PAUSE,
                            stringResource(if (media.playing) R.string.audio_mixer_pause else R.string.audio_mixer_play),
                            peer.online,
                        )
                        MediaButton(id, AudioRouting.MediaAction.NEXT, stringResource(R.string.audio_mixer_next), peer.online)
                    }
                }
            }
        }

        if (sink != null) {
            MixerCard {
                Text(stringResource(R.string.audio_mixer_phone_sound), style = MaterialTheme.typography.titleMedium)
                MixerToggle(stringResource(R.string.audio_mixer_include_phone), state.includePhone, AudioRouting::setIncludePhone)
                Meter(state.phoneLane.peak, stringResource(R.string.audio_mixer_phone_level))
                MixerGain(stringResource(R.string.audio_mixer_phone_volume), state.phoneLane.slider, AudioRouting::setPhoneGain)
                MixerToggle(stringResource(R.string.audio_mixer_mute_phone), state.phoneLane.muted, AudioRouting::setPhoneMuted)
            }
        }

        PhoneMicCard(state, offLabel)
        Text(stringResource(R.string.audio_mixer_playback_policy), style = MaterialTheme.typography.titleMedium)
        Segments(
            state.policy, PlaybackPolicy.entries,
            listOf(
                stringResource(R.string.audio_mixer_pause), stringResource(R.string.audio_mixer_mute),
                stringResource(R.string.audio_mixer_lower), offLabel,
            ),
            AudioRouting::setPolicy,
        )
        MixerToggle(stringResource(R.string.audio_mixer_selective_resume), state.selectiveResume, AudioRouting::setSelectiveResume)
        Text(stringResource(R.string.audio_mixer_latency), style = MaterialTheme.typography.titleMedium)
        Segments(
            state.latencyPreset, LiveAudioMixer.LatencyPreset.entries,
            listOf(
                stringResource(R.string.audio_mixer_lowest), stringResource(R.string.audio_mixer_balanced),
                stringResource(R.string.audio_mixer_stable),
            ),
            AudioRouting::setLatencyPreset,
        )
    }
}

@Composable
private fun PhoneMicCard(state: AudioRouting.State, offLabel: String) {
    val context = LocalContext.current
    val mic = state.mic
    var pendingTarget by rememberSaveable { mutableStateOf<String?>(null) }
    val requestMic = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) pendingTarget?.let(AudioRouting::selectMicTarget)
        pendingTarget = null
    }
    var pendingBluetoothId by rememberSaveable { mutableStateOf<Int?>(null) }
    val requestBluetooth = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) AudioRouting.setMicInput(AudioRouting.MicInput(AudioRouting.MicInputType.BLUETOOTH, pendingBluetoothId))
        pendingBluetoothId = null
    }
    LifecycleResumeEffect(context) {
        val watcher = MicService.watchInputs(context, AudioRouting::updateMicInputs)
        onPauseOrDispose { watcher.close() }
    }

    MixerCard {
        Text(stringResource(R.string.audio_mixer_mic), style = MaterialTheme.typography.titleMedium)
        val target = mic.target ?: mic.streamingTo
        val targets = state.peers.filter { (it.online && it.canUseMic) || it.deviceId == target }
        val targetChoices: List<Pair<String?, String>> = listOf(null to offLabel) +
            targets.map { it.deviceId to state.nameOf(it.deviceId) }
        Choice(stringResource(R.string.audio_mixer_mic_target), target, targetChoices) { id ->
            if (id == null) {
                if (mic.target == null) MicService.stopFromUser(context) else AudioRouting.selectMicTarget(null)
            } else if (MicService.hasPermission(context)) {
                AudioRouting.selectMicTarget(id)
            } else {
                pendingTarget = id
                requestMic.launch(Manifest.permission.RECORD_AUDIO)
            }
        }

        val inputTypes = AudioRouting.MicInputType.entries.map { AudioRouting.MicInput(it) to inputLabel(it) }
        val inputChoices = inputTypes + mic.availableInputs.map { it.input to it.name }
        Choice(stringResource(R.string.audio_mixer_mic_input), mic.input, inputChoices) { input ->
            if (input.type == AudioRouting.MicInputType.BLUETOOTH && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S &&
                ContextCompat.checkSelfPermission(context, Manifest.permission.BLUETOOTH_CONNECT) != PackageManager.PERMISSION_GRANTED
            ) {
                pendingBluetoothId = input.deviceId
                requestBluetooth.launch(Manifest.permission.BLUETOOTH_CONNECT)
            } else {
                AudioRouting.setMicInput(input)
            }
        }
        Meter(mic.peak, stringResource(R.string.audio_mixer_mic_level))
        if (mic.talkingWhileMuted) {
            Text(stringResource(R.string.audio_mixer_talking_muted), color = MaterialTheme.colorScheme.error)
        }
        MixerGain(stringResource(R.string.audio_mixer_mic_gain), mic.gain, AudioRouting::setMicGain, max = 2f)
        MixerToggle(stringResource(R.string.audio_mixer_mic_mute), mic.muted, AudioRouting::setMicMuted)
        target?.let { id ->
            val status = mic.statuses[id]
            if (status != null) {
                val label = stringResource(if (status.ready) R.string.audio_mixer_mic_ready else R.string.audio_mixer_mic_not_ready)
                Text(label, style = MaterialTheme.typography.bodyMedium)
                status.deviceName?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                status.setupReason?.let { Text(it, style = MaterialTheme.typography.bodySmall) }
                if (status.canOpenSetup) {
                    TextButton(onClick = { AudioRouting.micSetup(id) }) { Text(stringResource(R.string.audio_mixer_mic_setup)) }
                }
            }
        }
    }
}

@Composable
private fun inputLabel(type: AudioRouting.MicInputType): String = when (type) {
    AudioRouting.MicInputType.AUTO -> stringResource(R.string.audio_mixer_automatic)
    AudioRouting.MicInputType.BUILTIN -> stringResource(R.string.audio_mixer_builtin_mic)
    AudioRouting.MicInputType.WIRED_USB -> stringResource(R.string.audio_mixer_wired_usb)
    AudioRouting.MicInputType.BLUETOOTH -> stringResource(R.string.audio_mixer_bluetooth_mic)
}

@Composable
private fun MixerCard(content: @Composable () -> Unit) {
    Card(Modifier.fillMaxWidth()) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            content()
        }
    }
}

@Composable
private fun MixerToggle(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f).padding(end = 12.dp), style = MaterialTheme.typography.bodyMedium)
        Switch(checked, onChange, Modifier.semantics { contentDescription = label })
    }
}

@Composable
private fun MixerGain(label: String, gain: Float, onChange: (Float) -> Unit, max: Float = 1f) {
    Column {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(label, style = MaterialTheme.typography.bodyMedium)
            Text(stringResource(R.string.audio_mixer_percent, (gain * 100).roundToInt()), style = MaterialTheme.typography.labelMedium)
        }
        Slider(
            gain,
            onChange,
            Modifier.fillMaxWidth().semantics { contentDescription = label },
            valueRange = 0f..max,
        )
    }
}

@Composable
private fun Meter(peak: Float, label: String) {
    LinearProgressIndicator(
        progress = { peak },
        modifier = Modifier.fillMaxWidth().semantics { contentDescription = label },
    )
}

@Composable
private fun MediaButton(id: String, action: AudioRouting.MediaAction, label: String, enabled: Boolean) {
    TextButton(onClick = { AudioRouting.mediaControl(id, action) }, enabled = enabled) { Text(label) }
}

@Composable
private fun <T> Choice(label: String, selected: T, choices: List<Pair<T, String>>, onSelect: (T) -> Unit) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Text(label, Modifier.weight(1f).padding(end = 12.dp), style = MaterialTheme.typography.bodyMedium)
        Box(Modifier.weight(1.4f)) {
            OutlinedButton(onClick = { expanded = true }, modifier = Modifier.fillMaxWidth()) {
                Text(choices.firstOrNull { it.first == selected }?.second ?: stringResource(R.string.your_pc))
            }
            DropdownMenu(expanded, onDismissRequest = { expanded = false }) {
                for ((value, name) in choices) {
                    DropdownMenuItem(text = { Text(name) }, onClick = {
                        expanded = false
                        onSelect(value)
                    })
                }
            }
        }
    }
}

@Composable
private fun <T> Segments(selected: T, values: List<T>, labels: List<String>, onSelect: (T) -> Unit) {
    SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
        values.forEachIndexed { index, value ->
            SegmentedButton(
                shape = SegmentedButtonDefaults.itemShape(index, values.size),
                selected = value == selected,
                onClick = { onSelect(value) },
                icon = {},
                label = { Text(labels[index]) },
            )
        }
    }
}
