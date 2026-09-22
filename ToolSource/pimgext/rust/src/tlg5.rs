//! TLG5 decoder -> RGBA

use crate::error::{alloc_zeroed, index_error, struct_short, PyErr, R};
use crate::psb::slice;

pub const MAGIC: &[u8; 11] = b"TLG5.0\0raw\x1a";

pub struct Image {
    pub w: u32,
    pub h: u32,
    pub px: Vec<u8>,
}

fn at(src: &[u8], i: u64) -> R<u8> {
    src.get(i as usize).copied().ok_or_else(index_error)
}

pub fn decode(buf: &[u8]) -> R<Image> {
    if buf.len() < 11 || &buf[..11] != MAGIC {
        return Err(PyErr::Pimg("not a TLG5 image".into()));
    }
    if buf.len() < 24 {
        return Err(struct_short(11, 13, buf.len()));
    }
    let colors = buf[11];
    let rd = |o: usize| u32::from_le_bytes(buf[o..o + 4].try_into().unwrap()) as u64;
    let (w, h, bh) = (rd(12), rd(16), rd(20));
    if (colors != 3 && colors != 4) || bh == 0 {
        return Err(PyErr::Pimg(format!("unsupported TLG5 (colors={})", colors)));
    }

    let nblk = h.div_ceil(bh);
    let mut p = 24 + 4 * nblk;
    let mut text = [0u8; 4096];
    let mut r = 0usize;
    let stride = w * 4;
    let mut out = alloc_zeroed(stride as i128 * h as i128)?;
    let zero_row = alloc_zeroed(stride as i128)?;
    let stride = stride as usize;

    let mut by = 0u64;
    while by < h {
        let rows = bh.min(h - by);
        let mut chans: Vec<Vec<u8>> = Vec::with_capacity(colors as usize);
        for _ in 0..colors {
            let mark = at(buf, p)?;
            if (buf.len() as u64) < p + 5 {
                return Err(struct_short(p + 1, 4, buf.len()));
            }
            let size = u32::from_le_bytes(buf[p as usize + 1..p as usize + 5].try_into().unwrap()) as u64;
            p += 5;
            let src = slice(buf, p, p + size);
            p += size;
            if mark != 0 {
                chans.push(src.to_vec());
                continue;
            }
            let mut o = Vec::new();
            let mut i = 0u64;
            let mut flags = 0u32;
            while i < size {
                flags >>= 1;
                if flags & 0x100 == 0 {
                    flags = at(src, i)? as u32 | 0xFF00;
                    i += 1;
                }
                if flags & 1 != 0 {
                    let b0 = at(src, i)? as usize;
                    let b1 = at(src, i + 1)? as usize;
                    let mut mpos = b0 | ((b1 & 0x0F) << 8);
                    let mut mlen = (b1 >> 4) + 3;
                    i += 2;
                    if mlen == 18 {
                        mlen += at(src, i)? as usize;
                        i += 1;
                    }
                    for _ in 0..mlen {
                        let v = text[mpos];
                        o.push(v);
                        text[r] = v;
                        r = (r + 1) & 0xFFF;
                        mpos = (mpos + 1) & 0xFFF;
                    }
                } else {
                    let v = at(src, i)?;
                    i += 1;
                    o.push(v);
                    text[r] = v;
                    r = (r + 1) & 0xFFF;
                }
            }
            chans.push(o);
        }

        let need = (rows * w) as usize;
        if chans[0].len() < need {
            return Err(PyErr::Pimg("TLG5: truncated block".into()));
        }
        if need > 0 && chans[1..].iter().any(|c| c.len() < need) {
            return Err(PyErr::Index("bytearray index out of range".into()));
        }
        let (cb, cg, cr) = (&chans[0], &chans[1], &chans[2]);
        let ca = if colors == 4 { Some(&chans[3]) } else { None };
        for yy in 0..rows as usize {
            let y = by as usize + yy;
            let (before, cur) = out.split_at_mut(y * stride);
            let prev = if y == 0 { &zero_row[..] } else { &before[(y - 1) * stride..] };
            let row = &mut cur[..stride];
            let base = yy * w as usize;
            let (mut pb, mut pg, mut pr, mut pa) = (0u8, 0u8, 0u8, 0u8);
            for x in 0..w as usize {
                let g = cg[base + x];
                pb = pb.wrapping_add(cb[base + x]).wrapping_add(g);
                pg = pg.wrapping_add(g);
                pr = pr.wrapping_add(cr[base + x]).wrapping_add(g);
                if let Some(ca) = ca {
                    pa = pa.wrapping_add(ca[base + x]);
                }
                let k = x * 4;
                row[k] = prev[k].wrapping_add(pr);
                row[k + 1] = prev[k + 1].wrapping_add(pg);
                row[k + 2] = prev[k + 2].wrapping_add(pb);
                row[k + 3] = prev[k + 3].wrapping_add(pa);
            }
            if ca.is_none() {
                for k in (3..stride).step_by(4) {
                    row[k] = 0xFF;
                }
            }
        }
        by += bh;
    }
    Ok(Image { w: w as u32, h: h as u32, px: out })
}
