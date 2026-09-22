//! os.path / os helpers with CPython semantics (posixpath on Unix, ntpath on
//! Windows), plus file I/O that reports errors like str(OSError).

use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;

use crate::error::{PyErr, R};
use crate::pyfmt::str_repr;

/// Line terminator Python's text-mode files and std streams produce.
#[cfg(windows)]
pub const NL: &str = "\r\n";
#[cfg(not(windows))]
pub const NL: &str = "\n";

#[cfg(windows)]
const SEPS: &[char] = &['\\', '/'];
#[cfg(not(windows))]
const SEPS: &[char] = &['/'];

fn is_sep(c: char) -> bool {
    SEPS.contains(&c)
}

// ---------------------------------------------------------------------------
// posixpath
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
pub fn split(p: &str) -> (String, String) {
    let i = p.rfind('/').map_or(0, |i| i + 1);
    let (head, tail) = (&p[..i], &p[i..]);
    let head = if !head.is_empty() && !head.chars().all(|c| c == '/') {
        head.trim_end_matches('/')
    } else {
        head
    };
    (head.to_string(), tail.to_string())
}

#[cfg(not(windows))]
pub fn join(a: &str, b: &str) -> String {
    if b.starts_with('/') {
        b.to_string()
    } else if a.is_empty() || a.ends_with('/') {
        format!("{}{}", a, b)
    } else {
        format!("{}/{}", a, b)
    }
}

#[cfg(not(windows))]
fn normpath(p: &str) -> String {
    if p.is_empty() {
        return ".".into();
    }
    let initial = if p.starts_with('/') {
        if p.starts_with("//") && !p.starts_with("///") {
            2
        } else {
            1
        }
    } else {
        0
    };
    let mut comps: Vec<&str> = Vec::new();
    for comp in p.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp != ".." || (initial == 0 && comps.is_empty()) || comps.last() == Some(&"..") {
            comps.push(comp);
        } else if !comps.is_empty() {
            comps.pop();
        }
    }
    let s = format!("{}{}", "/".repeat(initial), comps.join("/"));
    if s.is_empty() {
        ".".into()
    } else {
        s
    }
}

#[cfg(not(windows))]
pub fn abspath(p: &str) -> R<String> {
    if p.starts_with('/') {
        return Ok(normpath(p));
    }
    let cwd = std::env::current_dir().map_err(|e| os_error(&e, None, Api::Crt))?;
    Ok(normpath(&join(&cwd.to_string_lossy(), p)))
}

// ---------------------------------------------------------------------------
// ntpath
// ---------------------------------------------------------------------------

#[cfg(windows)]
fn splitroot(p: &str) -> (String, String, String) {
    let normp: String = p.replace('/', "\\");
    let chars: Vec<char> = p.chars().collect();
    let nchars: Vec<char> = normp.chars().collect();
    let s = |a: usize, b: usize| -> String { chars[a.min(chars.len())..b.min(chars.len())].iter().collect() };
    let find = |from: usize| nchars.iter().skip(from).position(|&c| c == '\\').map(|i| i + from);
    let n = chars.len();
    if nchars.first() == Some(&'\\') {
        if nchars.get(1) == Some(&'\\') {
            let head: String = nchars.iter().take(8).collect::<String>().to_uppercase();
            let start = if head == "\\\\?\\UNC\\" { 8 } else { 2 };
            let Some(index) = find(start) else { return (p.into(), String::new(), String::new()) };
            let Some(index2) = find(index + 1) else { return (p.into(), String::new(), String::new()) };
            (s(0, index2), s(index2, index2 + 1), s(index2 + 1, n))
        } else {
            (String::new(), s(0, 1), s(1, n))
        }
    } else if nchars.get(1) == Some(&':') {
        if nchars.get(2) == Some(&'\\') {
            (s(0, 2), s(2, 3), s(3, n))
        } else {
            (s(0, 2), String::new(), s(2, n))
        }
    } else {
        (String::new(), String::new(), p.into())
    }
}

