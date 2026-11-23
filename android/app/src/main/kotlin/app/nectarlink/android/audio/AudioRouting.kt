// SPDX-License-Identifier: GPL-3.0-or-later
package app.nectarlink.android.audio

import android.content.Context
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

/** Process-wide mixer state. The protocol adapter lives in Core and is supplied through [Hooks]. */
object AudioRouting {
    sealed interface Output {
        data object Phone : Output
        data class Pc(val deviceId: String) : Output
    }

    enum class MediaAction { PREVIOUS, PLAY_PAUSE, NEXT }
    enum class MicInputType { AUTO, BUILTIN, WIRED_USB, BLUETOOTH }

    data class MicInput(val type: MicInputType = MicInputType.AUTO, val deviceId: Int? = null)
    data class MicInputDevice(val input: MicInput, val name: String)
    data class OutputDevice(val id: String, val name: String)
    data class MediaStatus(
        val title: String = "",
        val artist: String = "",
        val playing: Boolean = false,
    )

    data class Peer(
        val deviceId: String,
        val name: String,
        val online: Boolean = true,
        val canSource: Boolean = true,
        val canSink: Boolean = false,
        val canUseMic: Boolean = false,
        val canSilenceSpeaker: Boolean = false,
        val outputs: List<OutputDevice> = emptyList(),
        val outputId: String? = null,
        val rttMs: Float? = null,
        val media: MediaStatus? = null,
    )

    /** The slider position is stored so the panel can round-trip the quadratic curve. */
    data class Lane(
        val slider: Float = 1f,
        val muted: Boolean = false,
        val alias: String = "",
        val silenceLocalSpeaker: Boolean = true,
        val peak: Float = 0f,
        val active: Boolean = false,
    ) {
        val gain: Float get() = LiveAudioMixer.sliderGain(slider)
    }

    data class VirtualMicStatus(
        val ready: Boolean,
        val active: Boolean = false,
        val isDefaultInput: Boolean = false,
        val deviceName: String? = null,
        val setupReason: String? = null,
        val canOpenSetup: Boolean = false,
    )

    data class Mic(
        val target: String? = null,
        val streamingTo: String? = null,
        val input: MicInput = MicInput(),
        val availableInputs: List<MicInputDevice> = emptyList(),
        val gain: Float = 1f,
        val muted: Boolean = false,
        val peak: Float = 0f,
        val talkingWhileMuted: Boolean = false,
        val statuses: Map<String, VirtualMicStatus> = emptyMap(),
    )

    data class State(
        val enabled: Boolean = false,
        val listening: Boolean = false,
        val output: Output = Output.Phone,
        val outputDeviceId: String? = null,
        val generation: Long = 0L,
        val peers: List<Peer> = emptyList(),
        val lanes: Map<String, Lane> = emptyMap(),
        val outputLatencyMs: Float? = null,
        val masterGain: Float = 1f,
        val masterMuted: Boolean = false,
        val includePhone: Boolean = false,
        val phoneLane: Lane = Lane(silenceLocalSpeaker = false),
        val policy: PlaybackPolicy = PlaybackPolicy.PAUSE,
        val selectiveResume: Boolean = true,
        val phonePlaybackActive: Boolean = false,
        val latencyPreset: LiveAudioMixer.LatencyPreset = LiveAudioMixer.LatencyPreset.BALANCED,
        val mic: Mic = Mic(),
    ) {
        fun nameOf(pcId: String): String =
            lanes[pcId]?.alias?.takeIf { it.isNotBlank() }
                ?: peers.firstOrNull { it.deviceId == pcId }?.name.orEmpty()

        fun outputPeer(): Peer? = (output as? Output.Pc)?.let { pc ->
            peers.firstOrNull { it.deviceId == pc.deviceId }
        }

        /** Linear master gain for `audio.route.set` (the slider squared, like lanes). */
        val masterLinearGain: Float get() = LiveAudioMixer.sliderGain(masterGain)

        /** Sources whose audio this route mixes. */
        fun enrolledSources(): Set<String> =
            peers.filter { it.online && it.canSource }.mapTo(mutableSetOf()) { it.deviceId }
    }

