//! json.dump(tree, f, ensure_ascii=False, indent=2) with the pure-Python
//! encoder's output, including where it stops on RecursionError.

use crate::error::{recursion_error, PyErr};
use crate::psb::{call, Dict, Val};
use crate::pyfmt::float_repr;

/// Frame of the root dict's _iterencode_dict generator
/// (module, main, process, extract_raw, json.dump, _iterencode, _iterencode_dict).
const ROOT_FRAME: u32 = 7;

/// Returns the text written so far, and the error that stopped it (if any).
pub fn dump(tree: &Dict) -> (String, Option<PyErr>) {
    let mut out = String::new();
    let err = encode_dict(&mut out, tree, 0, ROOT_FRAME).err();
    (out, err)
}

/// c_encode_basestring (ensure_ascii=False)
fn encode_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\x08' => out.push_str("\\b"),
            '\x0c' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn floatstr(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f == f64::INFINITY {
        "Infinity".into()
    } else if f == f64::NEG_INFINITY {
        "-Infinity".into()
    } else {
        float_repr(f)
    }
}

fn indent(level: usize) -> String {
    format!("\n{}", "  ".repeat(level))
}

/// Scalars as the encoder yields them; None for containers.
fn scalar(v: &Val, frame: u32) -> Option<Result<String, PyErr>> {
    Some(Ok(match v {
        Val::Str(s) => {
            let mut o = String::new();
            encode_str(&mut o, s);
            o
        }
        Val::None => "null".into(),
        Val::Bool(true) => "true".into(),
        Val::Bool(false) => "false".into(),
        Val::Int(i) => i.to_string(),
        // _floatstr is a Python function → one more frame
        Val::Float(f) => {
            if frame + 1 > crate::psb::MAX_FRAMES {
                return Some(Err(recursion_error()));
            }
            floatstr(*f)
        }
        // extract_raw() replaces every Binary with a string
        Val::Bin(b) => format!("\"<binary:{}>\"", b.index),
        Val::List(_) | Val::Dict(_) => return None,
    }))
}

fn encode_child(out: &mut String, v: &Val, level: usize, frame: u32) -> Result<(), PyErr> {
    call(frame + 1)?;
    match v {
        Val::List(l) => encode_list(out, l, level, frame + 1),
        Val::Dict(d) => encode_dict(out, d, level, frame + 1),
        _ => unreachable!(),
    }
}

fn encode_list(out: &mut String, lst: &[Val], level: usize, frame: u32) -> Result<(), PyErr> {
    if lst.is_empty() {
        out.push_str("[]");
        return Ok(());
    }
    let level = level + 1;
    let newline_indent = indent(level);
    let mut buf = format!("[{}", newline_indent);
    for (i, value) in lst.iter().enumerate() {
        if i > 0 {
            buf = format!(",{}", newline_indent);
        }
        match scalar(value, frame) {
            Some(s) => {
                // `yield buf + ...`: nothing is emitted if the value fails
                let s = s?;
                out.push_str(&buf);
                out.push_str(&s);
            }
            None => {
                out.push_str(&buf);
                encode_child(out, value, level, frame)?;
            }
        }
    }
    out.push_str(&indent(level - 1));
    out.push(']');
    Ok(())
}

fn encode_dict(out: &mut String, dct: &Dict, level: usize, frame: u32) -> Result<(), PyErr> {
    if dct.is_empty() {
        out.push_str("{}");
        return Ok(());
    }
    out.push('{');
    let level = level + 1;
    let newline_indent = indent(level);
    let mut first = true;
    for (key, value) in dct.iter() {
        if first {
            first = false;
            out.push_str(&newline_indent);
        } else {
            out.push(',');
            out.push_str(&newline_indent);
        }
        encode_str(out, key);
        out.push_str(": ");
        match scalar(value, frame) {
            Some(s) => out.push_str(&s?),
            None => encode_child(out, value, level, frame)?,
        }
    }
    out.push_str(&indent(level - 1));
    out.push('}');
    Ok(())
}
