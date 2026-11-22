use compat_common::record;
use core::ffi::c_void;

pub const SNAP_FRAMES: [u64; 8] = [10, 60, 240, 600, 1200, 2400, 4800, 7200];

pub fn dir() -> Option<String> {
    std::env::var("TOMBOX_PRESENT_SNAP").ok().filter(|v| !v.trim().is_empty() && v != "0")
}

pub fn wanted(frame: u64) -> bool {
    dir().is_some() && SNAP_FRAMES.contains(&frame)
}

pub fn row_pitch(width: u32) -> u32 {
    (width * 4).div_ceil(256) * 256
}

pub fn bytes_per_pixel(format: u32) -> u32 {
    match format {
        9 | 10 => 8,
        _ => 4,
    }
}

pub fn pitch_for(width: u32, format: u32) -> u32 {
    (width * bytes_per_pixel(format)).div_ceil(256) * 256
}

pub fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    let e = ((h >> 10) & 0x1f) as i32;
    let m = (h & 0x3ff) as f32;
    match e {
        0 => sign * m * (2.0f32).powi(-24),
        31 => if m == 0.0 { sign * f32::INFINITY } else { f32::NAN },
        _ => sign * (1.0 + m / 1024.0) * (2.0f32).powi(e - 15),
    }
}

