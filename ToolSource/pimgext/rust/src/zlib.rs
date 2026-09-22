//! Thin zlib wrappers behaving like Python's zlib.compress / zlib.decompress /
//! zlib.crc32 (same library, same parameters, same error messages).

use std::ffi::{c_int, c_void, CStr};
use std::mem::{size_of, MaybeUninit};
use std::ptr;

use libz_sys as z;

extern "C" {
    fn calloc(n: usize, size: usize) -> *mut c_void;
    fn free(p: *mut c_void);
}

extern "C" fn zalloc(_: z::voidpf, items: z::uInt, size: z::uInt) -> z::voidpf {
    unsafe { calloc(items as usize, size as usize) }
}

extern "C" fn zfree(_: z::voidpf, p: z::voidpf) {
    unsafe { free(p) }
}

fn new_stream() -> Box<z::z_stream> {
    // SAFETY: every field is either overwritten here or plain data zlib initialises.
    let mut s: Box<MaybeUninit<z::z_stream>> = Box::new(MaybeUninit::zeroed());
    unsafe {
        let p = s.as_mut_ptr();
        ptr::addr_of_mut!((*p).zalloc).write(zalloc);
        ptr::addr_of_mut!((*p).zfree).write(zfree);
        Box::from_raw(Box::into_raw(s) as *mut z::z_stream)
    }
}

const CHUNK_MAX: usize = u32::MAX as usize;

/// Point the stream's output at the spare capacity of `out`, growing it like
/// CPython (initial 16 KiB, then doubling).
fn arrange_output(s: &mut z::z_stream, out: &mut Vec<u8>) {
    if out.len() == out.capacity() {
        let extra = if out.capacity() == 0 { 16384 } else { out.capacity() };
        out.reserve(extra);
    }
    let avail = (out.capacity() - out.len()).min(CHUNK_MAX);
    s.next_out = unsafe { out.as_mut_ptr().add(out.len()) };
    s.avail_out = avail as z::uInt;
}

/// Apply what zlib wrote into the spare capacity.
fn commit_output(s: &z::z_stream, out: &mut Vec<u8>) {
    let written = s.next_out as usize - out.as_ptr() as usize;
    unsafe { out.set_len(written) };
}

/// zlib.compress(data, level)
pub fn compress(data: &[u8], level: c_int) -> Vec<u8> {
    let mut s = new_stream();
    let mut out = Vec::new();
    unsafe {
        let ret = z::deflateInit2_(
            &mut *s,
            level,
            z::Z_DEFLATED,
            15,
            8,
            z::Z_DEFAULT_STRATEGY,
            z::zlibVersion(),
            size_of::<z::z_stream>() as c_int,
        );
        assert_eq!(ret, z::Z_OK, "deflateInit2 failed");
        let mut rest = data;
        loop {
            let n = rest.len().min(CHUNK_MAX);
            s.next_in = rest.as_ptr() as *mut u8;
            s.avail_in = n as z::uInt;
            rest = &rest[n..];
            let flush = if rest.is_empty() { z::Z_FINISH } else { z::Z_NO_FLUSH };
            loop {
                arrange_output(&mut s, &mut out);
                let ret = z::deflate(&mut *s, flush);
                commit_output(&s, &mut out);
                assert_ne!(ret, z::Z_STREAM_ERROR, "deflate failed");
                if s.avail_out != 0 {
                    break;
                }
            }
            if flush == z::Z_FINISH {
                break;
            }
        }
        z::deflateEnd(&mut *s);
    }
    out
}

fn zlib_error(err: c_int, msg: *const std::ffi::c_char) -> String {
    let zmsg = if !msg.is_null() {
        Some(unsafe { CStr::from_ptr(msg) }.to_string_lossy().into_owned())
    } else {
        match err {
            z::Z_BUF_ERROR => Some("incomplete or truncated stream".into()),
            z::Z_STREAM_ERROR => Some("inconsistent stream state".into()),
            z::Z_DATA_ERROR => Some("invalid input data".into()),
            _ => None,
        }
    };
    match zmsg {
        Some(m) => format!("Error {} while decompressing data: {}", err, m),
        None => format!("Error {} while decompressing data", err),
    }
}

/// zlib.decompress(data); Err holds str(zlib.error).
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut s = new_stream();
    let mut out = Vec::new();
    unsafe {
        let ret = z::inflateInit2_(&mut *s, 15, z::zlibVersion(), size_of::<z::z_stream>() as c_int);
        if ret != z::Z_OK {
            return Err(format!("Error {} while preparing to decompress data", ret));
        }
        let mut rest = data;
        let mut err;
        loop {
            let n = rest.len().min(CHUNK_MAX);
            s.next_in = rest.as_ptr() as *mut u8;
            s.avail_in = n as z::uInt;
            rest = &rest[n..];
            let flush = if rest.is_empty() { z::Z_FINISH } else { z::Z_NO_FLUSH };
            loop {
                arrange_output(&mut s, &mut out);
                err = z::inflate(&mut *s, flush);
                commit_output(&s, &mut out);
                match err {
                    z::Z_OK | z::Z_BUF_ERROR | z::Z_STREAM_END => {}
                    _ => {
                        let m = zlib_error(err, s.msg);
                        z::inflateEnd(&mut *s);
                        return Err(m);
                    }
                }
                if s.avail_out != 0 {
                    break;
                }
            }
            if err == z::Z_STREAM_END || rest.is_empty() {
                break;
            }
        }
        if err != z::Z_STREAM_END {
            let m = zlib_error(err, s.msg);
            z::inflateEnd(&mut *s);
            return Err(m);
        }
        z::inflateEnd(&mut *s);
    }
    Ok(out)
}

/// zlib.crc32(data)
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc: z::uLong = 0;
    for chunk in data.chunks(1 << 30) {
        crc = unsafe { z::crc32(crc, chunk.as_ptr(), chunk.len() as z::uInt) };
    }
    crc as u32
}