#[cfg(windows)]
pub fn split(p: &str) -> (String, String) {
    let (d, r, p) = splitroot(p);
    let chars: Vec<char> = p.chars().collect();
    let mut i = chars.len();
    while i > 0 && !is_sep(chars[i - 1]) {
        i -= 1;
    }
    let head: String = chars[..i].iter().collect();
    let tail: String = chars[i..].iter().collect();
    (format!("{}{}{}", d, r, head.trim_end_matches(SEPS)), tail)
}

#[cfg(windows)]
pub fn join(a: &str, b: &str) -> String {
    let (mut rd, mut rr, mut rp) = splitroot(a);
    let (pd, pr, pp) = splitroot(b);
    let mut done = false;
    if !pr.is_empty() {
        if !pd.is_empty() || rd.is_empty() {
            rd = pd;
        }
        rr = pr;
        rp = pp.clone();
        done = true;
    } else if !pd.is_empty() && pd != rd {
        if pd.to_lowercase() != rd.to_lowercase() {
            rd = pd;
            rr = pr;
            rp = pp.clone();
            done = true;
        } else {
            rd = pd;
        }
    }
    if !done {
        if rp.chars().last().is_some_and(|c| !is_sep(c)) {
            rp.push('\\');
        }
        rp.push_str(&pp);
    }
    if !rp.is_empty() && rr.is_empty() && rd.chars().last().is_some_and(|c| !":\\/".contains(c)) {
        return format!("{}\\{}", rd, rp);
    }
    format!("{}{}{}", rd, rr, rp)
}

#[cfg(windows)]
pub fn abspath(p: &str) -> R<String> {
    let p = if p.is_empty() { "." } else { p };
    std::path::absolute(p)
        .map(|a| a.to_string_lossy().into_owned())
        .map_err(|e| os_error(&e, None, Api::Win))
}

// ---------------------------------------------------------------------------
// common
// ---------------------------------------------------------------------------

pub fn basename(p: &str) -> String {
    split(p).1
}

pub fn dirname(p: &str) -> String {
    split(p).0
}

/// os.path.splitext
pub fn splitext(p: &str) -> (String, String) {
    let chars: Vec<char> = p.chars().collect();
    let sep_index = chars.iter().rposition(|&c| is_sep(c)).map_or(-1, |i| i as isize);
    if let Some(dot) = chars.iter().rposition(|&c| c == '.') {
        if dot as isize > sep_index {
            let mut i = (sep_index + 1) as usize;
            while i < dot {
                if chars[i] != '.' {
                    return (chars[..dot].iter().collect(), chars[dot..].iter().collect());
                }
                i += 1;
            }
        }
    }
    (p.to_string(), String::new())
}

pub fn exists(p: &str) -> bool {
    fs::metadata(p).is_ok()
}

pub fn isdir(p: &str) -> bool {
    fs::metadata(p).is_ok_and(|m| m.is_dir())
}

pub fn isfile(p: &str) -> bool {
    fs::metadata(p).is_ok_and(|m| m.is_file())
}

/// Which layer reported the error: the C runtime (errno, used by open()) or
/// the Win32 API (used by os.mkdir / os.listdir on Windows).
#[derive(Clone, Copy)]
pub enum Api {
    Crt,
    #[cfg_attr(not(windows), allow(dead_code))]
    Win,
}

const EEXIST: i32 = 17;

/// str(OSError(...)) for an I/O error, optionally carrying a filename.
pub fn os_error(e: &io::Error, filename: Option<&str>, api: Api) -> PyErr {
    let _ = api;
    let Some(code) = e.raw_os_error() else {
        return PyErr::Os { errno: None, text: e.to_string() };
    };
    #[cfg(windows)]
    let (errno, head) = match api {
        Api::Crt => {
            let errno = crt_errno(code);
            (errno, format!("[Errno {}] {}", errno, crt_strerror(errno)))
        }
        Api::Win => {
            let msg = e.to_string();
            let msg = msg.trim_end_matches(&format!(" (os error {})", code));
            let msg = msg.trim_end_matches(|c: char| c <= ' ' || c == '.');
            let errno = crt_errno(code);
            (errno, format!("[WinError {}] {}", code, msg))
        }
    };
    #[cfg(not(windows))]
    let (errno, head) = {
        let msg = e.to_string();
        let msg = msg.trim_end_matches(&format!(" (os error {})", code)).to_string();
        (code, format!("[Errno {}] {}", code, msg))
    };
    let text = match filename {
        Some(f) => format!("{}: {}", head, str_repr(f)),
        None => head,
    };
    PyErr::Os { errno: Some(errno), text }
}