pub fn linear_to_srgb8(v: f32) -> u8 {
    let c = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
    let s = if c <= 0.003_130_8 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub struct Stats {
    pub nonzero: u64,
    pub total: u64,
    pub distinct_rows: u64,
}

pub fn stats(data: &[u8], pitch: usize, width: usize, height: usize) -> Stats {
    stats_fmt(data, pitch, width, height, 28)
}

pub fn stats_fmt(data: &[u8], pitch: usize, width: usize, height: usize, format: u32) -> Stats {
    let bpp = bytes_per_pixel(format) as usize;
    let mut nonzero = 0u64;
    let mut distinct_rows = 0u64;
    let mut prev: Option<&[u8]> = None;
    for y in 0..height {
        let row = &data[y * pitch..y * pitch + width * bpp];
        for px in row.chunks_exact(bpp) {
            if px[..bpp - bpp / 4].iter().any(|&b| b != 0) {
                nonzero += 1;
            }
        }
        if prev != Some(row) {
            distinct_rows += 1;
        }
        prev = Some(row);
    }
    Stats { nonzero, total: (width * height) as u64, distinct_rows }
}

pub fn to_bgr(px: &[u8], format: u32) -> [u8; 3] {
    match format {
        9 | 10 => {
            let c = |i: usize| linear_to_srgb8(half_to_f32(u16::from_le_bytes([px[i * 2], px[i * 2 + 1]])));
            [c(2), c(1), c(0)]
        }
        87 | 88 | 90 | 91 => [px[0], px[1], px[2]],
        23 | 24 | 25 => {
            let v = u32::from_le_bytes([px[0], px[1], px[2], px[3]]);
            let c = |s: u32| (((v >> s) & 0x3ff) >> 2) as u8;
            [c(20), c(10), c(0)]
        }
        _ => [px[2], px[1], px[0]],
    }
}

pub fn bmp(data: &[u8], pitch: usize, width: usize, height: usize, format: u32) -> Vec<u8> {
    let bpp = bytes_per_pixel(format) as usize;
    let row = (width * 3).div_ceil(4) * 4;
    let size = 54 + row * height;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(size as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(height as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&[0u8; 24]);
    for y in (0..height).rev() {
        let src = &data[y * pitch..y * pitch + width * bpp];
        let start = out.len();
        for px in src.chunks_exact(bpp) {
            out.extend_from_slice(&to_bgr(px, format));
        }
        out.resize(start + row, 0);
    }
    out
}

pub fn footprint_location(buf: *mut c_void, format: u32, width: u32, height: u32) -> [u8; 0x30] {
    let mut l = [0u8; 0x30];
    l[0..8].copy_from_slice(&(buf as u64).to_le_bytes());
    l[8..12].copy_from_slice(&1u32.to_le_bytes());
    l[0x18..0x1c].copy_from_slice(&format.to_le_bytes());
    l[0x1c..0x20].copy_from_slice(&width.to_le_bytes());
    l[0x20..0x24].copy_from_slice(&height.to_le_bytes());
    l[0x24..0x28].copy_from_slice(&1u32.to_le_bytes());
    l[0x28..0x2c].copy_from_slice(&pitch_for(width, format).to_le_bytes());
    l
}

pub fn subresource_location(tex: *mut c_void) -> [u8; 0x30] {
    let mut l = [0u8; 0x30];
    l[0..8].copy_from_slice(&(tex as u64).to_le_bytes());
    l
}

pub fn finish(tag: &str, frame: u64, data: &[u8], width: u32, height: u32, format: u32, extra: &str) {
    let pitch = pitch_for(width, format) as usize;
    let s = stats_fmt(data, pitch, width as usize, height as usize, format);
    let mut saved = String::new();
    if let Some(d) = dir() {
        let _ = std::fs::create_dir_all(&d);
        let path = std::path::Path::new(&d).join(format!("{tag}_frame{frame:05}.bmp"));
        if std::fs::write(&path, bmp(data, pitch, width as usize, height as usize, format)).is_ok() {
            saved = format!(" saved {}", path.display());
        }
    }
    record("d3d12_xs", "PresentX.snap", &format!(
        "frame {frame} {tag}: {} of {} pixels non-black ({:.2}%), {} distinct rows of {height}{extra}{saved}",
        s.nonzero, s.total, 100.0 * s.nonzero as f64 / s.total.max(1) as f64, s.distinct_rows));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stats_count_non_black_pixels_and_distinct_rows() {
        let pitch = 256;
        let mut d = vec![0u8; pitch * 3];
        d[4..8].copy_from_slice(&[10, 0, 0, 255]);
        d[pitch * 2 + 8..pitch * 2 + 12].copy_from_slice(&[0, 0, 3, 0]);
        d[pitch + 12..pitch + 16].copy_from_slice(&[0, 0, 0, 255]);
        let s = stats(&d, pitch, 8, 3);
        assert_eq!(s, Stats { nonzero: 2, total: 24, distinct_rows: 3 }, "alpha alone is still black");
        assert_eq!(stats(&vec![0u8; pitch * 4], pitch, 8, 4).distinct_rows, 1);
    }

    #[test]
    fn bmp_is_bottom_up_bgr_with_padded_rows() {
        let pitch = 256;
        let mut d = vec![0u8; pitch * 2];
        d[0..4].copy_from_slice(&[1, 2, 3, 255]);
        d[pitch..pitch + 4].copy_from_slice(&[4, 5, 6, 255]);
        let b = bmp(&d, pitch, 1, 2, 28);
        assert_eq!(&b[0..2], b"BM");
        assert_eq!(b.len(), 54 + 4 * 2);
        assert_eq!(&b[54..57], &[6, 5, 4], "the bottom row comes first, RGBA swapped to BGR");
        assert_eq!(&b[58..61], &[3, 2, 1]);
        assert_eq!(to_bgr(&[9, 8, 7, 0], 87), [9, 8, 7]);
        let v: u32 = (1023 << 20) | (512 << 10) | 4;
        assert_eq!(to_bgr(&v.to_le_bytes(), 24), [255, 128, 1]);
    }

    #[test]
    fn footprint_rows_are_256_byte_aligned() {
        assert_eq!(row_pitch(1920), 7680);
        assert_eq!(row_pitch(1), 256);
        assert_eq!(row_pitch(65), 512);
        let l = footprint_location(0x1000 as *mut c_void, 28, 1920, 1080);
        assert_eq!(u32::from_le_bytes(l[8..12].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(l[0x28..0x2c].try_into().unwrap()), 7680);
        assert!(!wanted(11));
        let f = footprint_location(0x1000 as *mut c_void, 10, 1920, 1080);
        assert_eq!(u32::from_le_bytes(f[0x28..0x2c].try_into().unwrap()), 15360);
    }

    #[test]
    fn fp16_pitch_and_conversion() {
        assert_eq!(bytes_per_pixel(10), 8);
        assert_eq!(bytes_per_pixel(9), 8);
        assert_eq!(bytes_per_pixel(87), 4);
        assert_eq!(bytes_per_pixel(24), 4);
        assert_eq!(pitch_for(1920, 10), 15360);
        assert_eq!(pitch_for(1920, 28), row_pitch(1920));
        assert_eq!(pitch_for(33, 10), 512);
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0x3800), 0.5);
        assert_eq!(half_to_f32(0xbc00), -1.0);
        assert_eq!(half_to_f32(0), 0.0);
        assert!(half_to_f32(0x7e00).is_nan());
        assert_eq!(linear_to_srgb8(1.0), 255);
        assert_eq!(linear_to_srgb8(4.0), 255);
        assert_eq!(linear_to_srgb8(-1.0), 0);
        assert_eq!(linear_to_srgb8(0.5), 188);
        let px: Vec<u8> = [0x3c00u16, 0x3800, 0, 0x3c00].iter().flat_map(|h| h.to_le_bytes()).collect();
        assert_eq!(to_bgr(&px, 10), [0, 188, 255]);
        let pitch = 256;
        let mut d = vec![0u8; pitch * 2];
        d[..8].copy_from_slice(&px);
        let b = bmp(&d, pitch, 2, 2, 10);
        assert_eq!(b.len(), 54 + 8 * 2);
        assert_eq!(&b[62..65], &[0, 188, 255], "top row is last in the bottom-up file");
        let s = stats_fmt(&d, pitch, 2, 2, 10);
        assert_eq!(s.nonzero, 1);
        assert_eq!(s.total, 4);
        let mut alpha_only = vec![0u8; pitch];
        alpha_only[6..8].copy_from_slice(&0x3c00u16.to_le_bytes());
        assert_eq!(stats_fmt(&alpha_only, pitch, 1, 1, 10).nonzero, 0);
    }
}
