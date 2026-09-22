#!/usr/bin/env python3
"""pimgext - .pimg (PSB layered image) extractor.

Extracts the raw contents of a .pimg file and composites "base + one diff"
PNG images. See SPEC.md for the full specification.

Usage: pimgext.py [-o DIR] [--no-composite] [-f] [-q] INPUT...
"""

import argparse
import json
import os
import re
import struct
import sys
import zlib

PROG = 'pimgext'

PSB_TYPE_DICT = 0x21
LT_PS_NORMAL = 13  # tTVPLayerType::ltPsNormal (krkrz/visual/drawable.h)


class PimgError(Exception):
    """The input cannot be processed at all."""


# ---------------------------------------------------------------------------
# PSB reader (port of KrkrExtract.Core/PsbWorker.cpp, offsets as 32bit ints)
# ---------------------------------------------------------------------------

def unpack_mdf(data):
    if len(data) <= 10:
        raise PimgError('mdf: file too small')
    size = struct.unpack_from('<I', data, 4)[0]
    try:
        out = zlib.decompress(data[8:])
    except zlib.error as e:
        raise PimgError('mdf: zlib decompression failed (%s)' % e)
    if len(out) != size:
        raise PimgError('mdf: size mismatch (header %d, actual %d)' % (size, len(out)))
    return out


class PsbArray:
    def __init__(self, data, p):
        t = data[p]
        if not 13 <= t <= 16:
            raise PimgError('psb: invalid array header 0x%02x at 0x%x' % (t, p))
        l = t - 11
        self.data = data
        self.count = int.from_bytes(data[p + 1:p + l], 'little')
        self.size = data[p + l] - 12
        if not 1 <= self.size <= 4:
            raise PimgError('psb: invalid array element size at 0x%x' % p)
        self.code = p + l + 1
        self.nbytes = self.count * self.size + l + 1

    def __getitem__(self, i):
        o = self.code + i * self.size
        return int.from_bytes(self.data[o:o + self.size], 'little')

    def __len__(self):
        return self.count


class Binary:
    """A binary value in the PSB tree (bytes are sliced lazily)."""

    def __init__(self, index, offset, size):
        self.index = index
        self.offset = offset
        self.size = size


class PsbFile:
    def __init__(self, data):
        if data[:4] == b'mdf\0':
            data = unpack_mdf(data)
        if len(data) < 64 or data[:4] != b'PSB\0':
            raise PimgError('not a PSB/PIMG file')

        self.data = data
        self.version, self.flag = struct.unpack_from('<HH', data, 4)
        if self.version not in (2, 3):
            raise PimgError('unsupported PSB version %d' % self.version)

        (_, self.p_str_index, self.p_str_offset, self.p_str_pool,
         self.p_bin_offset, self.p_bin_size, self.p_bin_pool,
         self.p_root) = struct.unpack_from('<8I', data, 8)

        if (self.flag & 3) or self.p_root >= len(data) or data[self.p_root] != PSB_TYPE_DICT:
            raise PimgError('encrypted PSB is not supported')

        p = self.p_str_index
        self.name_a1 = PsbArray(data, p)
        p += self.name_a1.nbytes
        self.name_a2 = PsbArray(data, p)
        p += self.name_a2.nbytes
        self.name_a3 = PsbArray(data, p)
        self.str_offsets = PsbArray(data, self.p_str_offset)
        self.bin_offsets = PsbArray(data, self.p_bin_offset)
        self.bin_sizes = PsbArray(data, self.p_bin_size)

    def name(self, idx):
        a1, a2 = self.name_a1, self.name_a2
        out = bytearray()
        c1 = a2[self.name_a3[idx]]
        while c1:
            c2 = a2[c1]
            out.append((c1 - a1[c2]) & 0xFF)
            c1 = c2
        out.reverse()
        return out.decode('utf-8', 'replace')

    def string(self, idx):
        s = self.p_str_pool + self.str_offsets[idx]
        e = self.data.index(b'\0', s)
        return self.data[s:e].decode('utf-8', 'replace')

    def binary(self, b):
        return self.data[b.offset:b.offset + b.size]

    def root(self):
        return self.value(self.p_root)

    def value(self, p):
        data = self.data
        t = data[p]
        if t in (0, 1):
            return None
        if t == 2:
            return True
        if t == 3:
            return False
        if t == 4:
            return 0
        if 5 <= t <= 12:
            return int.from_bytes(data[p + 1:p + t - 3], 'little', signed=True)
        if t == 29:
            return 0.0
        if t == 30:
            return struct.unpack_from('<f', data, p + 1)[0]
        if t == 31:
            return struct.unpack_from('<d', data, p + 1)[0]
        if 21 <= t <= 24:
            return self.string(int.from_bytes(data[p + 1:p + t - 19], 'little'))
        if 25 <= t <= 28:
            i = int.from_bytes(data[p + 1:p + t - 23], 'little')
            return Binary(i, self.p_bin_pool + self.bin_offsets[i], self.bin_sizes[i])
        if t == 0x20:
            a = PsbArray(data, p + 1)
            base = p + 1 + a.nbytes
            return [self.value(base + a[i]) for i in range(len(a))]
        if t == PSB_TYPE_DICT:
            names = PsbArray(data, p + 1)
            offsets = PsbArray(data, p + 1 + names.nbytes)
            base = p + 1 + names.nbytes + offsets.nbytes
            return {self.name(names[i]): self.value(base + offsets[i]) for i in range(len(names))}
        raise PimgError('psb: unknown value type 0x%02x at 0x%x' % (t, p))


