//! Error kinds mirroring the Python exceptions that pimgext.py distinguishes.

#[derive(Debug)]
pub enum PyErr {
    /// PimgError
    Pimg(String),
    /// IndexError
    Index(String),
    /// ValueError
    Value(String),
    /// struct.error
    Struct(String),
    /// OSError (`text` is already formatted like str(OSError))
    Os { errno: Option<i32>, text: String },
    /// An exception pimgext.py never catches (the Python version dies with a
    /// traceback). `kind` is the exception class name.
    Fatal { kind: &'static str, msg: String },
}

pub type R<T> = Result<T, PyErr>;

impl PyErr {
    /// str(exception)
    pub fn text(&self) -> String {
        match self {
            PyErr::Pimg(m) | PyErr::Index(m) | PyErr::Value(m) | PyErr::Struct(m) => m.clone(),
            PyErr::Os { text, .. } => text.clone(),
            PyErr::Fatal { msg, .. } => msg.clone(),
        }
    }

    pub fn fatal(kind: &'static str, msg: impl Into<String>) -> Self {
        PyErr::Fatal { kind, msg: msg.into() }
    }
}

pub fn index_error() -> PyErr {
    PyErr::Index("index out of range".into())
}

pub fn recursion_error() -> PyErr {
    PyErr::fatal("RecursionError", "maximum recursion depth exceeded")
}

pub fn type_error(msg: impl Into<String>) -> PyErr {
    PyErr::fatal("TypeError", msg)
}

/// struct.unpack_from() on a too short buffer.
pub fn struct_short(offset: u64, size: u64, buflen: usize) -> PyErr {
    PyErr::Struct(format!(
        "unpack_from requires a buffer of at least {} bytes for unpacking {} bytes at offset {} (actual buffer size is {})",
        offset + size,
        size,
        offset,
        buflen
    ))
}

/// bytearray(n): zero-filled buffer, failing the way CPython does.
pub fn alloc_zeroed(n: i128) -> R<Vec<u8>> {
    if n > isize::MAX as i128 || n < isize::MIN as i128 {
        return Err(PyErr::fatal("OverflowError", "cannot fit 'int' into an index-sized integer"));
    }
    if n < 0 {
        return Err(PyErr::fatal("ValueError", "negative count"));
    }
    let n = n as usize;
    let mut v = Vec::new();
    v.try_reserve_exact(n).map_err(|_| PyErr::fatal("MemoryError", ""))?;
    v.resize(n, 0);
    Ok(v)
}
