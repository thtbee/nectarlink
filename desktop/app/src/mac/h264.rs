// SPDX-License-Identifier: GPL-3.0-or-later
//! H.264 decoding with macOS's own VideoToolbox (Apple's hardware decoder
//! where the Mac has one): each Annex B access unit in, its picture out
//! right away, as NV12. HEVC and AV1 aren't decoded on macOS yet.

mod annexb;
mod vt;

/// A decoded picture: NV12, `stride` bytes per row, the chroma plane after
/// `plane_rows` rows of luma.
#[derive(Debug)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub plane_rows: usize,
    pub nv12: Vec<u8>,
    /// How its colors turn into RGB, as the stream says.
    pub colors: Colors,
}

/// The YUV-to-RGB formula a stream uses (its VUI, as the decoder reports
/// it). Phones are asked for BT.709 limited range, but not all comply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Colors {
    pub bt601: bool,
    pub full_range: bool,
}

impl Colors {
    /// Fixed-point (×256) coefficients: luma scale, then V→R, U→G, V→G, U→B.
    fn coefficients(self) -> (i32, i32, i32, i32, i32) {
        let luma = if self.full_range { 256 } else { 298 };
        let chroma = match (self.bt601, self.full_range) {
            (false, false) => (459, 55, 136, 541),
            (false, true) => (403, 48, 120, 475),
            (true, false) => (409, 100, 208, 516),
            (true, true) => (359, 88, 183, 454),
        };
        (luma, chroma.0, chroma.1, chroma.2, chroma.3)
    }
}

pub struct Decoder {
    codec: String,
    /// Whether to ask for Apple's hardware decoder first.
    prefer_gpu: bool,
    sets: annexb::ParameterSets,
    /// The format of the latest SPS and PPS.
    format: Option<vt::Format>,
    /// Started at the first slice, as it needs the stream's format.
    session: Option<vt::Session>,
    /// Last presentation timestamp submitted, in microseconds. Kept across
    /// flush: a new session must not repeat one CoreMedia already accepted.
    last_pts: Option<i64>,
}

impl std::fmt::Debug for Decoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Decoder")
            .field("codec", &self.codec)
            .field("on_gpu", &self.on_gpu())
            .finish_non_exhaustive()
    }
}

impl Decoder {
    /// A low-latency decoder for `"h264"`; VideoToolbox picks its decoder.
    pub fn new_for_codec(codec: &str) -> Result<Decoder, String> {
        Self::create(codec, false)
    }

    /// As [`Decoder::new_for_codec`], asking for Apple's hardware decoder
    /// first and falling back to whichever VideoToolbox has.
    pub fn new_gpu(codec: &str) -> Result<Decoder, String> {
        let mut decoder = Self::new_for_codec(codec)?;
        decoder.prefer_gpu = true;
        Ok(decoder)
    }

    fn create(codec: &str, prefer_gpu: bool) -> Result<Decoder, String> {
        if codec != "h264" {
            return Err(format!("{} video can't be decoded on macOS yet, only H.264", codec.to_uppercase()));
        }
        Ok(Decoder {
            codec: codec.to_owned(),
            prefer_gpu,
            sets: annexb::ParameterSets::default(),
            format: None,
            session: None,
            last_pts: None,
        })
    }

    /// Whether the hardware decoder runs the current session, as
    /// VideoToolbox reports it; false until the first picture's session starts.
    pub fn on_gpu(&self) -> bool {
        self.session.as_ref().is_some_and(vt::Session::on_gpu)
    }

    pub fn codec(&self) -> &str {
        &self.codec
    }

    /// Decodes one access unit (Annex B); returns the pictures it completed.
    /// After an error the session is dropped: the caller starts over from the
    /// next keyframe, which gets a fresh one.
    pub fn decode(&mut self, data: &[u8], time_us: u64) -> Result<Vec<Picture>, String> {
        let unit = annexb::access_unit(data, &mut self.sets)?;
        if self.sets.take_changed() {
            self.reformat()?;
        }
        if !unit.has_slice {
            return Ok(Vec::new());
        }
        let format = self.format.as_ref().ok_or("an H.264 slice before its SPS and PPS")?;
        // Reserve the timestamp before borrowing the session. A failed open
        // still keeps it, so a retry cannot submit the same value twice.
        let pts = monotonic_presentation_us(&mut self.last_pts, time_us)?;
        let session = match &mut self.session {
            Some(session) => session,
            none => none.insert(open(format, &mut self.prefer_gpu)?),
        };
        let result = session.decode(format, &unit.avcc, pts);
        if result.is_err() {
            self.session = None;
        }
        result
    }

    /// Starts over (after lost packets): the next keyframe decodes cleanly
    /// in a new session, with the parameter sets seen so far. The presentation
    /// clock stays, so that session cannot repeat a timestamp.
    pub fn flush(&mut self) {
        self.session = None;
    }

