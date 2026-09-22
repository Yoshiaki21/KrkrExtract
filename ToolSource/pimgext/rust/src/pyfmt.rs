//! Python-compatible formatting (str(), repr(), '%d') and numeric semantics.

use std::cmp::Ordering;

use crate::error::{type_error, PyErr, R};
use crate::psb::Val;

/// float.__repr__
pub fn float_repr(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x > 0.0 { "inf" } else { "-inf" }.into();
    }
    // Rust's {:e} gives the shortest round-trip digits, like Python's repr.
    let s = format!("{:e}", x);
    let (mant, exp) = s.split_once('e').unwrap();
    let exp: i32 = exp.parse().unwrap();
    let (neg, mant) = match mant.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, mant),
    };
    let digits: String = mant.chars().filter(|&c| c != '.').collect();
    let n = digits.len() as i32;
    let decpt = exp + 1;
    let mut out = String::new();
    if neg {
        out.push('-');
    }
    if -4 < decpt && decpt <= 16 {
        if decpt <= 0 {
            out.push_str("0.");
            out.push_str(&"0".repeat((-decpt) as usize));
            out.push_str(&digits);
        } else if decpt >= n {
            out.push_str(&digits);
            out.push_str(&"0".repeat((decpt - n) as usize));
            out.push_str(".0");
        } else {
            out.push_str(&digits[..decpt as usize]);
            out.push('.');
            out.push_str(&digits[decpt as usize..]);
        }
    } else {
        out.push_str(&digits[..1]);
        if n > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        out.push('e');
        out.push(if exp < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", exp.abs()));
    }
    out
}

/// Approximation of str.isprintable() for repr() (no Unicode tables in std).
fn is_printable(c: char) -> bool {
    let u = c as u32;
    !(u < 0x20
        || (0x7f..=0xa0).contains(&u)
        || u == 0xad
        || u == 0x1680
        || (0x2000..=0x200f).contains(&u)
        || (0x2028..=0x202f).contains(&u)
        || (0x205f..=0x206f).contains(&u)
        || u == 0x3000
        || (0xd800..=0xf8ff).contains(&u)
        || u == 0xfeff
        || (0xfff0..=0xfffb).contains(&u)
        || (0xf0000..=0x10ffff).contains(&u))
}

/// str.__repr__
pub fn str_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ if c == quote => {
                out.push('\\');
                out.push(c);
            }
            _ if !is_printable(c) => {
                let u = c as u32;
                if u < 0x100 {
                    out.push_str(&format!("\\x{:02x}", u));
                } else if u < 0x10000 {
                    out.push_str(&format!("\\u{:04x}", u));
                } else {
                    out.push_str(&format!("\\U{:08x}", u));
                }
            }
            _ => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// bytes.__repr__
pub fn bytes_repr(b: &[u8]) -> String {
    let quote = if b.contains(&b'\'') && !b.contains(&b'"') { b'"' } else { b'\'' };
    let mut out = String::from("b");
    out.push(quote as char);
    for &c in b {
        match c {
            b'\\' => out.push_str("\\\\"),
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            _ if c == quote => {
                out.push('\\');
                out.push(c as char);
            }
            0x20..=0x7e => out.push(c as char),
            _ => out.push_str(&format!("\\x{:02x}", c)),
        }
    }
    out.push(quote as char);
    out
}

/// str(v) / '%s' % v
pub fn py_str(v: &Val) -> String {
    match v {
        Val::Str(s) => s.clone(),
        _ => py_repr(v),
    }
}

