//! pimgext - .pimg (PSB layered image) extractor.
//!
//! Rust port of ../pimgext.py with identical behaviour: same command line,
//! output files (byte for byte), warnings, errors and exit status.
//! See ../SPEC.md for the specification.

mod args;
mod error;
mod json;
mod png;
mod psb;
mod pyfmt;
mod pypath;
mod tlg5;
mod zlib;

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::rc::Rc;

use args::{Args, PROG};
use error::{alloc_zeroed, type_error, PyErr, R};
use psb::{Binary, Dict, PsbFile, Val};
use pyfmt::{add_int, bytes_repr, eq_num, is_py_space, max0, min2, pct_d, py_str, Num};
use pypath::{join, NL};
use tlg5::Image;

/// ltPsNormal (krkrz/visual/drawable.h)
const LT_PS_NORMAL: i128 = 13;

fn eprint_line(s: &str) {
    let mut err = std::io::stderr().lock();
    let _ = write!(err, "{}{}", s, NL);
    let _ = err.flush();
}

struct Context {
    label: String,
    quiet: bool,
}

impl Context {
    fn warn(&self, msg: &str) {
        if !self.quiet {
            eprint_line(&format!("{}: warning: {}: {}", PROG, self.label, msg));
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn safe_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| if "\\/:*?\"<>|".contains(c) || (c as u32) < 0x20 { '_' } else { c })
        .collect();
    let s = replaced.trim_matches(is_py_space);
    if s.is_empty() || s == "." || s == ".." {
        String::new()
    } else {
        s.to_string()
    }
}

/// Decode a TLG binary. Err(PimgError) if it is not possible.
fn decode_image(data: &[u8]) -> R<Image> {
    if data.len() < 11 || &data[..11] != tlg5::MAGIC {
        return Err(PyErr::Pimg(format!("not a TLG5 image ({})", bytes_repr(&data[..data.len().min(6)]))));
    }
    match tlg5::decode(data) {
        Err(PyErr::Index(m)) | Err(PyErr::Struct(m)) => Err(PyErr::Pimg(format!("TLG5 decode failed ({})", m))),
        r => r,
    }
}

/// Result of decode_image kept for reuse: the image or str(PimgError).
type Decoded = Result<Rc<Image>, String>;

fn decode_cached(psb: &PsbFile, b: &Binary) -> R<Decoded> {
    match decode_image(psb.binary(b)) {
        Ok(img) => Ok(Ok(Rc::new(img))),
        Err(PyErr::Pimg(m)) => Ok(Err(m)),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// raw/
// ---------------------------------------------------------------------------

/// Replace binaries by "<binary:N>" and collect them as _binN.bin.
fn to_json(v: &Val, extra: &mut Vec<(String, Binary)>) -> Val {
    match v {
        Val::Bin(b) => {
            let name = format!("_bin{}.bin", b.index);
            match extra.iter_mut().find(|(n, _)| *n == name) {
                Some(e) => e.1 = *b,
                None => extra.push((name, *b)),
            }
            Val::Str(format!("<binary:{}>", b.index))
        }
        Val::Dict(d) => {
            let mut out = Dict::new();
            for (k, x) in d.iter() {
                out.insert(k.clone(), to_json(x, extra));
            }
            Val::Dict(out)
        }
        Val::List(l) => Val::List(l.iter().map(|x| to_json(x, extra)).collect()),
        Val::None => Val::None,
        Val::Bool(b) => Val::Bool(*b),
        Val::Int(i) => Val::Int(*i),
        Val::Float(f) => Val::Float(*f),
        Val::Str(s) => Val::Str(s.clone()),
    }
}

struct RawResult {
    /// root key -> Binary
    root_bins: HashMap<String, Binary>,
    /// root key -> decode result (for keys whose file name ends with .tlg)
    decoded: HashMap<String, Decoded>,
}

/// Write raw/ (root binaries, PNG of each .tlg, tree JSON).
fn extract_raw(ctx: &Context, psb: &PsbFile, root: &Dict, raw_dir: &str, stem: &str) -> R<RawResult> {
    let mut root_bins: Vec<(String, String, Binary)> = Vec::new(); // (key, fname, bin)
    for (key, v) in root.iter() {
        if let Val::Bin(b) = v {
            let mut fname = safe_name(key);
            if fname.is_empty() {
                fname = format!("_bin{}.bin", b.index);
            }
            root_bins.push((key.clone(), fname, *b));
        }
    }

    let mut extra: Vec<(String, Binary)> = Vec::new();
    let mut tree = Dict::new();
    for (key, v) in root.iter() {
        match root_bins.iter().find(|(k, _, _)| k == key) {
            Some((_, fname, _)) => tree.insert(key.clone(), Val::Str(fname.clone())),
            None => tree.insert(key.clone(), to_json(v, &mut extra)),
        }
    }

    for (_, fname, b) in &root_bins {
        pypath::write_file(&join(raw_dir, fname), psb.binary(b))?;
    }
    for (name, b) in &extra {
        pypath::write_file(&join(raw_dir, name), psb.binary(b))?;
    }

    let mut f = pypath::OutFile::create(&join(raw_dir, &format!("{}.json", stem)))?;
    let (text, err) = json::dump(&tree);
    f.write(text.replace('\n', NL).as_bytes())?;
    if let Some(e) = err {
        return Err(e);
    }
    f.write(NL.as_bytes())?;
    drop(f);

    // PNG conversion of each .tlg (the original .tlg is kept as well)
    let written: HashSet<String> = root_bins
        .iter()
        .map(|(_, f, _)| f.to_lowercase())
        .chain(extra.iter().map(|(n, _)| n.to_lowercase()))
        .collect();
    let mut decoded = HashMap::new();
    for (key, fname, b) in &root_bins {
        if !fname.to_lowercase().ends_with(".tlg") {
            continue;
        }
        let d = decode_cached(psb, b)?;
        decoded.insert(key.clone(), d.clone());
        let img = match d {
            Ok(img) => img,
            Err(e) => {
                ctx.warn(&format!("{}: {}; PNG not written", fname, e));
                continue;
            }
        };
        let png = format!("{}.png", &fname[..fname.len() - 4]);
        if written.contains(&png.to_lowercase()) {
            ctx.warn(&format!("{}: \"{}\" already used by another binary; PNG not written", fname, png));
            continue;
        }
        png::write_png(&join(raw_dir, &png), img.w as i128, img.h as i128, &img.px)?;
    }

    Ok(RawResult { root_bins: root_bins.into_iter().map(|(k, _, b)| (k, b)).collect(), decoded })
}

// ---------------------------------------------------------------------------
// composite/
// ---------------------------------------------------------------------------

/// dict.get(key, default)
fn get<'a>(d: &'a Dict, key: &str, default: &'a Val) -> &'a Val {
    d.get(key).unwrap_or(default)
}

const NONE: Val = Val::None;
const ZERO: Val = Val::Int(0);

/// range(a, b) arguments must be ints.
fn range_ints(a: Num, b: Num) -> R<(i128, i128)> {
    match (a, b) {
        (Num::I(a), Num::I(b)) => Ok((a, b)),
        _ => Err(type_error("'float' object cannot be interpreted as an integer")),
    }
}

/// Normal alpha blending (ltPsNormal) of img onto canvas, clipped.
#[allow(clippy::too_many_arguments)]
fn blend(canvas: &mut [u8], cw: i128, ch: i128, img: &Image, left: &Val, top: &Val, opacity: i128) -> R<()> {
    let (iw, ih) = (img.w as i128, img.h as i128);
    let x0 = max0(left)?;
    let y0 = max0(top)?;
    let x1 = min2(Num::I(cw), add_int(left, iw)?);
    let y1 = min2(Num::I(ch), add_int(top, ih)?);
    let (y0, y1) = range_ints(y0, y1)?;
    if y0 >= y1 {
        return Ok(());
    }
    let (x0, x1) = range_ints(x0, x1)?;
    if x0 >= x1 {
        return Ok(());
    }
    let (top, left) = match (Num::of(top), Num::of(left)) {
        (Some(Num::I(t)), Some(Num::I(l))) => (t, l),
        _ => return Err(type_error("bytearray indices must be integers or slices, not float")),
    };
    let px = &img.px;
    for y in y0..y1 {
        let mut s = (((y - top) * iw + (x0 - left)) * 4) as usize;
        let mut d = ((y * cw + x0) * 4) as usize;
        for _ in x0..x1 {
            let mut a = px[s + 3] as i128;
            if opacity != 255 {
                a = a * opacity / 255;
            }
            if a == 255 {
                canvas[d..d + 3].copy_from_slice(&px[s..s + 3]);
                canvas[d + 3] = 255;
            } else if a != 0 {
                let da = canvas[d + 3] as i128 * (255 - a) / 255;
                let oa = a + da;
                for c in 0..3 {
                    canvas[d + c] = ((px[s + c] as i128 * a + canvas[d + c] as i128 * da + oa / 2) / oa) as u8;
                }
                canvas[d + 3] = oa as u8;
            }
            s += 4;
            d += 4;
        }
    }
    Ok(())
}

fn composite(ctx: &Context, psb: &PsbFile, root: &Dict, raw: &RawResult, comp_dir: &str, stem: &str) -> R<()> {
    let layers = match root.get("layers") {
        Some(Val::List(l)) if !l.is_empty() => l,
        _ => {
            ctx.warn("no \"layers\"; composite skipped");
            return Ok(());
        }
    };
    let layers: Vec<&Dict> = layers
        .iter()
        .filter_map(|l| match l {
            Val::Dict(d) => Some(d),
            _ => None,
        })
        .collect();

    // Decode layer images
    let mut images: Vec<Option<Rc<Image>>> = vec![None; layers.len()];
    let mut late: HashMap<String, Decoded> = HashMap::new();
    for (idx, l) in layers.iter().enumerate() {
        let lid = py_str(get(l, "layer_id", &NONE));
        let layer_type = get(l, "layer_type", &ZERO);
        if !eq_num(layer_type, 0) {
            ctx.warn(&format!(
                "layer {}: layer_type {} (group layer etc.) is not supported",
                lid,
                py_str(get(l, "layer_type", &NONE))
            ));
        }
        let key = format!("{}.tlg", lid);
        let Some(b) = raw.root_bins.get(&key) else {
            if eq_num(layer_type, 0) {
                ctx.warn(&format!("layer {}: image \"{}\" not found; skipped", lid, key));
            }
            continue;
        };
        let img = match raw.decoded.get(&key).or_else(|| late.get(&key)) {
            Some(d) => d.clone(),
            None => {
                let d = decode_cached(psb, b)?;
                late.insert(key.clone(), d.clone());
                d
            }
        };
        let img = match img {
            Ok(img) => img,
            Err(e) => {
                ctx.warn(&format!("layer {}: {}; skipped", lid, e));
                continue;
            }
        };
        let (width, height) = (get(l, "width", &NONE), get(l, "height", &NONE));
        if !(eq_num(width, img.w as i128) && eq_num(height, img.h as i128)) {
            ctx.warn(&format!(
                "layer {}: image size {}x{} differs from layer {}x{}",
                lid,
                img.w,
                img.h,
                py_str(width),
                py_str(height)
            ));
        }
        let lt13 = Val::Int(LT_PS_NORMAL as i64);
        if !eq_num(get(l, "type", &lt13), LT_PS_NORMAL) {
            ctx.warn(&format!("layer {}: blend type {} treated as normal", lid, py_str(get(l, "type", &NONE))));
        }
        images[idx] = Some(img);
    }

    let usable: Vec<(&Dict, Rc<Image>)> =
        layers.iter().zip(&images).filter_map(|(l, img)| img.clone().map(|i| (*l, i))).collect();
    if usable.is_empty() {
        ctx.warn("no decodable layer images; composite skipped");
        return Ok(());
    }

    // isinstance(v, int) and v > 0
    let positive_int = |v: Option<&Val>| match v {
        Some(Val::Int(i)) if *i > 0 => Some(*i as i128),
        Some(Val::Bool(true)) => Some(1),
        _ => None,
    };
    let (cw, ch) = match (positive_int(root.get("width")), positive_int(root.get("height"))) {
        (Some(w), Some(h)) => (Num::I(w), Num::I(h)),
        _ => {
            // max(l.get('left', 0) + iw for l in usable), same for top
            let fold_max = |key: &str, size: fn(&Image) -> u32| -> R<Num> {
                let mut m: Option<Num> = None;
                for (l, img) in &usable {
                    let v = add_int(get(l, key, &ZERO), size(img) as i128)?;
                    if m.is_none_or(|cur| v.gt(cur)) {
                        m = Some(v);
                    }
                }
                Ok(m.unwrap())
            };
            let cw = fold_max("left", |i| i.w)?;
            let ch = fold_max("top", |i| i.h)?;
            ctx.warn(&format!("root width/height missing; using {}x{}", pct_d(cw)?, pct_d(ch)?));
            (cw, ch)
        }
    };

    // Base: full canvas at (0,0), fully opaque; nearest to the end of layers
    let mut base = None;
    for (i, (l, img)) in usable.iter().enumerate().rev() {
        if eq_num(get(l, "left", &ZERO), 0)
            && eq_num(get(l, "top", &ZERO), 0)
            && Num::I(img.w as i128).eq(cw)
            && Num::I(img.h as i128).eq(ch)
            && img.px.iter().skip(3).step_by(4).all(|&a| a == 255)
        {
            base = Some(i);
            break;
        }
    }
    let base = match base {
        Some(b) => b,
        None => {
            let b = usable.len() - 1;
            ctx.warn(&format!(
                "no full-canvas opaque base layer; using layer {} as base",
                py_str(get(usable[b].0, "layer_id", &NONE))
            ));
            b
        }
    };

    // File names
    let empty = Val::Str(String::new());
    let raw_names: Vec<String> = usable.iter().map(|(l, _)| safe_name(&py_str(get(l, "name", &empty)))).collect();
    let names: Vec<String> = usable
        .iter()
        .zip(&raw_names)
        .map(|((l, _), n)| {
            let lid = py_str(get(l, "layer_id", &NONE));
            if n.is_empty() {
                format!("layer_{}", lid)
            } else if raw_names.iter().filter(|x| *x == n).count() > 1 {
                format!("{}_{}", n, lid)
            } else {
                n.clone()
            }
        })
        .collect();

    // bytearray(cw * ch * 4)
    let (cw, ch) = match (cw, ch) {
        (Num::I(w), Num::I(h)) => (w, h),
        _ => return Err(type_error("cannot convert 'float' object to bytearray")),
    };
    let size = cw
        .checked_mul(ch)
        .and_then(|n| n.checked_mul(4))
        .unwrap_or(i128::MAX);

    let draw = |i: usize, canvas: &mut [u8]| -> R<()> {
        let (l, img) = &usable[i];
        let opacity = match get(l, "opacity", &Val::Int(255)) {
            Val::Int(o) => *o as i128,
            Val::Bool(b) => *b as i128,
            _ => 255,
        };
        blend(canvas, cw, ch, img, get(l, "left", &ZERO), get(l, "top", &ZERO), opacity.clamp(0, 255))
    };

    let mut base_canvas = alloc_zeroed(size)?;
    draw(base, &mut base_canvas)?;
    let base_name = &names[base];
    png::write_png(&join(comp_dir, &format!("{}-{}.png", stem, base_name)), cw, ch, &base_canvas)?;

    for (i, name) in names.iter().enumerate() {
        if i == base {
            continue;
        }
        let mut canvas = base_canvas.clone();
        draw(i, &mut canvas)?;
        png::write_png(&join(comp_dir, &format!("{}-{}+{}.png", stem, base_name, name)), cw, ch, &canvas)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Processing
// ---------------------------------------------------------------------------

/// Ok(false) = this input failed; Err = error that escapes process()
/// (OSError is reported by main, anything else is fatal).
fn process(path: &str, args: &Args) -> R<bool> {
    let ctx = Context { label: pypath::basename(path), quiet: args.quiet };
    let stem = pypath::splitext(&pypath::basename(path)).0;
    let parent = match args.output.as_deref() {
        Some(o) if !o.is_empty() => o.to_string(),
        _ => pypath::dirname(&pypath::abspath(path)?),
    };
    let out_dir = join(&parent, &stem);

    if pypath::exists(&out_dir) && !args.force {
        ctx.warn(&format!("output \"{}\" already exists; skipped (use -f to overwrite)", out_dir));
        return Ok(true);
    }

    let parsed = pypath::read_file(path).and_then(|data| {
        let psb = PsbFile::new(data)?;
        let root = psb.root()?;
        Ok((psb, root))
    });
    let (psb, root) = match parsed {
        Ok((psb, Val::Dict(root))) => (psb, root),
        Ok(_) => unreachable!("the root is always a dict"),
        Err(e @ PyErr::Fatal { .. }) => return Err(e),
        Err(e) => {
            eprint_line(&format!("{}: error: {}: {}", PROG, ctx.label, e.text()));
            return Ok(false);
        }
    };

    let raw_dir = join(&out_dir, "raw");
    pypath::makedirs(&raw_dir, true)?;
    let raw = extract_raw(&ctx, &psb, &root, &raw_dir, &stem)?;

    if !args.no_composite {
        let comp_dir = join(&out_dir, "composite");
        pypath::makedirs(&comp_dir, true)?;
        composite(&ctx, &psb, &root, &raw, &comp_dir, &stem)?;
    }
    Ok(true)
}

fn collect_inputs(paths: &[String]) -> R<Vec<String>> {
    let mut files = Vec::new();
    for p in paths {
        if pypath::isdir(p) {
            let mut found: Vec<String> = pypath::listdir(p)?
                .into_iter()
                .filter(|n| n.to_lowercase().ends_with(".pimg") && pypath::isfile(&join(p, n)))
                .map(|n| join(p, &n))
                .collect();
            found.sort();
            files.extend(found);
        } else {
            files.push(p.clone());
        }
    }
    Ok(files)
}

/// The Python version dies with a traceback here; report it and stop.
fn fatal(label: Option<&str>, e: &PyErr) -> i32 {
    let (kind, msg) = match e {
        PyErr::Fatal { kind, msg } => (*kind, msg.clone()),
        PyErr::Os { errno, .. } => (
            match errno {
                Some(1) | Some(13) => "PermissionError",
                Some(2) => "FileNotFoundError",
                Some(17) => "FileExistsError",
                Some(20) => "NotADirectoryError",
                Some(21) => "IsADirectoryError",
                _ => "OSError",
            },
            e.text(),
        ),
        _ => ("Error", e.text()),
    };
    let what = if msg.is_empty() { kind.to_string() } else { format!("{}: {}", kind, msg) };
    match label {
        Some(l) => eprint_line(&format!("{}: fatal: {}: {}", PROG, l, what)),
        None => eprint_line(&format!("{}: fatal: {}", PROG, what)),
    }
    1
}

fn real_main() -> i32 {
    let argv: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let args = args::parse(argv);

    let files = match collect_inputs(&args.inputs) {
        Ok(f) => f,
        Err(e) => return fatal(None, &e),
    };
    let mut ok = true;
    for path in &files {
        match process(path, &args) {
            Ok(r) => ok = r && ok,
            Err(e @ PyErr::Os { .. }) => {
                eprint_line(&format!("{}: error: {}: {}", PROG, pypath::basename(path), e.text()));
                ok = false;
            }
            Err(e) => return fatal(Some(&pypath::basename(path)), &e),
        }
    }
    if ok {
        0
    } else {
        1
    }
}

fn main() {
    // Deeply nested PSB trees recurse up to Python's limit (1000 frames).
    let code = std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(real_main)
        .expect("failed to start main thread")
        .join()
        .unwrap_or(1);
    std::process::exit(code);
}