    /// Takes in new parameter sets, keeping the session if it can go on.
    fn reformat(&mut self) -> Result<(), String> {
        let Some((sps, pps)) = self.sets.sets() else {
            return Ok(());
        };
        match vt::Format::h264(sps, &pps) {
            Ok(format) => {
                if self.session.as_ref().is_some_and(|session| !session.can_accept(&format)) {
                    self.session = None;
                }
                self.format = Some(format);
                Ok(())
            }
            Err(e) => {
                self.format = None;
                self.session = None;
                self.sets = annexb::ParameterSets::default();
                Err(e)
            }
        }
    }
}

/// A session for `format`. Hardware is required when preferred. Only a
/// decoder-unavailable status falls back, and then not again for this decoder.
fn open(format: &vt::Format, prefer_gpu: &mut bool) -> Result<vt::Session, String> {
    if *prefer_gpu {
        match vt::Session::new(format, true) {
            Ok(session) => return Ok(session),
            Err(error) if error.hardware_unavailable() => {
                tracing::info!(
                    status = error.status_code(),
                    error = %error,
                    "hardware H.264 decoder unavailable; VideoToolbox picks a decoder"
                );
                *prefer_gpu = false;
            }
            Err(error) => return Err(error.into_message()),
        }
    }
    vt::Session::new(format, false).map_err(vt::StartError::into_message)
}

/// Presentation time in microseconds, strictly after `last` once one is set.
/// CoreMedia leaves equal timestamps in one stream undefined, and the protocol
/// only needs differences, so a repeat or a step backward becomes last + 1.
/// `u64` values that do not fit in `i64` saturate; the clock then stops with
/// an error instead of repeating `i64::MAX`.
fn monotonic_presentation_us(last: &mut Option<i64>, time_us: u64) -> Result<i64, String> {
    let requested = i64::try_from(time_us).unwrap_or(i64::MAX);
    let pts = match *last {
        Some(prev) if requested <= prev => {
            prev.checked_add(1).ok_or("H.264 presentation timestamps ran out of range")?
        }
        _ => requested,
    };
    *last = Some(pts);
    Ok(pts)
}

/// Pixels from which a frame is converted on several cores: 4K takes about
/// 24 ms on one core, most of a 30 fps frame's 33 ms, while 720p is quick
/// enough on the calling thread.
const PARALLEL_FROM_PIXELS: usize = 1920 * 1080;

/// NV12 (in the picture's own colors) to 32-bit BGRX, the visible
/// `width` x `height`. Large frames are split into bands of rows, one per
/// core (scoped threads cost microseconds, against milliseconds of work).
pub fn to_bgrx(picture: &Picture, width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width.min(picture.width) as usize, height.min(picture.height) as usize);
    let mut out = vec![0u8; w * h * 4];
    if w == 0 || h == 0 {
        return out;
    }
    if !holds(picture, w, h) {
        tracing::warn!(
            width = picture.width,
            height = picture.height,
            stride = picture.stride,
            len = picture.nv12.len(),
            "an NV12 picture smaller than it says; not converted"
        );
        return out;
    }
    let cores = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);
    if w * h < PARALLEL_FROM_PIXELS || cores < 2 {
        convert_rows(picture, w, 0, &mut out);
        return out;
    }
    // Bands of an even number of rows, so each starts on a chroma row.
    let band = h.div_ceil(cores).next_multiple_of(2);
    std::thread::scope(|scope| {
        for (i, rows) in out.chunks_mut(band * w * 4).enumerate() {
            scope.spawn(move || convert_rows(picture, w, i * band, rows));
        }
    });
    out
}

/// Whether `p` has the luma and chroma rows `convert_rows` reads for `w` x `h`.
fn holds(p: &Picture, w: usize, h: usize) -> bool {
    let needed = p
        .stride
        .checked_mul(p.plane_rows)
        .and_then(|luma| luma.checked_add(p.stride.checked_mul(h.div_ceil(2))?));
    p.stride >= w.next_multiple_of(2) && p.plane_rows >= h && needed.is_some_and(|n| p.nv12.len() >= n)
}

fn convert_rows(p: &Picture, w: usize, first: usize, out: &mut [u8]) {
    let uv_plane = p.stride * p.plane_rows;
    let (luma, vr, ug, vg, ub) = p.colors.coefficients();
    let black = if p.colors.full_range { 0 } else { 16 };
    for (i, row) in out.chunks_exact_mut(w * 4).enumerate() {
        let y_row = first + i;
        let y_line = &p.nv12[y_row * p.stride..][..w];
        let uv_line = &p.nv12[uv_plane + (y_row / 2) * p.stride..][..w.next_multiple_of(2)];
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let c = (i32::from(y_line[x]) - black) * luma;
            let d = i32::from(uv_line[x & !1]) - 128;
            let e = i32::from(uv_line[(x & !1) + 1]) - 128;
            px[0] = clamp((c + ub * d + 128) >> 8);
            px[1] = clamp((c - ug * d - vg * e + 128) >> 8);
            px[2] = clamp((c + vr * e + 128) >> 8);
            px[3] = 255;
        }
    }
}