# ---------------------------------------------------------------------------
# TLG5 decoder -> RGBA bytearray
# ---------------------------------------------------------------------------

TLG5_MAGIC = b'TLG5.0\0raw\x1a'


def decode_tlg5(buf):
    if buf[:11] != TLG5_MAGIC:
        raise PimgError('not a TLG5 image')
    colors, w, h, bh = struct.unpack_from('<BIII', buf, 11)
    if colors not in (3, 4) or bh == 0:
        raise PimgError('unsupported TLG5 (colors=%d)' % colors)

    nblk = (h + bh - 1) // bh
    p = 24 + 4 * nblk
    text = bytearray(4096)
    r = 0
    stride = w * 4
    out = bytearray(stride * h)
    prev = bytearray(stride)

    for by in range(0, h, bh):
        rows = min(bh, h - by)
        chans = []
        for _ in range(colors):
            mark = buf[p]
            size = struct.unpack_from('<I', buf, p + 1)[0]
            p += 5
            src = buf[p:p + size]
            p += size
            if mark != 0:
                chans.append(bytearray(src))
                continue
            o = bytearray()
            i = 0
            flags = 0
            while i < size:
                flags >>= 1
                if not flags & 0x100:
                    flags = src[i] | 0xFF00
                    i += 1
                if flags & 1:
                    mpos = src[i] | ((src[i + 1] & 0x0F) << 8)
                    mlen = (src[i + 1] >> 4) + 3
                    i += 2
                    if mlen == 18:
                        mlen += src[i]
                        i += 1
                    for _ in range(mlen):
                        v = text[mpos]
                        o.append(v)
                        text[r] = v
                        r = (r + 1) & 0xFFF
                        mpos = (mpos + 1) & 0xFFF
                else:
                    v = src[i]
                    i += 1
                    o.append(v)
                    text[r] = v
                    r = (r + 1) & 0xFFF
            chans.append(o)

        if len(chans[0]) < rows * w:
            raise PimgError('TLG5: truncated block')
        cb, cg, cr = chans[0], chans[1], chans[2]
        ca = chans[3] if colors == 4 else None
        for yy in range(rows):
            base = yy * w
            pb = pg = pr = pa = 0
            row = bytearray(stride)
            for x in range(w):
                g = cg[base + x]
                pb = (pb + cb[base + x] + g) & 255
                pg = (pg + g) & 255
                pr = (pr + cr[base + x] + g) & 255
                if ca is not None:
                    pa = (pa + ca[base + x]) & 255
                k = x * 4
                row[k] = (prev[k] + pr) & 255
                row[k + 1] = (prev[k + 1] + pg) & 255
                row[k + 2] = (prev[k + 2] + pb) & 255
                row[k + 3] = (prev[k + 3] + pa) & 255
            if ca is None:
                row[3::4] = b'\xff' * w
            y = by + yy
            out[y * stride:(y + 1) * stride] = row
            prev = row
    return w, h, out