/// repr(v)
pub fn py_repr(v: &Val) -> String {
    match v {
        Val::None => "None".into(),
        Val::Bool(true) => "True".into(),
        Val::Bool(false) => "False".into(),
        Val::Int(i) => i.to_string(),
        Val::Float(f) => float_repr(*f),
        Val::Str(s) => str_repr(s),
        // The Python version prints an object address here.
        Val::Bin(b) => format!("<__main__.Binary object at 0x{:x}>", 0x7f00_0000_0000u64 + b.index * 0x30),
        Val::List(l) => {
            let items: Vec<String> = l.iter().map(py_repr).collect();
            format!("[{}]", items.join(", "))
        }
        Val::Dict(d) => {
            let items: Vec<String> = d.iter().map(|(k, v)| format!("{}: {}", str_repr(k), py_repr(v))).collect();
            format!("{{{}}}", items.join(", "))
        }
    }
}

pub fn type_name(v: &Val) -> &'static str {
    match v {
        Val::None => "NoneType",
        Val::Bool(_) => "bool",
        Val::Int(_) => "int",
        Val::Float(_) => "float",
        Val::Str(_) => "str",
        Val::Bin(_) => "Binary",
        Val::List(_) => "list",
        Val::Dict(_) => "dict",
    }
}

/// A Python int or float.
#[derive(Clone, Copy, Debug)]
pub enum Num {
    I(i128),
    F(f64),
}

impl Num {
    /// isinstance(v, (int, float)) (bool is an int)
    pub fn of(v: &Val) -> Option<Num> {
        match v {
            Val::Int(i) => Some(Num::I(*i as i128)),
            Val::Bool(b) => Some(Num::I(*b as i128)),
            Val::Float(f) => Some(Num::F(*f)),
            _ => None,
        }
    }

    fn as_f64(self) -> f64 {
        match self {
            Num::I(i) => i as f64,
            Num::F(f) => f,
        }
    }

    /// Exact comparison like Python (None if NaN is involved).
    pub fn cmp(self, other: Num) -> Option<Ordering> {
        match (self, other) {
            (Num::I(a), Num::I(b)) => Some(a.cmp(&b)),
            (Num::F(a), Num::F(b)) => a.partial_cmp(&b),
            (Num::I(a), Num::F(b)) => cmp_int_float(a, b),
            (Num::F(a), Num::I(b)) => cmp_int_float(b, a).map(Ordering::reverse),
        }
    }

    pub fn eq(self, other: Num) -> bool {
        self.cmp(other) == Some(Ordering::Equal)
    }

    pub fn gt(self, other: Num) -> bool {
        self.cmp(other) == Some(Ordering::Greater)
    }

    pub fn lt(self, other: Num) -> bool {
        self.cmp(other) == Some(Ordering::Less)
    }

    pub fn add(self, other: Num) -> Num {
        match (self, other) {
            (Num::I(a), Num::I(b)) => Num::I(a + b),
            _ => Num::F(self.as_f64() + other.as_f64()),
        }
    }
}

fn cmp_int_float(i: i128, f: f64) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    let t = f.trunc();
    if t.abs() >= 1.0e38 {
        return Some(if f > 0.0 { Ordering::Less } else { Ordering::Greater });
    }
    match i.cmp(&(t as i128)) {
        Ordering::Equal => {
            let frac = f - t;
            Some(if frac > 0.0 {
                Ordering::Less
            } else if frac < 0.0 {
                Ordering::Greater
            } else {
                Ordering::Equal
            })
        }
        o => Some(o),
    }
}

/// v == n
pub fn eq_num(v: &Val, n: i128) -> bool {
    Num::of(v).is_some_and(|x| x.eq(Num::I(n)))
}

/// v + n (n is an int)
pub fn add_int(v: &Val, n: i128) -> R<Num> {
    match Num::of(v) {
        Some(x) => Ok(x.add(Num::I(n))),
        None => Err(type_error(match v {
            Val::Str(_) => "can only concatenate str (not \"int\") to str".to_string(),
            Val::List(_) => "can only concatenate list (not \"int\") to list".to_string(),
            _ => format!("unsupported operand type(s) for +: '{}' and 'int'", type_name(v)),
        })),
    }
}

