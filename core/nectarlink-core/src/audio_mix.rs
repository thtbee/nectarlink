// SPDX-License-Identifier: MPL-2.0
//! Adaptive stereo PCM mixer for independently clocked live audio lanes.

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

const CAPACITY: usize = 65_536;
const FRESH_AUDIO: Duration = Duration::from_millis(100);
const NEW_BURST: Duration = Duration::from_secs(1);
const JITTER_RECOVERY: Duration = Duration::from_millis(80);
const BURST_WARMUP: Duration = Duration::from_millis(1500);
const MAX_SEQUENCE_STEP: u32 = 8;
const PCM_SCALE: f32 = 1.0 / 32768.0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum LatencyPreset {
    Lowest,
    #[default]
    Balanced,
    Stable,
}

impl LatencyPreset {
    pub fn margin_ms(self) -> f64 {
        match self {
            Self::Lowest => 4.0,
            Self::Balanced => 12.0,
            Self::Stable => 35.0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum SinkMode {
    #[default]
    Program,
    Microphone,
}

impl SinkMode {
    fn max_extra_ms(self) -> f64 {
        match self {
            Self::Program => 150.0,
            Self::Microphone => 40.0,
        }
    }
}

/// The effective policy. Use `Off` while the activity that triggers the policy is idle.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackPolicy {
    Pause,
    Mute,
    Lower,
    #[default]
    Off,
}

impl PlaybackPolicy {
    pub fn multiplier(self) -> f32 {
        match self {
            Self::Pause | Self::Mute => 0.0,
            Self::Lower => 0.15,
            Self::Off => 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u8,
}

/// Converts a slider position in 0..=1 to linear gain.
pub fn slider_gain(slider: f32) -> f32 {
    let slider = nonnegative_gain(slider).min(1.0);
    slider * slider
}

fn nonnegative_gain(gain: f32) -> f32 {
    if gain.is_finite() { gain.max(0.0) } else { 0.0 }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaneStatus {
    pub active: bool,
    pub playing: bool,
    pub peak: f32,
    pub buffered_ms: f64,
    pub extra_margin_ms: f64,
    pub rate_correction: f64,
}

/// The caller serializes packet delivery and rendering, and authorizes lanes using
/// authenticated stream metadata. A packet cannot create or reconfigure a lane.
#[derive(Debug)]
pub struct LiveAudioMixer<SourceId = u64> {
    lanes: BTreeMap<SourceId, Lane>,
    latency: LatencyPreset,
    mode: SinkMode,
    policy: PlaybackPolicy,
    master_gain: f32,
    master_muted: bool,
    mix: Vec<[f32; 2]>,
}

impl<SourceId: Ord> LiveAudioMixer<SourceId> {
    pub fn new(latency: LatencyPreset, mode: SinkMode) -> Self {
        Self {
            lanes: BTreeMap::new(),
            latency,
            mode,
            policy: PlaybackPolicy::Off,
            master_gain: 1.0,
            master_muted: false,
            mix: Vec::new(),
        }
    }

    /// Authorizes a lane or resets its stream when the generation or format changes.
    /// Gain, mute and active state survive a stream reset. Zero rate or channels is invalid.
    pub fn authorize_lane(&mut self, source_id: SourceId, generation: u64, format: AudioFormat) -> bool {
        if format.sample_rate == 0 || format.channels == 0 {
            return false;
        }
        let lane = self.lanes.entry(source_id).or_insert_with(|| Lane::new(generation, format));
        if (lane.generation, lane.format) != (generation, format) {
            lane.generation = generation;
            lane.format = format;
            lane.reset_stream();
        }
        true
    }

    pub fn remove_lane(&mut self, source_id: &SourceId) -> bool {
        self.lanes.remove(source_id).is_some()
    }

    pub fn set_latency(&mut self, latency: LatencyPreset) {
        self.latency = latency;
    }

    pub fn set_master_gain(&mut self, gain: f32) {
        self.master_gain = nonnegative_gain(gain);
    }

    pub fn set_master_muted(&mut self, muted: bool) {
        self.master_muted = muted;
    }

    pub fn set_policy(&mut self, policy: PlaybackPolicy) {
        self.policy = policy;
    }

    pub fn set_lane_gain(&mut self, source_id: &SourceId, gain: f32) {
        if let Some(lane) = self.lanes.get_mut(source_id) {
            lane.target_gain = nonnegative_gain(gain);
        }
    }

    pub fn set_lane_muted(&mut self, source_id: &SourceId, muted: bool) {
        if let Some(lane) = self.lanes.get_mut(source_id) {
            lane.muted = muted;
        }
    }

    pub fn set_lane_active(&mut self, source_id: &SourceId, active: bool) {
        if let Some(lane) = self.lanes.get_mut(source_id)
            && active != lane.active
        {
            lane.active = active;
            lane.reset_stream();
        }
    }

    pub fn lane_status(&self, source_id: &SourceId) -> Option<LaneStatus> {
        self.lanes.get(source_id).map(Lane::status)
    }

    /// Queues complete interleaved PCM frames. With no sequence, arrivals are played in order.
    /// Mono is duplicated and multichannel PCM uses its first two channels.
    pub fn push_pcm(
        &mut self,
        source_id: &SourceId,
        generation: u64,
        sequence: Option<u32>,
        pcm: &[i16],
        now: Instant,
    ) -> bool {
        let mode = self.mode;
        let Some(lane) = self.accepting_lane(source_id, generation) else { return false };
        let channels = usize::from(lane.format.channels);
        if pcm.is_empty() || !pcm.len().is_multiple_of(channels) {
            return false;
        }
        lane.push(
            sequence,
            pcm.chunks_exact(channels).map(|frame| {
                let left = frame[0];
                let right = frame[usize::from(channels > 1)];
                to_float_frame(left, right)
            }),
            now,
            mode,
        )
    }

    /// Queues interleaved signed 16-bit little-endian PCM without an intermediate allocation.
    pub fn push_pcm_le(
        &mut self,
        source_id: &SourceId,
        generation: u64,
        sequence: Option<u32>,
        pcm: &[u8],
        now: Instant,
    ) -> bool {
        let mode = self.mode;
        let Some(lane) = self.accepting_lane(source_id, generation) else { return false };
        let stride = usize::from(lane.format.channels) * 2;
        if pcm.is_empty() || !pcm.len().is_multiple_of(stride) {
            return false;
        }
        lane.push(
            sequence,
            pcm.chunks_exact(stride).map(|frame| {
                let left = i16::from_le_bytes([frame[0], frame[1]]);
                let right = if stride > 2 { i16::from_le_bytes([frame[2], frame[3]]) } else { left };
                to_float_frame(left, right)
            }),
            now,
            mode,
        )
    }

    fn accepting_lane(&mut self, source_id: &SourceId, generation: u64) -> Option<&mut Lane> {
        self.lanes.get_mut(source_id).filter(|lane| lane.active && lane.generation == generation)
    }

    /// Renders stereo PCM and reports whether any lane consumed audio, even with output muted.
    /// A zero output rate produces silence and leaves the lane clocks alone.
    pub fn render(&mut self, output_rate: u32, output: &mut [[i16; 2]], now: Instant) -> bool {
        if output_rate == 0 || output.is_empty() {
            output.fill([0; 2]);
            return false;
        }
        self.mix.resize(output.len(), [0.0; 2]);
        self.mix.fill([0.0; 2]);
        let mut any = false;
        for lane in self.lanes.values_mut() {
            any |= lane.mix(&mut self.mix, output_rate, self.latency.margin_ms(), now);
        }

        let gain = if self.master_muted { 0.0 } else { self.master_gain * self.policy.multiplier() };
        if gain == 0.0 {
            output.fill([0; 2]);
        } else {
            for (output, frame) in output.iter_mut().zip(&self.mix) {
                *output = frame.map(|sample| ((sample * gain).clamp(-1.0, 1.0) * 32767.0) as i16);
            }
        }
        any
    }
}

fn to_float_frame(left: i16, right: i16) -> [f32; 2] {
    [f32::from(left) * PCM_SCALE, f32::from(right) * PCM_SCALE]
}

#[derive(Debug)]
struct Lane {
    generation: u64,
    format: AudioFormat,
    active: bool,
    muted: bool,
    target_gain: f32,
    ring: Vec<[f32; 2]>,
    stream: Stream,
}

#[derive(Debug)]
struct Stream {
    write_pos: u64,
    read_pos: f64,
    playing: bool,
    gain: f32,
    peak: f32,
    extra_ms: f64,
    window_min: f64,
    previous_min: Option<f64>,
    last_min: Option<f64>,
    window_frames: u64,
    correction: f64,
    last_sequence: Option<u32>,
    last_audio: Option<Instant>,
    burst_start: Option<Instant>,
    starved_at: Option<Instant>,
}

impl Default for Stream {
    fn default() -> Self {
        Self {
            write_pos: 0,
            read_pos: 0.0,
            playing: false,
            gain: 0.0,
            peak: 0.0,
            extra_ms: 0.0,
            window_min: f64::INFINITY,
            previous_min: None,
            last_min: None,
            window_frames: 0,
            correction: 0.0,
            last_sequence: None,
            last_audio: None,
            burst_start: None,
            starved_at: None,
        }
    }
}

impl Lane {
    fn new(generation: u64, format: AudioFormat) -> Self {
        Self {
            generation,
            format,
            active: true,
            muted: false,
            target_gain: 1.0,
            ring: vec![[0.0; 2]; CAPACITY],
            stream: Stream::default(),
        }
    }

    fn reset_stream(&mut self) {
        self.stream = Stream::default();
    }

    fn status(&self) -> LaneStatus {
        let stream = &self.stream;
        LaneStatus {
            active: self.active,
            playing: stream.playing,
            peak: stream.peak,
            buffered_ms: (stream.write_pos as f64 - stream.read_pos) * 1000.0
                / f64::from(self.format.sample_rate),
            extra_margin_ms: stream.extra_ms,
            rate_correction: stream.correction,
        }
    }

    fn push(
        &mut self,
        sequence: Option<u32>,
        frames: impl ExactSizeIterator<Item = [f32; 2]>,
        now: Instant,
        mode: SinkMode,
    ) -> bool {
        let stream = &mut self.stream;
        let since_audio = stream.last_audio.map(|last| now.saturating_duration_since(last));
        let mut silence = 0;
        if let (Some(seq), Some(last)) = (sequence, stream.last_sequence) {
            let delta = seq.wrapping_sub(last);
            if delta == 0 || delta >= 1 << 31 {
                return false;
            }
            if (2..=MAX_SEQUENCE_STEP).contains(&delta) && since_audio.is_some_and(|age| age < FRESH_AUDIO) {
                silence = (delta as usize - 1) * frames.len().min(CAPACITY);
            }
        }

        if since_audio.is_none_or(|age| age > NEW_BURST) {
            stream.burst_start = Some(now);
            stream.read_pos = stream.write_pos as f64;
            stream.playing = false;
        }
        if stream.starved_at.take().is_some_and(|t| now.saturating_duration_since(t) < JITTER_RECOVERY)
            && stream.burst_start.is_some_and(|t| now.saturating_duration_since(t) > BURST_WARMUP)
        {
            stream.extra_ms = (stream.extra_ms + 10.0).min(mode.max_extra_ms());
        }

        let silence = silence.min(CAPACITY);
        for _ in 0..silence {
            self.ring[ring_index(stream.write_pos)] = [0.0; 2];
            stream.write_pos += 1;
        }
        let overwritten = frames.len().saturating_sub(CAPACITY);
        stream.write_pos += overwritten as u64;
        for frame in frames.skip(overwritten) {
            self.ring[ring_index(stream.write_pos)] = frame;
            stream.write_pos += 1;
        }
        let oldest = stream.write_pos.saturating_sub(CAPACITY as u64) as f64;
        if stream.read_pos < oldest {
            // The writer lapped us. Rebuffer instead of interpolating overwritten samples.
            stream.read_pos = oldest;
            stream.playing = false;
        }
        stream.last_sequence = sequence;
        stream.last_audio = Some(now);
        true
    }

    fn mix(&mut self, output: &mut [[f32; 2]], output_rate: u32, margin_ms: f64, now: Instant) -> bool {
        if !self.active {
            return false;
        }
        let stream = &mut self.stream;
        let rate = f64::from(self.format.sample_rate);
        let target = (margin_ms + stream.extra_ms) * rate / 1000.0;
        let write_pos = stream.write_pos as f64;
        let mut fill = write_pos - stream.read_pos;
        if !stream.playing {
            if !stream.last_audio.is_some_and(|last| now.saturating_duration_since(last) < FRESH_AUDIO)
                || fill < target
            {
                stream.peak *= 0.86;
                return false;
            }
            stream.playing = true;
            fill = fill.min(target + rate * 0.015);
            stream.read_pos = write_pos - fill;
            stream.gain = 0.0;
            stream.window_min = f64::INFINITY;
            stream.previous_min = None;
            stream.last_min = None;
            stream.window_frames = 0;
        }

        stream.window_min = stream.window_min.min(fill);
        stream.window_frames += output.len() as u64;
        let window_frames = u64::from(output_rate / 2).max(1);
        if stream.window_frames >= window_frames {
            stream.last_min = stream.previous_min.map(|previous| previous.min(stream.window_min));
            stream.previous_min = Some(stream.window_min);
            stream.window_min = f64::INFINITY;
            stream.window_frames %= window_frames;
            if let Some(minimum) = stream.last_min
                && minimum > target + rate * 0.030
            {
                let skip = minimum - target;
                stream.read_pos += skip;
                fill -= skip;
                stream.last_min = Some(target);
                stream.previous_min = Some(target);
            }
        }

        stream.correction =
            stream.last_min.map_or(0.0, |min| (0.3 * (min - target) / rate).clamp(-0.004, 0.008));
        let step = rate / f64::from(output_rate) * (1.0 + stream.correction);
        let underrun = fill < output.len() as f64 * step + 2.0;
        let count = if underrun { ((fill - 2.0) / step).max(0.0) as usize } else { output.len() };
        let gain_end = if underrun || self.muted { 0.0 } else { self.target_gain };
        let gain_step = (gain_end - stream.gain) / count.max(1) as f32;
        let mut peak: f32 = 0.0;
        for (i, out) in output[..count].iter_mut().enumerate() {
            let pos = stream.read_pos;
            let whole = pos as u64;
            let fraction = (pos - whole as f64) as f32;
            let before = self.ring[ring_index(whole)];
            let after = self.ring[ring_index(whole + 1)];
            let gain = stream.gain + gain_step * (i + 1) as f32;
            for channel in 0..2 {
                let sample = (before[channel] + (after[channel] - before[channel]) * fraction) * gain;
                out[channel] += sample;
                peak = peak.max(sample.abs());
            }
            stream.read_pos += step;
        }
        stream.gain = gain_end;
        stream.peak = peak.max(stream.peak * 0.86).min(1.0);
        if underrun {
            stream.playing = false;
            stream.starved_at = Some(now);
        } else {
            stream.extra_ms =
                (stream.extra_ms - 0.07 * output.len() as f64 / f64::from(output_rate)).max(0.0);
        }
        count > 0
    }
}

fn ring_index(position: u64) -> usize {
    position as usize & (CAPACITY - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONO: AudioFormat = AudioFormat { sample_rate: 48_000, channels: 1 };
    const FRAMES: usize = 240;
    const BLOCK: Duration = Duration::from_millis(5);

    fn mixer(mode: SinkMode) -> LiveAudioMixer {
        let mut mixer = LiveAudioMixer::new(LatencyPreset::Balanced, mode);
        assert!(mixer.authorize_lane(1, 1, MONO));
        mixer
    }

    #[test]
    fn independent_pc_clocks_converge_without_underruns() {
        let mut mixer = mixer(SinkMode::Program);
        mixer.authorize_lane(2, 1, MONO);
        let start = Instant::now();
        let initial = [8_192; 1_200];
        let packet = [8_192; FRAMES + 1];
        let mut output = [[0; 2]; FRAMES];
        mixer.push_pcm(&1, 1, None, &initial, start);
        mixer.push_pcm(&2, 1, None, &initial, start);

        for block in 0..12_000 {
            let now = start + block * BLOCK;
            // Each sender produces 240 +/- 0.048 frames per 5 ms of sink time.
            let difference = (block + 1) * 48 / 1_000 - block * 48 / 1_000;
            let faster = FRAMES + difference as usize;
            let slower = FRAMES - difference as usize;
            assert!(mixer.push_pcm(&1, 1, None, &packet[..faster], now));
            assert!(mixer.push_pcm(&2, 1, None, &packet[..slower], now));
            assert!(mixer.render(48_000, &mut output, now));
            assert!(mixer.lane_status(&1).unwrap().playing);
            assert!(mixer.lane_status(&2).unwrap().playing);
            if block > 0 {
                assert!(output.iter().all(|frame| *frame == [16_383; 2]));
            }
        }
        for (id, expected) in [(1, 0.0002), (2, -0.0002)] {
            let lane = mixer.lane_status(&id).unwrap();
            assert!((lane.rate_correction - expected).abs() < 0.00002, "{lane:?}");
            assert!((lane.buffered_ms - 7.0).abs() < 1.0, "{lane:?}");
        }
    }

    #[test]
    fn wrapped_sequences_fill_only_short_fresh_gaps_and_drop_late_packets() {
        let mut mixer = mixer(SinkMode::Program);
        let start = Instant::now();
        let packet = [8_192; 240];
        assert!(!mixer.push_pcm(&3, 1, None, &packet, start));
        assert!(mixer.push_pcm(&1, 1, Some(u32::MAX), &packet, start));
        assert!(mixer.push_pcm(&1, 1, Some(0), &packet, start + BLOCK));
        assert!(mixer.push_pcm(&1, 1, Some(2), &packet, start + 2 * BLOCK));
        assert!(!mixer.push_pcm(&1, 1, Some(2), &packet, start + 3 * BLOCK));
        assert!(!mixer.push_pcm(&1, 1, Some(u32::MAX), &packet, start + 3 * BLOCK));

        let mut output = [[0; 2]; FRAMES];
        assert!(mixer.render(48_000, &mut output, start + 2 * BLOCK));
        assert!(output.last().unwrap()[0] > 8_000);
        assert!(mixer.render(48_000, &mut output, start + 3 * BLOCK));
        assert!(output.iter().all(|frame| frame[0] > 8_000));
        assert!(mixer.render(48_000, &mut output, start + 4 * BLOCK));
        assert_eq!(output, [[0; 2]; FRAMES]);

        let before = mixer.lanes[&1].stream.write_pos;
        assert!(mixer.push_pcm(&1, 1, Some(11), &packet, start + 5 * BLOCK));
        assert!(mixer.push_pcm(&1, 1, Some(13), &packet, start + Duration::from_millis(130)));
        assert!(mixer.push_pcm(&1, 1, None, &packet, start + Duration::from_millis(131)));
        assert_eq!(mixer.lanes[&1].stream.write_pos - before, (3 * FRAMES) as u64);
    }

    #[test]
    fn caps_startup_backlog_and_skips_persistent_excess() {
        let mut mixer = mixer(SinkMode::Program);
        let start = Instant::now();
        let mut output = [[0; 2]; FRAMES];
        let packet = [2_000; FRAMES];
        assert!(mixer.push_pcm(&1, 1, None, &[2_000; 4_800], start));
        mixer.render(48_000, &mut output, start);
        assert_eq!(mixer.lane_status(&1).unwrap().buffered_ms, 22.0);

        assert!(mixer.push_pcm(&1, 1, None, &[2_000; 4_800], start + BLOCK));
        for block in 1..300 {
            let now = start + block * BLOCK;
            mixer.push_pcm(&1, 1, None, &packet, now);
            assert!(mixer.render(48_000, &mut output, now));
            if block == 198 {
                assert!(mixer.lane_status(&1).unwrap().buffered_ms > 100.0);
            }
        }
        assert!(mixer.lane_status(&1).unwrap().buffered_ms < 15.0);
    }

    #[test]
    fn writer_overflow_never_plays_overwritten_frames() {
        let mut mixer = mixer(SinkMode::Program);
        let now = Instant::now();
        let mut pcm = vec![-2000; CAPACITY * 2];
        pcm[CAPACITY..].fill(2000);
        mixer.push_pcm(&1, 1, None, &pcm, now);
        let mut output = [[0; 2]; FRAMES];
        assert!(mixer.render(48_000, &mut output, now));
        assert!(output.iter().all(|frame| frame[0] > 0));
    }

    #[test]
    fn short_mid_burst_underruns_raise_margin_with_mode_caps_and_slow_decay() {
        for (mode, cap) in [(SinkMode::Program, 150.0), (SinkMode::Microphone, 40.0)] {
            let mut mixer = mixer(mode);
            let start = Instant::now();
            let packet = [1_000; FRAMES];
            let mut output = [[0; 2]; FRAMES];
            for block in 0..330 {
                let now = start + block * BLOCK;
                mixer.push_pcm(&1, 1, None, &packet, now);
                mixer.render(48_000, &mut output, now);
            }

            let starved = start + 330 * BLOCK;
            mixer.render(48_000, &mut [[0; 2]; 2_400], starved);
            mixer.push_pcm(&1, 1, None, &packet, starved + BLOCK);
            assert_eq!(mixer.lane_status(&1).unwrap().extra_margin_ms, 10.0);
            let mut now = starved + BLOCK;
            let refill = [1_000; 8_640];
            let mut drain = [[0; 2]; 9_600];
            for _ in 0..16 {
                mixer.push_pcm(&1, 1, None, &refill, now);
                mixer.render(48_000, &mut drain, now);
                now += BLOCK;
            }
            mixer.push_pcm(&1, 1, None, &refill, now);
            assert_eq!(mixer.lane_status(&1).unwrap().extra_margin_ms, cap);
            for _ in 0..3_000 {
                mixer.push_pcm(&1, 1, None, &packet, now);
                mixer.render(48_000, &mut output, now);
                now += BLOCK;
            }
            let extra = mixer.lane_status(&1).unwrap().extra_margin_ms;
            assert!((extra - (cap - 1.05)).abs() < 0.0001, "{extra}");
        }
    }

    #[test]
    fn ramps_lane_gain_and_applies_master_and_policy_before_single_clip() {
        let mut mixer = mixer(SinkMode::Program);
        assert_eq!(slider_gain(0.5), 0.25);
        let now = Instant::now();
        mixer.authorize_lane(2, 1, MONO);
        mixer.set_lane_gain(&1, 2.0);
        mixer.set_lane_gain(&2, 2.0);
        mixer.push_pcm(&1, 1, None, &[24_576; 1_000], now);
        mixer.push_pcm(&2, 1, None, &[-8_192; 1_000], now);
        let mut output = [[0; 2]; 4];
        mixer.render(48_000, &mut output, now);
        assert_eq!(output, [[8_191; 2], [16_383; 2], [24_575; 2], [32_767; 2]]);
        assert_eq!(mixer.lane_status(&1).unwrap().peak, 1.0);
        mixer.set_master_gain(0.5);
        mixer.set_policy(PlaybackPolicy::Lower);
        mixer.render(48_000, &mut output, now);
        assert_eq!(output, [[2_457; 2]; 4]);
        mixer.set_policy(PlaybackPolicy::Off);
        mixer.set_master_gain(1.0);
        mixer.set_lane_muted(&2, true);
        mixer.render(48_000, &mut output, now);
        assert_eq!(output, [[32_767; 2]; 4]);
        for policy in [PlaybackPolicy::Pause, PlaybackPolicy::Mute] {
            mixer.set_policy(policy);
            assert!(mixer.render(48_000, &mut output, now));
            assert_eq!(output, [[0; 2]; 4]);
        }
        mixer.set_policy(PlaybackPolicy::Off);
        mixer.set_master_muted(true);
        assert!(mixer.render(48_000, &mut output, now));
        assert_eq!(output, [[0; 2]; 4]);
    }

    #[test]
    fn interpolates_between_source_frames_when_upsampling() {
        let mut mixer = LiveAudioMixer::new(LatencyPreset::Lowest, SinkMode::Program);
        let format = AudioFormat { sample_rate: 24_000, channels: 1 };
        let now = Instant::now();
        mixer.authorize_lane(1, 1, format);
        let pcm: Vec<_> = (0..100).map(|sample| sample * 128).collect();
        mixer.push_pcm(&1, 1, None, &pcm, now);
        mixer.render(48_000, &mut [[0; 2]; 2], now);
        let mut output = [[0; 2]; 4];
        assert!(mixer.render(48_000, &mut output, now));
        assert_eq!(output, [[127; 2], [191; 2], [255; 2], [319; 2]]);
    }

    #[test]
    fn removal_format_changes_and_generation_changes_are_lane_local() {
        let mut mixer = mixer(SinkMode::Program);
        let now = Instant::now();
        mixer.authorize_lane(2, 2, MONO);
        for (id, sample) in [(1, 1_000), (2, 2_000)] {
            mixer.push_pcm(&id, id, None, &[sample; 1_000], now);
        }
        let mut output = [[0; 2]; FRAMES];
        mixer.render(48_000, &mut output, now);
        assert_eq!(output.last(), Some(&[2_999; 2]));
        assert!(mixer.remove_lane(&1));
        assert_eq!(mixer.lane_status(&1), None);
        mixer.render(48_000, &mut output, now + BLOCK);
        assert_eq!(output, [[1_999; 2]; FRAMES]);

        let stereo = AudioFormat { sample_rate: 24_000, channels: 2 };
        assert!(mixer.authorize_lane(2, 3, MONO));
        assert!(!mixer.push_pcm(&2, 2, None, &[1_000; 1_000], now));
        assert!(!mixer.render(48_000, &mut output, now));
        mixer.push_pcm(&2, 3, Some(10), &[2_000; 1_000], now);
        assert!(mixer.authorize_lane(2, 3, stereo));
        assert!(!mixer.render(48_000, &mut output, now));
        assert!(!mixer.push_pcm_le(&2, 3, Some(0), &[0; 3], now));
        let pcm: Vec<_> = (0..400).flat_map(|_| [0x00, 0x20, 0x00, 0xe0]).collect();
        assert!(mixer.push_pcm_le(&2, 3, Some(0), &pcm, now));
        assert!(mixer.render(48_000, &mut output, now));
        assert_eq!(output.last(), Some(&[8_191, -8_191]));
        mixer.set_lane_active(&2, false);
        assert!(!mixer.push_pcm_le(&2, 3, Some(1), &pcm, now));
        assert!(!mixer.render(48_000, &mut output, now));
    }
}
