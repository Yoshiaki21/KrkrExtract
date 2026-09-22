//! Minimal RGBA PNG writer (same bytes as pimgext.py's write_png).

use crate::error::{PyErr, R};
use crate::pypath::OutFile;
use crate::zlib;

const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

fn chunk(out: &mut Vec<u8>, tag: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(tag);
    out.extend_from_slice(body);
    let crc = zlib::crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// write_png(path, w, h, rgba). `w`/`h` are Python ints and may be out of the
/// PNG range for a bogus canvas size, which makes struct.pack fail.
pub fn write_png(path: &str, w: i128, h: i128, rgba: &[u8]) -> R<()> {
    let mut raw = Vec::new();
    // (a canvas with h > 0 always has w >= 0: bytearray() rejects the rest)
    if h > 0 && w >= 0 {
        let stride = (w * 4) as usize;
        raw.reserve((stride + 1) * h as usize);
        for y in 0..h as usize {
            raw.push(0);
            let a = (y * stride).min(rgba.len());
            let b = ((y + 1) * stride).min(rgba.len());
            raw.extend_from_slice(&rgba[a..b]);
        }
    }

    let mut f = OutFile::create(path)?;
    let range = 0..=u32::MAX as i128;
    if !range.contains(&w) || !range.contains(&h) {
        f.write(SIGNATURE)?;
        return Err(PyErr::fatal("struct.error", "'I' format requires 0 <= number <= 4294967295"));
    }

    let mut out = Vec::from(SIGNATURE);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib::compress(&raw, 6));
    chunk(&mut out, b"IEND", b"");
    f.write(&out)
}