fn clamp(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h264_keyframe_decodes_to_nv12_and_converts_in_its_range() {
        // The hardening's success path, without VideoToolbox: a later timestamp
        // is kept, a required-hardware create with no property answer stays
        // hardware, and a 64-byte aligned row has a checked offset.
        let mut clock = None;
        assert_eq!(monotonic_presentation_us(&mut clock, 0).unwrap(), 0);
        assert_eq!(monotonic_presentation_us(&mut clock, 33_333).unwrap(), 33_333);
        assert!(vt::hardware_in_use(true, None));
        assert!(vt::hardware_in_use(false, Some(true)));
        assert_eq!(vt::plane_row_offset(1, 64, 32).unwrap(), 64);

        // Synthetic solid red 32x32 baseline IDR (AUD, SPS, PPS, slice),
        // tagged BT.709 limited range.
        let mut decoder = Decoder::new_for_codec("h264").unwrap();
        let pictures = decoder.decode(include_bytes!("testdata/red-32x32.annexb"), 0).unwrap();
        assert_eq!(pictures.len(), 1);
        let picture = &pictures[0];
        assert_eq!((picture.width, picture.height), (32, 32));
        assert!(!picture.colors.bt601);
        let bgrx = to_bgrx(picture, 32, 32);
        assert_eq!(bgrx.len(), 32 * 32 * 4);
        // Read as full range, this red would come out near (15, 15, 239).
        for px in bgrx.as_chunks::<4>().0 {
            assert!(px[0] <= 10 && px[1] <= 10 && px[2] >= 244 && px[3] == 255, "{px:?}");
        }
    }

    #[test]
    fn decoder_refuses_other_codecs_and_malformed_h264() {
        assert!(Decoder::new_gpu("h265").is_err());
        assert!(Decoder::new_for_codec("av1").is_err());
        let mut decoder = Decoder::new_gpu("h264").unwrap();
        assert!(decoder.decode(&[0x65, 0x88, 0x84], 0).is_err(), "no start code");
        assert!(decoder.decode(&[0, 0, 0, 1, 0x65, 0x88, 0x84], 0).is_err(), "a slice before SPS and PPS");
        assert!(decoder.decode(&vec![0; annexb::MAX_ACCESS_UNIT + 1], 0).is_err(), "over the packet limit");
        assert!(!decoder.on_gpu());

        // Parameter and bad-data failures are not "no hardware". Only the
        // three decoder-unavailable statuses are. Unknown hardware stays false.
        assert!(!vt::hardware_decoder_unavailable(-12902));
        assert!(!vt::hardware_decoder_unavailable(-12904));
        assert!(!vt::hardware_decoder_unavailable(-12909));
        assert!(vt::hardware_decoder_unavailable(-12906));
        assert!(vt::hardware_decoder_unavailable(-12907));
        assert!(vt::hardware_decoder_unavailable(-12913));
        assert!(!vt::hardware_in_use(false, None));
        assert!(!vt::hardware_in_use(true, Some(false)));

        let mut clock = None;
        assert_eq!(monotonic_presentation_us(&mut clock, 10).unwrap(), 10);
        assert_eq!(monotonic_presentation_us(&mut clock, 10).unwrap(), 11);
        assert_eq!(monotonic_presentation_us(&mut clock, 9).unwrap(), 12);
        clock = Some(i64::MAX);
        assert!(monotonic_presentation_us(&mut clock, u64::MAX).is_err());
        assert_eq!(clock, Some(i64::MAX));

        assert!(vt::plane_row_offset(0, 31, 32).is_err(), "stride shorter than the row");
        assert!(vt::plane_row_offset(1, usize::MAX / 2, 32).is_err(), "stride past the cap");
        let wide = vt::max_plane_pitch(32);
        assert!(vt::plane_row_offset(1, wide, 32).is_ok());
        assert!(vt::plane_row_offset(1, wide + 1, 32).is_err());

        // Rotation must not combine a new SPS with an old encoder's PPS.
        let mut sets = annexb::ParameterSets::default();
        let fixture = include_bytes!("testdata/red-32x32.annexb");
        annexb::access_unit(fixture, &mut sets).unwrap();
        let mut changed_sps = sets.sets().unwrap().0.to_vec();
        changed_sps[1] ^= 1;
        let mut packet = vec![0, 0, 0, 1];
        packet.extend(changed_sps);
        annexb::access_unit(&packet, &mut sets).unwrap();
        assert!(sets.sets().is_none(), "a changed SPS invalidates previous PPS ids");
        annexb::access_unit(fixture, &mut sets).unwrap();
        assert!(sets.sets().is_some(), "the next complete keyframe restores both parameter sets");
    }
}