/// _dosmaperr(): Win32 error code -> errno
#[cfg(windows)]
fn crt_errno(code: i32) -> i32 {
    match code {
        2 | 3 | 15 | 18 | 53 | 67 | 161 | 206 => 2, // ENOENT
        4 => 24,                                    // EMFILE
        5 | 19..=36 | 65 | 82 | 108 | 1816 => 13,   // EACCES
        6 => 9,                                     // EBADF
        8 | 14 => 12,                               // ENOMEM
        80 | 183 => 17,                             // EEXIST
        112 => 28,                                  // ENOSPC
        145 => 41,                                  // ENOTEMPTY
        _ => 22,                                    // EINVAL
    }
}

#[cfg(windows)]
fn crt_strerror(errno: i32) -> &'static str {
    match errno {
        2 => "No such file or directory",
        9 => "Bad file descriptor",
        12 => "Not enough space",
        13 => "Permission denied",
        17 => "File exists",
        24 => "Too many open files",
        28 => "No space left on device",
        41 => "Directory not empty",
        _ => "Invalid argument",
    }
}

/// os.makedirs(name, exist_ok=exist_ok)
pub fn makedirs(name: &str, exist_ok: bool) -> R<()> {
    let (mut head, mut tail) = split(name);
    if tail.is_empty() {
        (head, tail) = split(&head);
    }
    if !head.is_empty() && !tail.is_empty() && !exists(&head) {
        match makedirs(&head, exist_ok) {
            Err(PyErr::Os { errno: Some(EEXIST), .. }) => {}
            r => r?,
        }
        if tail == "." {
            return Ok(());
        }
    }
    if let Err(e) = fs::create_dir(name) {
        if !exist_ok || !isdir(name) {
            #[cfg(windows)]
            return Err(os_error(&e, Some(name), Api::Win));
            #[cfg(not(windows))]
            return Err(os_error(&e, Some(name), Api::Crt));
        }
    }
    Ok(())
}

/// open(path, 'rb').read()
pub fn read_file(path: &str) -> R<Vec<u8>> {
    let mut f = fs::File::open(path).map_err(|e| os_error(&e, Some(path), Api::Crt))?;
    let mut data = Vec::new();
    f.read_to_end(&mut data).map_err(|e| os_error(&e, None, Api::Crt))?;
    Ok(data)
}

/// An open(path, 'wb') file.
pub struct OutFile(fs::File);

impl OutFile {
    pub fn create(path: &str) -> R<Self> {
        fs::File::create(path).map(OutFile).map_err(|e| os_error(&e, Some(path), Api::Crt))
    }

    pub fn write(&mut self, data: &[u8]) -> R<()> {
        self.0.write_all(data).map_err(|e| os_error(&e, None, Api::Crt))
    }
}

/// with open(path, 'wb') as f: f.write(data)
pub fn write_file(path: &str, data: &[u8]) -> R<()> {
    OutFile::create(path)?.write(data)
}

/// os.listdir(path)
pub fn listdir(path: &str) -> R<Vec<String>> {
    #[cfg(windows)]
    let api = Api::Win;
    #[cfg(not(windows))]
    let api = Api::Crt;
    let err = |e: io::Error| os_error(&e, Some(path), api);
    let mut names = Vec::new();
    for entry in fs::read_dir(Path::new(path)).map_err(err)? {
        names.push(entry.map_err(err)?.file_name().to_string_lossy().into_owned());
    }
    Ok(names)
}