/// max(0, v)
pub fn max0(v: &Val) -> R<Num> {
    match Num::of(v) {
        Some(x) => Ok(if x.gt(Num::I(0)) { x } else { Num::I(0) }),
        None => Err(type_error(format!(
            "'>' not supported between instances of '{}' and 'int'",
            type_name(v)
        ))),
    }
}

/// min(a, b)
pub fn min2(a: Num, b: Num) -> Num {
    if b.lt(a) {
        b
    } else {
        a
    }
}

/// '%d' % n
pub fn pct_d(n: Num) -> R<String> {
    match n {
        Num::I(i) => Ok(i.to_string()),
        Num::F(f) if f.is_nan() => Err(PyErr::fatal("ValueError", "cannot convert float NaN to integer")),
        Num::F(f) if f.is_infinite() => {
            Err(PyErr::fatal("OverflowError", "cannot convert float infinity to integer"))
        }
        Num::F(f) => Ok(float_to_int_string(f.trunc())),
    }
}

/// Exact decimal digits of an integral float (int(f)).
fn float_to_int_string(f: f64) -> String {
    if f.abs() < 1.0e38 {
        return (f as i128).to_string();
    }
    // f = m * 2^e exactly; build the decimal with base-1e9 limbs.
    let bits = f.abs().to_bits();
    let exp = ((bits >> 52) & 0x7ff) as i32 - 1075;
    let mant = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
    let mut limbs: Vec<u64> = vec![mant % 1_000_000_000, mant / 1_000_000_000 % 1_000_000_000, mant / 1_000_000_000_000_000_000];
    for _ in 0..exp {
        let mut carry = 0u64;
        for l in limbs.iter_mut() {
            let v = *l * 2 + carry;
            *l = v % 1_000_000_000;
            carry = v / 1_000_000_000;
        }
        if carry > 0 {
            limbs.push(carry);
        }
    }
    while limbs.len() > 1 && *limbs.last().unwrap() == 0 {
        limbs.pop();
    }
    let mut s = if f < 0.0 { String::from("-") } else { String::new() };
    s.push_str(&limbs.last().unwrap().to_string());
    for l in limbs.iter().rev().skip(1) {
        s.push_str(&format!("{:09}", l));
    }
    s
}

/// str.isspace() for str.strip()
pub fn is_py_space(c: char) -> bool {
    matches!(c,
        '\t' | '\n' | '\x0b' | '\x0c' | '\r' | '\x1c'..='\x1f' | ' ' | '\u{85}' | '\u{a0}'
        | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}'
        | '\u{205f}' | '\u{3000}')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats() {
        assert_eq!(float_repr(1.0), "1.0");
        assert_eq!(float_repr(-0.0), "-0.0");
        assert_eq!(float_repr(0.1), "0.1");
        assert_eq!(float_repr(1e16), "1e+16");
        assert_eq!(float_repr(1e15), "1000000000000000.0");
        assert_eq!(float_repr(0.0001), "0.0001");
        assert_eq!(float_repr(0.00001), "1e-05");
        assert_eq!(float_repr(0.1f32 as f64), "0.10000000149011612");
        assert_eq!(float_repr(1.5e300), "1.5e+300");
        assert_eq!(float_to_int_string(1e38), "99999999999999997748809823456034029568");
        assert_eq!(float_to_int_string(-2f64.powi(130)), "-1361129467683753853853498429727072845824");
    }

    #[test]
    fn reprs() {
        assert_eq!(str_repr("a'b"), "\"a'b\"");
        assert_eq!(str_repr("a'\"b"), "'a\\'\"b'");
        assert_eq!(str_repr("x\x01\u{3000}あ"), "'x\\x01\\u3000あ'");
        assert_eq!(bytes_repr(b"TLG6.0"), "b'TLG6.0'");
        assert_eq!(bytes_repr(b"\x00'\xff"), "b\"\\x00'\\xff\"");
    }
}