    data class Hooks(
        val onRouteChanged: (State) -> Unit = {},
        val sendRoutePcm: (Long, String, UInt, Int, Int, ByteArray) -> Boolean = { _, _, _, _, _, _ -> false },
        val onMicTargetChanged: (String?) -> Unit = {},
        val requestMicStatus: (String) -> Unit = {},
        val micSetup: (String) -> Unit = {},
        val mediaControl: (String, MediaAction) -> Unit = { _, _ -> },
    )

    @Volatile var hooks = Hooks()

    private val lock = Any()
    // Lazy: only bindPlayer needs the main thread, so routing state also works off a device.
    private val scope by lazy { CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate) }
    private val current = MutableStateFlow(State())
    val state: StateFlow<State> = current.asStateFlow()
    private var player: PcSpeakerPlayer? = null
    private var phoneSequence = 0u
    /** The generation before the last bump, still accepted from sources that stayed enrolled. */
    private var previousGeneration: Long? = null
    private var previousUntilMs = 0L

    /** Back to defaults, for tests. */
    internal fun reset() {
        synchronized(lock) {
            current.value = State()
            phoneSequence = 0u
            previousGeneration = null
            previousUntilMs = 0L
        }
        hooks = Hooks()
    }

    fun bindPlayer(context: Context, speakerPlayer: PcSpeakerPlayer) {
        val playback = PhonePlaybackPolicy.forContext(context)
        synchronized(lock) {
            check(player == null || player === speakerPlayer)
            if (player === speakerPlayer) return
            player = speakerPlayer
            speakerPlayer.attach(context, playback)
            applyToPlayer(current.value)
        }
        if (playback != null) scope.launch {
            playback.phonePlaybackActive.collect { active ->
                changeRoute { it.copy(phonePlaybackActive = active) }
            }
        }
        scope.launch {
            combine(speakerPlayer.laneStats, speakerPlayer.outputLatencyMs, ::Pair).collect { (stats, latency) ->
                update { state ->
                    if (state.output != Output.Phone || !state.listening) return@update state
                    val lanes = state.lanes.mapValues { (id, lane) ->
                        lane.copy(peak = stats[id]?.peak ?: 0f, active = stats[id]?.live == true)
                    }
                    state.copy(lanes = lanes, outputLatencyMs = latency)
                }
            }
        }
    }

    fun setEnabled(enabled: Boolean) = changeRoute { state ->
        if (enabled == state.enabled) state else state.copy(enabled = enabled, listening = enabled)
    }

    fun setListening(listening: Boolean) = changeRoute { state ->
        state.copy(enabled = state.enabled || listening, listening = listening)
    }

    fun selectOutput(output: Output) = changeRoute { state ->
        val pc = output as? Output.Pc
        if (output == state.output || (pc != null && state.peers.none {
                it.deviceId == pc.deviceId && it.online && it.canSink
            })
        ) state else state.copy(output = output, outputDeviceId = null)
    }

    fun selectOutputDevice(outputId: String?) = changeRoute { state ->
        val peer = state.outputPeer() ?: return@changeRoute state
        if (outputId == null || peer.outputs.any { it.id == outputId }) {
            state.copy(outputDeviceId = outputId)
        } else state
    }

    fun updatePeers(peers: List<Peer>) {
        val unique = peers.distinctBy { it.deviceId }
        val change = synchronized(lock) {
            val before = current.value
            val lanes = before.lanes.toMutableMap()
            val online = State(peers = unique).enrolledSources()
            for (peer in unique) lanes.putIfAbsent(peer.deviceId, Lane())
            val output = before.outputPeer()?.deviceId
            val sinkOnline = output == null || unique.any { it.deviceId == output && it.online && it.canSink }
            val outputStillPresent = before.outputDeviceId == null || unique
                .firstOrNull { it.deviceId == output }?.outputs?.any { it.id == before.outputDeviceId } == true
            val next = before.copy(
                peers = unique,
                lanes = lanes.mapValues { (id, lane) ->
                    if (id in online) lane else lane.copy(peak = 0f, active = false)
                },
                output = if (sinkOnline) before.output else Output.Phone,
                outputDeviceId = if (sinkOnline && outputStillPresent) before.outputDeviceId else null,
            )
            val topologyChanged = before.enrolledSources() != online ||
                before.output != next.output || before.outputDeviceId != next.outputDeviceId
            val after = publish(before, next, topologyChanged)
            if (topologyChanged && after.enabled) after else null
        }
        change?.let { hooks.onRouteChanged(it) }
    }

    fun setMasterGain(gain: Float) = changeRoute { it.copy(masterGain = gain.clamp(1f)) }
    fun setMasterMuted(muted: Boolean) = changeRoute { it.copy(masterMuted = muted) }
    fun setLaneGain(pcId: String, slider: Float) = changeLane(pcId) { it.copy(slider = slider.clamp(1f)) }
    fun setLaneMuted(pcId: String, muted: Boolean) = changeLane(pcId) { it.copy(muted = muted) }
    fun setLaneAlias(pcId: String, alias: String) = update {
        it.copy(lanes = it.lanes + (pcId to (it.lanes[pcId] ?: Lane()).copy(alias = alias.trim())))
    }
    fun setSilenceLocalSpeaker(pcId: String, silence: Boolean) =
        changeLane(pcId) { it.copy(silenceLocalSpeaker = silence) }

    fun stepLaneGain(pcId: String, direction: Int) = changeLane(pcId) {
        it.copy(slider = (it.slider + direction * 0.05f).clamp(1f))
    }

    fun setIncludePhone(include: Boolean) = changeRoute { it.copy(includePhone = include) }
    fun setPhoneGain(slider: Float) = changeRoute { it.copy(phoneLane = it.phoneLane.copy(slider = slider.clamp(1f))) }
    fun setPhoneMuted(muted: Boolean) = changeRoute { it.copy(phoneLane = it.phoneLane.copy(muted = muted)) }
    fun setPolicy(policy: PlaybackPolicy) = changeRoute { it.copy(policy = policy) }
    fun setSelectiveResume(enabled: Boolean) = changeRoute { it.copy(selectiveResume = enabled) }
    fun setLatencyPreset(preset: LiveAudioMixer.LatencyPreset) = changeRoute { it.copy(latencyPreset = preset) }

    /**
     * A managed PCM frame for phone playback. Frames for the current generation from an
     * enrolled source play. So do frames for the generation just before it, for
     * [GENERATION_GRACE_MS], so a PC that stays in the route keeps playing while it
     * learns the new generation.
     */
    fun receivePcm(generation: Long, sourceId: String, seq: UInt, sampleRate: Int, channels: Int, pcm: ByteArray) {
        synchronized(lock) {
            val state = current.value
            if (!state.listening || state.output != Output.Phone) return
            val accepted = generation == state.generation ||
                (generation == previousGeneration && monotonicMs() < previousUntilMs)
            if (!accepted || state.peers.none { it.deviceId == sourceId && it.online && it.canSource }) return
            player?.playFrame(sampleRate, channels, pcm, sourceId, seq)
        }
    }

    /** The PC sink applies [State.phoneLane] gain and mute. */
    fun sendPhonePcm(sampleRate: Int, channels: Int, pcm: ByteArray): Boolean {
        val outgoing = synchronized(lock) {
            val state = current.value
            val output = state.output as? Output.Pc ?: return false
            if (!state.listening || !state.includePhone) return false
            Triple(state.generation, output.deviceId, phoneSequence++)
        }
        return hooks.sendRoutePcm(outgoing.first, outgoing.second, outgoing.third, sampleRate, channels, pcm)
    }

    fun updateRemoteLevel(generation: Long, sourceId: String, peak: Float, active: Boolean = true) = update { state ->
        if (generation != state.generation || state.output !is Output.Pc || !state.listening) return@update state
        val lane = state.lanes[sourceId] ?: return@update state
        state.copy(lanes = state.lanes + (sourceId to lane.copy(peak = peak.clamp(1f), active = active)))
    }

    fun updatePhonePeak(peak: Float) = update { it.copy(phoneLane = it.phoneLane.copy(peak = peak.clamp(1f))) }

    fun selectMicTarget(pcId: String?) {
        val changed = synchronized(lock) {
            val value = current.value
            if (value.mic.target == pcId) false else {
                current.value = value.copy(mic = value.mic.copy(target = pcId))
                true
            }
        }
        if (changed) {
            hooks.onMicTargetChanged(pcId)
            if (pcId != null) hooks.requestMicStatus(pcId)
        }
    }

    fun setMicInput(input: MicInput) = updateMic { it.copy(input = input) }
    fun setMicGain(gain: Float) = updateMic { it.copy(gain = gain.clamp(2f)) }
    fun setMicMuted(muted: Boolean) = updateMic { it.copy(muted = muted, talkingWhileMuted = muted && it.peak > TALK_THRESHOLD) }
    fun updateMicInputs(inputs: List<MicInputDevice>) = updateMic { it.copy(availableInputs = inputs) }
    fun updateMicStreaming(pcId: String?) = updateMic {
        it.copy(streamingTo = pcId, peak = 0f, talkingWhileMuted = false)
    }
    fun updateMicPeak(peak: Float) = updateMic {
        it.copy(peak = peak.clamp(1f), talkingWhileMuted = it.muted && peak > TALK_THRESHOLD)
    }
    fun updateMicStatus(pcId: String, status: VirtualMicStatus?) = updateMic {
        it.copy(statuses = if (status == null) it.statuses - pcId else it.statuses + (pcId to status))
    }

    fun requestMicStatus(pcId: String) = hooks.requestMicStatus(pcId)
    fun micSetup(pcId: String) = hooks.micSetup(pcId)
    fun mediaControl(pcId: String, action: MediaAction) = hooks.mediaControl(pcId, action)

    private fun changeLane(pcId: String, transform: (Lane) -> Lane) = changeRoute {
        it.copy(lanes = it.lanes + (pcId to transform(it.lanes[pcId] ?: Lane())))
    }

    private fun updateMic(transform: (Mic) -> Mic) = update { it.copy(mic = transform(it.mic)) }

    private fun update(transform: (State) -> State) {
        synchronized(lock) { current.value = transform(current.value) }
    }

    private fun changeRoute(transform: (State) -> State) {
        val change = synchronized(lock) {
            val before = current.value
            val next = transform(before)
            val topologyChanged = before.listening != next.listening || before.output != next.output ||
                before.outputDeviceId != next.outputDeviceId || before.includePhone != next.includePhone
            val after = publish(before, next, topologyChanged)
            if (before != after && (before.enabled || after.enabled)) after else null
        }
        change?.let { hooks.onRouteChanged(it) }
    }

    private fun publish(before: State, next: State, topologyChanged: Boolean): State {
        if (before == next) return before
        val after = if (topologyChanged) {
            phoneSequence = 0u
            val wasPlaying = before.listening && before.output == Output.Phone
            val keepsPlaying = next.listening && next.output == Output.Phone
            if (wasPlaying && keepsPlaying) {
                // Only the sources that left stop; the rest play through the bump.
                for (id in before.enrolledSources() - next.enrolledSources()) player?.stopForPc(id)
                previousGeneration = before.generation
                previousUntilMs = monotonicMs() + GENERATION_GRACE_MS
            } else {
                if (wasPlaying) player?.stop()
                previousGeneration = null
            }
            next.copy(
                generation = before.generation + 1,
                lanes = next.lanes.mapValues { (_, lane) -> lane.copy(peak = 0f, active = false) },
                phoneLane = next.phoneLane.copy(peak = 0f),
                outputLatencyMs = null,
            )
        } else next
        applyToPlayer(after)
        current.value = after
        return after
    }

    private fun applyToPlayer(state: State) {
        val mixer = player?.mixer ?: return
        mixer.latencyPreset = state.latencyPreset
        mixer.masterGain = if (state.enabled) state.masterLinearGain else 1f
        mixer.masterMuted = state.enabled && state.masterMuted
        mixer.policyGain = if (state.enabled) state.policy.gain(state.phonePlaybackActive) else 1f
        for ((id, lane) in state.lanes) mixer.setLaneGain(id, lane.slider, state.enabled && lane.muted)
    }

    private fun Float.clamp(max: Float) = if (isFinite()) coerceIn(0f, max) else 0f

    private fun monotonicMs() = System.nanoTime() / 1_000_000

    /** Pre-mute peak (about -30 dBFS) above which a muted mic counts as talking. */
    const val TALK_THRESHOLD = 0.03f
    private const val GENERATION_GRACE_MS = 2_000L
}