# ---------------------------------------------------------------------------
# Output helpers
# ---------------------------------------------------------------------------

def write_png(path, w, h, rgba):
    stride = w * 4
    raw = b''.join(b'\0' + bytes(rgba[y * stride:(y + 1) * stride]) for y in range(h))

    def chunk(tag, body):
        return (struct.pack('>I', len(body)) + tag + body
                + struct.pack('>I', zlib.crc32(tag + body) & 0xFFFFFFFF))

    with open(path, 'wb') as f:
        f.write(b'\x89PNG\r\n\x1a\n')
        f.write(chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)))
        f.write(chunk(b'IDAT', zlib.compress(raw, 6)))
        f.write(chunk(b'IEND', b''))


def safe_name(name):
    name = re.sub(r'[\\/:*?"<>|\x00-\x1f]', '_', str(name)).strip()
    return '' if name in ('', '.', '..') else name


def blend(canvas, cw, ch, img, iw, ih, left, top, opacity):
    """Normal alpha blending (ltPsNormal) of img onto canvas, clipped."""
    x0, y0 = max(0, left), max(0, top)
    x1, y1 = min(cw, left + iw), min(ch, top + ih)
    for y in range(y0, y1):
        s = ((y - top) * iw + (x0 - left)) * 4
        d = (y * cw + x0) * 4
        for _ in range(x0, x1):
            a = img[s + 3]
            if opacity != 255:
                a = a * opacity // 255
            if a == 255:
                canvas[d:d + 3] = img[s:s + 3]
                canvas[d + 3] = 255
            elif a:
                da = canvas[d + 3] * (255 - a) // 255
                oa = a + da
                for c in range(3):
                    canvas[d + c] = (img[s + c] * a + canvas[d + c] * da + oa // 2) // oa
                canvas[d + 3] = oa
            s += 4
            d += 4


# ---------------------------------------------------------------------------
# Processing
# ---------------------------------------------------------------------------

class Context:
    def __init__(self, path, quiet):
        self.label = os.path.basename(path)
        self.quiet = quiet

    def warn(self, msg):
        if not self.quiet:
            print('%s: warning: %s: %s' % (PROG, self.label, msg), file=sys.stderr)


def decode_image(psb, b):
    """Decode a TLG binary to (w, h, rgba). Raises PimgError if not possible."""
    data = psb.binary(b)
    if data[:11] != TLG5_MAGIC:
        raise PimgError('not a TLG5 image (%r)' % bytes(data[:6]))
    try:
        return decode_tlg5(data)
    except (IndexError, struct.error) as e:
        raise PimgError('TLG5 decode failed (%s)' % e)


def extract_raw(ctx, psb, root, raw_dir, stem):
    """Write raw/ (root binaries, PNG of each .tlg, tree JSON).

    Returns ({key: Binary}, {key: (w, h, rgba) | PimgError}) for root binaries;
    the second dict holds the TLG decode results so composite can reuse them.
    """
    root_bins = {}
    extra_bins = {}

    for key, v in root.items():
        if isinstance(v, Binary):
            fname = safe_name(key) or '_bin%d.bin' % v.index
            root_bins[key] = (fname, v)

    def to_json(v):
        if isinstance(v, Binary):
            name = '_bin%d.bin' % v.index
            extra_bins[name] = v
            return '<binary:%d>' % v.index
        if isinstance(v, dict):
            return {k: to_json(x) for k, x in v.items()}
        if isinstance(v, list):
            return [to_json(x) for x in v]
        return v

    tree = {}
    for key, v in root.items():
        tree[key] = root_bins[key][0] if key in root_bins else to_json(v)

    for fname, b in list(root_bins.values()) + [(n, b) for n, b in extra_bins.items()]:
        with open(os.path.join(raw_dir, fname), 'wb') as f:
            f.write(psb.binary(b))
    with open(os.path.join(raw_dir, stem + '.json'), 'w', encoding='utf-8') as f:
        json.dump(tree, f, ensure_ascii=False, indent=2)
        f.write('\n')

    # PNG conversion of each .tlg (the original .tlg is kept as well)
    written = {fname.lower() for fname, _ in root_bins.values()} | {n.lower() for n in extra_bins}
    decoded = {}
    for key, (fname, b) in root_bins.items():
        if not fname.lower().endswith('.tlg'):
            continue
        try:
            decoded[key] = decode_image(psb, b)
        except PimgError as e:
            decoded[key] = e
            ctx.warn('%s: %s; PNG not written' % (fname, e))
            continue
        png = fname[:-4] + '.png'
        if png.lower() in written:
            ctx.warn('%s: "%s" already used by another binary; PNG not written' % (fname, png))
            continue
        w, h, px = decoded[key]
        write_png(os.path.join(raw_dir, png), w, h, px)
    return {k: b for k, (_, b) in root_bins.items()}, decoded


def composite(ctx, psb, root, root_bins, decoded, comp_dir, stem):
    layers = root.get('layers')
    if not isinstance(layers, list) or not layers:
        ctx.warn('no "layers"; composite skipped')
        return
    layers = [l for l in layers if isinstance(l, dict)]

    # Decode layer images
    images = {}
    for l in layers:
        lid = l.get('layer_id')
        if l.get('layer_type', 0) != 0:
            ctx.warn('layer %s: layer_type %s (group layer etc.) is not supported'
                     % (lid, l.get('layer_type')))
        key = '%s.tlg' % lid
        b = root_bins.get(key)
        if b is None:
            if l.get('layer_type', 0) == 0:
                ctx.warn('layer %s: image "%s" not found; skipped' % (lid, key))
            continue
        img = decoded.get(key)
        if img is None:
            try:
                img = decode_image(psb, b)
            except PimgError as e:
                img = e
        if isinstance(img, PimgError):
            ctx.warn('layer %s: %s; skipped' % (lid, img))
            continue
        iw, ih, px = img
        if (iw, ih) != (l.get('width'), l.get('height')):
            ctx.warn('layer %s: image size %dx%d differs from layer %sx%s'
                     % (lid, iw, ih, l.get('width'), l.get('height')))
        if l.get('type', LT_PS_NORMAL) != LT_PS_NORMAL:
            ctx.warn('layer %s: blend type %s treated as normal' % (lid, l.get('type')))
        images[id(l)] = (iw, ih, px)

    usable = [l for l in layers if id(l) in images]
    if not usable:
        ctx.warn('no decodable layer images; composite skipped')
        return

    cw, ch = root.get('width'), root.get('height')
    if not isinstance(cw, int) or not isinstance(ch, int) or cw <= 0 or ch <= 0:
        cw = max(l.get('left', 0) + images[id(l)][0] for l in usable)
        ch = max(l.get('top', 0) + images[id(l)][1] for l in usable)
        ctx.warn('root width/height missing; using %dx%d' % (cw, ch))

    # Base: full canvas at (0,0), fully opaque; nearest to the end of layers
    base = None
    for l in reversed(usable):
        iw, ih, px = images[id(l)]
        if (l.get('left', 0), l.get('top', 0), iw, ih) == (0, 0, cw, ch) \
                and px[3::4].count(255) == iw * ih:
            base = l
            break
    if base is None:
        base = usable[-1]
        ctx.warn('no full-canvas opaque base layer; using layer %s as base'
                 % base.get('layer_id'))

    # File names
    raw_names = [safe_name(l.get('name', '')) for l in usable]
    names = {}
    for l, n in zip(usable, raw_names):
        lid = l.get('layer_id')
        if not n:
            names[id(l)] = 'layer_%s' % lid
        elif raw_names.count(n) > 1:
            names[id(l)] = '%s_%s' % (n, lid)
        else:
            names[id(l)] = n

    def draw(layer, canvas):
        iw, ih, px = images[id(layer)]
        opacity = layer.get('opacity', 255)
        if not isinstance(opacity, int):
            opacity = 255
        blend(canvas, cw, ch, px, iw, ih, layer.get('left', 0), layer.get('top', 0),
              max(0, min(255, opacity)))

    base_canvas = bytearray(cw * ch * 4)
    draw(base, base_canvas)
    base_name = names[id(base)]
    write_png(os.path.join(comp_dir, '%s-%s.png' % (stem, base_name)), cw, ch, base_canvas)

    for l in usable:
        if l is base:
            continue
        canvas = bytearray(base_canvas)
        draw(l, canvas)
        write_png(os.path.join(comp_dir, '%s-%s+%s.png' % (stem, base_name, names[id(l)])),
                  cw, ch, canvas)


def process(path, out_parent, args):
    ctx = Context(path, args.quiet)
    stem = os.path.splitext(os.path.basename(path))[0]
    out_dir = os.path.join(out_parent or os.path.dirname(os.path.abspath(path)), stem)

    if os.path.exists(out_dir) and not args.force:
        ctx.warn('output "%s" already exists; skipped (use -f to overwrite)' % out_dir)
        return True

    try:
        with open(path, 'rb') as f:
            data = f.read()
        psb = PsbFile(data)
        root = psb.root()
    except OSError as e:
        print('%s: error: %s: %s' % (PROG, ctx.label, e), file=sys.stderr)
        return False
    except (PimgError, IndexError, ValueError, struct.error) as e:
        print('%s: error: %s: %s' % (PROG, ctx.label, e), file=sys.stderr)
        return False

    raw_dir = os.path.join(out_dir, 'raw')
    os.makedirs(raw_dir, exist_ok=True)
    root_bins, decoded = extract_raw(ctx, psb, root, raw_dir, stem)

    if not args.no_composite:
        comp_dir = os.path.join(out_dir, 'composite')
        os.makedirs(comp_dir, exist_ok=True)
        composite(ctx, psb, root, root_bins, decoded, comp_dir, stem)
    return True


def collect_inputs(paths):
    files = []
    for p in paths:
        if os.path.isdir(p):
            files += sorted(os.path.join(p, n) for n in os.listdir(p)
                            if n.lower().endswith('.pimg') and os.path.isfile(os.path.join(p, n)))
        else:
            files.append(p)
    return files


def main(argv=None):
    ap = argparse.ArgumentParser(prog=PROG, description='Extract and composite .pimg files.')
    ap.add_argument('inputs', nargs='+', metavar='INPUT', help='.pimg file or directory')
    ap.add_argument('-o', '--output', metavar='DIR', help='parent directory for output')
    ap.add_argument('--no-composite', action='store_true', help='write raw/ only')
    ap.add_argument('-f', '--force', action='store_true', help='overwrite existing output')
    ap.add_argument('-q', '--quiet', action='store_true', help='suppress warnings')
    args = ap.parse_args(argv)

    ok = True
    for path in collect_inputs(args.inputs):
        try:
            ok = process(path, args.output, args) and ok
        except OSError as e:
            print('%s: error: %s: %s' % (PROG, os.path.basename(path), e), file=sys.stderr)
            ok = False
    return 0 if ok else 1


if __name__ == '__main__':
    sys.exit(main())
