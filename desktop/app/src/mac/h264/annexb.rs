// SPDX-License-Identifier: GPL-3.0-or-later
//! H.264 Annex B access units (docs/protocol/mirror.md §3) turned into what
//! VideoToolbox takes: the stream's SPS and PPS kept aside for its format
//! description, the rest as 4-byte length-prefixed NAL units (AVCC).

/// The protocol's packet limit; anything longer is refused unread.
pub(super) const MAX_ACCESS_UNIT: usize = 8 << 20;
/// NAL units in one access unit, at most (phones send a handful).
const MAX_NAL_UNITS: usize = 512;
/// A parameter set larger than this is refused (real ones are tens of bytes).
const MAX_PARAMETER_SET: usize = 4096;
/// Distinct PPS ids kept, at most.
const MAX_PPS: usize = 32;

const SLICE: u8 = 1;
const SLICE_IDR: u8 = 5;
const SEI: u8 = 6;
const SPS: u8 = 7;
const PPS: u8 = 8;

/// The parameter sets seen so far: the latest SPS and each PPS by id.
#[derive(Debug, Default)]
pub(super) struct ParameterSets {
    sps: Option<Vec<u8>>,
    pps: Vec<(u32, Vec<u8>)>,
    changed: bool,
}

impl ParameterSets {
    /// The SPS and every PPS, when there are both.
    pub(super) fn sets(&self) -> Option<(&[u8], Vec<&[u8]>)> {
        let sps = self.sps.as_deref()?;
        (!self.pps.is_empty()).then(|| (sps, self.pps.iter().map(|(_, pps)| pps.as_slice()).collect()))
    }

    /// Whether a set changed since the last call.
    pub(super) fn take_changed(&mut self) -> bool {
        std::mem::take(&mut self.changed)
    }

    fn set_sps(&mut self, nal: &[u8]) -> Result<(), String> {
        check_parameter_set("SPS", nal)?;
        if self.sps.as_deref() != Some(nal) {
            self.sps = Some(nal.to_vec());
            self.pps.clear();
            self.changed = true;
        }
        Ok(())
    }

    fn set_pps(&mut self, nal: &[u8]) -> Result<(), String> {
        check_parameter_set("PPS", nal)?;
        let id = first_ue(&nal[1..]).ok_or("a PPS without a readable id")?;
        let at_limit = self.pps.len() >= MAX_PPS;
        match self.pps.iter_mut().find(|(known, _)| *known == id) {
            Some((_, pps)) if pps.as_slice() == nal => return Ok(()),
            Some((_, pps)) => *pps = nal.to_vec(),
            None if at_limit => return Err(format!("more than {MAX_PPS} PPS ids")),
            None => self.pps.push((id, nal.to_vec())),
        }
        self.changed = true;
        Ok(())
    }
}

fn check_parameter_set(kind: &str, nal: &[u8]) -> Result<(), String> {
    if nal.len() < 2 || nal.len() > MAX_PARAMETER_SET {
        return Err(format!("a {kind} of {} bytes", nal.len()));
    }
    Ok(())
}

/// One access unit, ready for VideoToolbox.
#[derive(Debug)]
pub(super) struct AccessUnit {
    /// The coded slices and SEI, each after its length (big-endian u32).
    pub(super) avcc: Vec<u8>,
    /// Whether it holds a picture to decode (it may be parameter sets only).
    pub(super) has_slice: bool,
}

/// Splits an Annex B access unit, keeping its parameter sets in `sets`.
pub(super) fn access_unit(data: &[u8], sets: &mut ParameterSets) -> Result<AccessUnit, String> {
    if data.len() > MAX_ACCESS_UNIT {
        return Err(format!("an access unit of {} bytes", data.len()));
    }
    let mut avcc = Vec::with_capacity(data.len() + 16);
    let mut has_slice = false;
    for nal in nal_units(data)? {
        match nal[0] & 0x1f {
            SPS => sets.set_sps(nal)?,
            PPS => sets.set_pps(nal)?,
            kind @ (SLICE | SLICE_IDR | SEI) => {
                has_slice |= kind != SEI;
                // At most MAX_ACCESS_UNIT bytes, so it fits.
                avcc.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                avcc.extend_from_slice(nal);
            }
            // Data-partitioned slices (Extended profile): VideoToolbox can't.
            2..=4 => return Err("data-partitioned H.264 slices aren't supported".into()),
            // Delimiters, end of sequence or stream, filler, SVC/MVC layers
            // over the base one, reserved types: nothing to decode here.
            _ => {}
        }
    }
    Ok(AccessUnit { avcc, has_slice })
}

/// The NAL units of an Annex B buffer, without start codes or trailing zeros.
fn nal_units(data: &[u8]) -> Result<Vec<&[u8]>, String> {
    let first = start_code(data, 0).ok_or("not Annex B: no start code")?;
    if data[..first].iter().any(|&b| b != 0) {
        return Err("not Annex B: bytes before the first start code".into());
    }
    let mut units = Vec::new();
    let mut pos = first + 3;
    loop {
        let next = start_code(data, pos);
        let mut nal = &data[pos..next.unwrap_or(data.len())];
        // A 4-byte start code's leading zero, or trailing_zero_8bits.
        while let [rest @ .., 0] = nal {
            nal = rest;
        }
        if let Some(&header) = nal.first() {
            if header & 0x80 != 0 {
                return Err("a NAL unit with its forbidden bit set".into());
            }
            if units.len() == MAX_NAL_UNITS {
                return Err(format!("more than {MAX_NAL_UNITS} NAL units in an access unit"));
            }
            units.push(nal);
        }
        match next {
            Some(at) => pos = at + 3,
            None => return Ok(units),
        }
    }
}

/// Where the next `00 00 01` at or after `from` starts.
fn start_code(data: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 2 < data.len() {
        match data[i + 2] {
            1 if data[i] == 0 && data[i + 1] == 0 => return Some(i),
            0 => i += 1,
            _ => i += 3,
        }
    }
    None
}

/// The first Exp-Golomb `ue(v)` of an escaped RBSP (a PPS's id).
fn first_ue(escaped: &[u8]) -> Option<u32> {
    let mut bits = escaped
        .iter()
        .enumerate()
        // Emulation prevention: a 3 after two zeros isn't data.
        .filter(|&(i, &b)| !(b == 3 && i >= 2 && escaped[i - 2] == 0 && escaped[i - 1] == 0))
        .flat_map(|(_, &b)| (0..8).rev().map(move |bit| (b >> bit) & 1 == 1));
    let mut zeros = 0u32;
    while !bits.next()? {
        zeros += 1;
        if zeros > 31 {
            return None;
        }
    }
    let mut suffix = 0u64;
    for _ in 0..zeros {
        suffix = (suffix << 1) | u64::from(bits.next()?);
    }
    u32::try_from((1u64 << zeros) - 1 + suffix).ok()
}
