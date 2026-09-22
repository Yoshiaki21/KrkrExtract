//! PSB reader (port of KrkrExtract.Core/PsbWorker.cpp, offsets as 32bit ints).
//!
//! Mirrors pimgext.py exactly, including Python's forgiving slicing (reads past
//! the end yield short/empty data instead of failing) and its recursion limit.

use std::collections::HashMap;

use crate::error::{index_error, recursion_error, struct_short, PyErr, R};
use crate::zlib;

pub const PSB_TYPE_DICT: u8 = 0x21;

/// Python's default recursion limit: a call that would make the stack deeper
/// than this many frames raises RecursionError. The frame accounting below
/// reproduces where pimgext.py hits it on CPython 3.14 (approximate: CPython's
/// adaptive specialization can shift the exact point by a frame or two).
pub const MAX_FRAMES: u32 = 1000;

/// Raise RecursionError if a Python function called at stack depth `frame`
/// would exceed the recursion limit.
pub fn call(frame: u32) -> R<()> {
    if frame > MAX_FRAMES {
        Err(recursion_error())
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Binary {
    pub index: u64,
    pub offset: u64,
    pub size: u64,
}

/// A value of the PSB tree (the Python objects PsbFile.value() returns).
#[derive(Debug)]
pub enum Val {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Bin(Binary),
    List(Vec<Val>),
    Dict(Dict),
}

/// Insertion-ordered dict with Python semantics (a repeated key keeps its
/// first position and takes the last value).
#[derive(Debug, Default)]
pub struct Dict {
    items: Vec<(String, Val)>,
    index: HashMap<String, usize>,
}

impl Dict {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, key: String, val: Val) {
        if let Some(&i) = self.index.get(&key) {
            self.items[i].1 = val;
        } else {
            self.index.insert(key.clone(), self.items.len());
            self.items.push((key, val));
        }
    }

    pub fn get(&self, key: &str) -> Option<&Val> {
        self.index.get(key).map(|&i| &self.items[i].1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Val)> {
        self.items.iter().map(|(k, v)| (k, v))
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// data[a:b] with Python slice clamping.
pub fn slice(data: &[u8], a: u64, b: u64) -> &[u8] {
    let len = data.len() as u64;
    let a = a.min(len);
    let b = b.clamp(a, len);
    &data[a as usize..b as usize]
}

/// data[p] (IndexError if out of range).
fn byte(data: &[u8], p: u64) -> R<u8> {
    usize::try_from(p).ok().and_then(|p| data.get(p).copied()).ok_or_else(index_error)
}

/// int.from_bytes(s, 'little')
fn le_uint(s: &[u8]) -> u64 {
    s.iter().rev().fold(0u64, |acc, &b| (acc << 8) | b as u64)
}

/// int.from_bytes(s, 'little', signed=True) for len(s) <= 8
fn le_int(s: &[u8]) -> i64 {
    if s.is_empty() {
        return 0;
    }
    let v = le_uint(s);
    let bits = s.len() * 8;
    if bits == 64 {
        v as i64
    } else {
        ((v << (64 - bits)) as i64) >> (64 - bits)
    }
}

fn unpack_mdf(data: &[u8]) -> R<Vec<u8>> {
    if data.len() <= 10 {
        return Err(PyErr::Pimg("mdf: file too small".into()));
    }
    let size = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let out = zlib::decompress(&data[8..])
        .map_err(|e| PyErr::Pimg(format!("mdf: zlib decompression failed ({})", e)))?;
    if out.len() != size {
        return Err(PyErr::Pimg(format!(
            "mdf: size mismatch (header {}, actual {})",
            size,
            out.len()
        )));
    }
    Ok(out)
}

struct PsbArray {
    count: u64,
    size: u64,
    code: u64,
    nbytes: u64,
}

impl PsbArray {
    fn new(data: &[u8], p: u64) -> R<Self> {
        let t = byte(data, p)?;
        if !(13..=16).contains(&t) {
            return Err(PyErr::Pimg(format!("psb: invalid array header 0x{:02x} at 0x{:x}", t, p)));
        }
        let l = (t - 11) as u64;
        let count = le_uint(slice(data, p + 1, p + l));
        let size = byte(data, p + l)? as i64 - 12;
        if !(1..=4).contains(&size) {
            return Err(PyErr::Pimg(format!("psb: invalid array element size at 0x{:x}", p)));
        }
        let size = size as u64;
        Ok(PsbArray { count, size, code: p + l + 1, nbytes: count * size + l + 1 })
    }

    /// __getitem__ (no bounds check, like the Python version)
    fn get(&self, data: &[u8], i: u64) -> u64 {
        let o = self.code + i * self.size;
        le_uint(slice(data, o, o + self.size))
    }
}

pub struct PsbFile {
    pub data: Vec<u8>,
    p_str_pool: u64,
    p_bin_pool: u64,
    p_root: u64,
    name_a1: PsbArray,
    name_a2: PsbArray,
    name_a3: PsbArray,
    str_offsets: PsbArray,
    bin_offsets: PsbArray,
    bin_sizes: PsbArray,
}

impl PsbFile {
    pub fn new(data: Vec<u8>) -> R<Self> {
        let data = if data.starts_with(b"mdf\0") { unpack_mdf(&data)? } else { data };
        if data.len() < 64 || &data[..4] != b"PSB\0" {
            return Err(PyErr::Pimg("not a PSB/PIMG file".into()));
        }
        let u16_at = |o: usize| u16::from_le_bytes(data[o..o + 2].try_into().unwrap());
        let u32_at = |o: usize| u32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as u64;
        let (version, flag) = (u16_at(4), u16_at(6));
        if version != 2 && version != 3 {
            return Err(PyErr::Pimg(format!("unsupported PSB version {}", version)));
        }
        let p_str_index = u32_at(12);
        let p_str_offset = u32_at(16);
        let p_str_pool = u32_at(20);
        let p_bin_offset = u32_at(24);
        let p_bin_size = u32_at(28);
        let p_bin_pool = u32_at(32);
        let p_root = u32_at(36);

        if (flag & 3) != 0 || p_root >= data.len() as u64 || data[p_root as usize] != PSB_TYPE_DICT {
            return Err(PyErr::Pimg("encrypted PSB is not supported".into()));
        }

        let mut p = p_str_index;
        let name_a1 = PsbArray::new(&data, p)?;
        p += name_a1.nbytes;
        let name_a2 = PsbArray::new(&data, p)?;
        p += name_a2.nbytes;
        let name_a3 = PsbArray::new(&data, p)?;
        let str_offsets = PsbArray::new(&data, p_str_offset)?;
        let bin_offsets = PsbArray::new(&data, p_bin_offset)?;
        let bin_sizes = PsbArray::new(&data, p_bin_size)?;

        Ok(PsbFile {
            data,
            p_str_pool,
            p_bin_pool,
            p_root,
            name_a1,
            name_a2,
            name_a3,
            str_offsets,
            bin_offsets,
            bin_sizes,
        })
    }

    fn name(&self, idx: u64) -> R<String> {
        let d = &self.data[..];
        let (a1, a2) = (&self.name_a1, &self.name_a2);
        let mut out = Vec::new();
        let mut c1 = a2.get(d, self.name_a3.get(d, idx));
        while c1 != 0 {
            let c2 = a2.get(d, c1);
            out.push((c1 as i64 - a1.get(d, c2) as i64) as u8);
            c1 = c2;
            // A cyclic name table makes Python loop until it runs out of memory.
            if out.len() > (1 << 28) {
                return Err(PyErr::fatal("MemoryError", ""));
            }
        }
        out.reverse();
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    fn string(&self, idx: u64) -> R<String> {
        let d = &self.data[..];
        let s = self.p_str_pool + self.str_offsets.get(d, idx);
        let rest = if s < d.len() as u64 { &d[s as usize..] } else { &[][..] };
        let e = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| PyErr::Value("subsection not found".into()))?;
        Ok(String::from_utf8_lossy(&rest[..e]).into_owned())
    }

    /// data[offset:offset + size]
    pub fn binary(&self, b: &Binary) -> &[u8] {
        slice(&self.data, b.offset, b.offset + b.size)
    }

    /// root() — called from process() (module, main, process, root → value at frame 5).
    pub fn root(&self) -> R<Val> {
        self.value(self.p_root, 5)
    }

    /// value(p) running as Python frame number `frame`.
    fn value(&self, p: u64, frame: u32) -> R<Val> {
        let data = &self.data[..];
        let t = byte(data, p)?;
        Ok(match t {
            0 | 1 => Val::None,
            2 => Val::Bool(true),
            3 => Val::Bool(false),
            4 => Val::Int(0),
            5..=12 => Val::Int(le_int(slice(data, p + 1, p + t as u64 - 3))),
            29 => Val::Float(0.0),
            30 => {
                if (data.len() as u64) < p + 5 {
                    return Err(struct_short(p + 1, 4, data.len()));
                }
                let o = (p + 1) as usize;
                Val::Float(f32::from_le_bytes(data[o..o + 4].try_into().unwrap()) as f64)
            }
            31 => {
                if (data.len() as u64) < p + 9 {
                    return Err(struct_short(p + 1, 8, data.len()));
                }
                let o = (p + 1) as usize;
                Val::Float(f64::from_le_bytes(data[o..o + 8].try_into().unwrap()))
            }
            21..=24 => {
                // self.string() → self.str_offsets[idx]
                call(frame + 2)?;
                Val::Str(self.string(le_uint(slice(data, p + 1, p + t as u64 - 19)))?)
            }
            25..=28 => {
                // self.bin_offsets[i], self.bin_sizes[i], Binary()
                call(frame + 1)?;
                let i = le_uint(slice(data, p + 1, p + t as u64 - 23));
                Val::Bin(Binary {
                    index: i,
                    offset: self.p_bin_pool + self.bin_offsets.get(data, i),
                    size: self.bin_sizes.get(data, i),
                })
            }
            0x20 => {
                // PsbArray(), len(a), a[i], self.value()
                call(frame + 1)?;
                let a = PsbArray::new(data, p + 1)?;
                let base = p + 1 + a.nbytes;
                let mut list = Vec::new();
                for i in 0..a.count {
                    list.push(self.value(base + a.get(data, i), frame + 1)?);
                }
                Val::List(list)
            }
            PSB_TYPE_DICT => {
                call(frame + 1)?;
                let names = PsbArray::new(data, p + 1)?;
                let offsets = PsbArray::new(data, p + 1 + names.nbytes)?;
                let base = p + 1 + names.nbytes + offsets.nbytes;
                let mut dict = Dict::new();
                for i in 0..names.count {
                    // self.name(). Its PsbArray.__getitem__ calls run hot and get
                    // specialized by CPython, which then skips the recursion check
                    // (measured with Python 3.14), so they add no frame here.
                    call(frame + 1)?;
                    let key = self.name(names.get(data, i))?;
                    let v = self.value(base + offsets.get(data, i), frame + 1)?;
                    dict.insert(key, v);
                }
                Val::Dict(dict)
            }
            _ => {
                return Err(PyErr::Pimg(format!("psb: unknown value type 0x{:02x} at 0x{:x}", t, p)));
            }
        })
    }
}
