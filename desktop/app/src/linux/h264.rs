use std::io::Result;

#[derive(Debug, Clone)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub plane_rows: usize,
    pub nv12: Vec<u8>,
    pub colors: Colors,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Colors {
    pub bt601: bool,
    pub full_range: bool,
}

#[derive(Debug)]
pub struct Decoder {
    codec_name: &'static str,
}

pub(crate) fn ensure_mf_started() -> Result<()> {
    Ok(())
}

impl Decoder {
    pub fn new() -> Result<Decoder> {
        Self::new_for_codec("h264")
    }

    pub fn new_for_codec(codec: &str) -> Result<Decoder> {
        ensure_mf_started()?;
        let codec_name = if codec.eq_ignore_ascii_case("hevc")
            || codec.eq_ignore_ascii_case("h265")
        {
            "hevc"
        } else {
            "h264"
        };
        Ok(Decoder { codec_name })
    }

    pub fn new_gpu(codec: &str) -> Result<Decoder> {
        Self::new_for_codec(codec)
    }

    pub fn on_gpu(&self) -> bool {
        false
    }

    pub fn codec(&self) -> &str {
        self.codec_name
    }

    pub fn decode(&mut self, _data: &[u8], _time_us: u64) -> Result<Vec<Picture>> {
        Ok(Vec::new())
    }

    pub fn flush(&mut self) {}
}

pub fn to_bgrx(picture: &Picture, width: u32, height: u32) -> Vec<u8> {
    let w = (width as usize).min(picture.width as usize);
    let h = (height as usize).min(picture.height as usize);
    let stride = picture.stride.max(w);
    let uv_base = stride * picture.plane_rows.max(h);

    let (y_scale, y_bias, c_rv, c_gu, c_gv, c_bu) =
        match (picture.colors.bt601, picture.colors.full_range) {
            (false, false) => (298, 16, 459, 55, 136, 541),
            (false, true) => (256, 0, 403, 48, 120, 475),
            (true, false) => (298, 16, 409, 100, 208, 516),
            (true, true) => (256, 0, 359, 88, 183, 454),
        };

    let mut out = vec![0u8; w * h * 4];
    for row in 0..h {
        let y_row = row * stride;
        let uv_row = uv_base + (row / 2) * stride;
        let dst_row = row * w * 4;
        for col in 0..w {
            let y_val = i32::from(*picture.nv12.get(y_row + col).unwrap_or(&0));
            let uv_col = (col & !1) + uv_row;
            let u_val = i32::from(*picture.nv12.get(uv_col).unwrap_or(&128)) - 128;
            let v_val = i32::from(*picture.nv12.get(uv_col + 1).unwrap_or(&128)) - 128;

            let c = (y_val - y_bias).max(0) * y_scale;
            let r = (c + c_rv * v_val + 128) >> 8;
            let g = (c - c_gu * u_val - c_gv * v_val + 128) >> 8;
            let b = (c + c_bu * u_val + 128) >> 8;

            let dst = dst_row + col * 4;
            out[dst] = b.clamp(0, 255) as u8;
            out[dst + 1] = g.clamp(0, 255) as u8;
            out[dst + 2] = r.clamp(0, 255) as u8;
            out[dst + 3] = 0xFF;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_bgrx_converts_neutral_luma_to_grayscale() {
        let width = 4;
        let height = 2;
        let stride = 4;
        let plane_rows = 2;
        let mut nv12 = vec![128u8; stride * plane_rows];
        nv12.extend(vec![128u8; stride * (plane_rows / 2)]);
        let pic = Picture {
            width: width as u32,
            height: height as u32,
            stride,
            plane_rows,
            nv12,
            colors: Colors {
                bt601: false,
                full_range: true,
            },
        };
        let bgrx = to_bgrx(&pic, width as u32, height as u32);
        assert_eq!(bgrx.len(), width * height * 4);
        for px in bgrx.chunks_exact(4) {
            assert_eq!(px[0], 128);
            assert_eq!(px[1], 128);
            assert_eq!(px[2], 128);
            assert_eq!(px[3], 255);
        }
    }
}
