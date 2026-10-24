#!/usr/bin/env python3
"""Convert the owl video into a compact boot asset (MITOSV format 1).

    scripts/make-owl-asset.py [SRC_VIDEO] [OUT.mitosv] [--crop W:H:X:Y] [--hold N]

Needs: ffmpeg on PATH, python3, numpy.  (Dev/CI tool only -- nothing here
ships in the initramfs.)

Why this exists
---------------
Raw MITOSV is RGBA8 (~600 MB for this clip) and the FFmpeg backend drags
libavcodec & friends into early userspace.  Format 1 is the opposite end:

  * crop to the owl's face band (the clip is 85% black margin; this also
    drops the baked-in watermark rows below the eyes),
  * one shared 256-colour palette (k-means, ordered by luma),
  * "closed-loop hold": a pixel only changes when it differs from what the
    decoder already shows by more than --hold (default 6/255), which wipes
    out H.264 shimmer but keeps every real motion (the blink),
  * each frame = per-pixel wrapping index delta vs the previous frame,
    zlib-compressed (the decoder reuses flate2, already pulled in by `png`).

Decoder footprint: two w*h byte planes + one frame of u32 pixels.
The script finishes by decoding its own output exactly as src/video/decoder.rs
does and refuses to write a file that does not round-trip bit-exactly.
"""
import argparse
import struct
import subprocess
import sys
import zlib

import numpy as np

MAGIC = b"MITOSV1\0"
FORMAT_PAL8_ZDELTA = 1
CRUSH = 4  # values at or below this (max channel) become pure black


def probe_fps(src):
    out = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
         "stream=r_frame_rate", "-of", "default=nw=1:nk=1", src],
        capture_output=True, text=True, check=True).stdout.strip()
    num, den = out.split("/")
    return max(1, min(240, round(int(num) / int(den))))


def load_frames(src, crop):
    w, h, x, y = crop
    cmd = ["ffmpeg", "-v", "error", "-i", src, "-vf", f"crop={w}:{h}:{x}:{y}",
           "-f", "rawvideo", "-pix_fmt", "rgb24", "-"]
    raw = subprocess.run(cmd, capture_output=True, check=True).stdout
    fr = np.frombuffer(raw, np.uint8).reshape(-1, h, w, 3).copy()
    fr[fr.max(axis=3) <= CRUSH] = 0
    return fr


def pack(rgb):
    return (rgb[..., 0].astype(np.uint32) << 16) | (rgb[..., 1].astype(np.uint32) << 8) | rgb[..., 2]


def nearest(colors, pal):
    """Index of the nearest palette entry for every row of `colors` (float32 Nx3)."""
    out = np.empty(len(colors), np.uint8)
    p2 = (pal * pal).sum(axis=1)
    for i in range(0, len(colors), 8192):
        c = colors[i:i + 8192]
        d = (c * c).sum(axis=1)[:, None] - 2.0 * c @ pal.T + p2[None, :]
        out[i:i + 8192] = d.argmin(axis=1)
    return out


def median_cut(colors, weights, n):
    boxes = [np.arange(len(colors))]
    while len(boxes) < n:
        # split the box with the largest weighted extent
        best, best_score = None, -1.0
        for bi, ix in enumerate(boxes):
            if len(ix) < 2:
                continue
            span = np.ptp(colors[ix], axis=0).max()
            score = span * weights[ix].sum()
            if score > best_score:
                best, best_score = bi, score
        if best is None:
            break
        ix = boxes.pop(best)
        ch = int(np.ptp(colors[ix], axis=0).argmax())
        ix = ix[np.argsort(colors[ix, ch], kind="stable")]
        cum = np.cumsum(weights[ix])
        cut = int(np.searchsorted(cum, cum[-1] / 2.0))
        cut = min(max(cut, 1), len(ix) - 1)
        boxes.extend([ix[:cut], ix[cut:]])
    return np.array([np.average(colors[ix], axis=0, weights=weights[ix]) for ix in boxes],
                    np.float32)


def build_palette(frames):
    u, cnt = np.unique(pack(frames), return_counts=True)
    colors = np.stack([(u >> 16) & 255, (u >> 8) & 255, u & 255], axis=1).astype(np.float32)
    # sqrt(count): rare bright colours (iris, highlights) still get their own entries
    weights = np.sqrt(cnt.astype(np.float64))
    keep = u != 0
    pal = median_cut(colors[keep], weights[keep], 255)
    for _ in range(8):  # weighted Lloyd refinement
        assign = nearest(colors[keep], pal)
        for k in range(len(pal)):
            m = assign == k
            if m.any():
                pal[k] = np.average(colors[keep][m], axis=0, weights=weights[keep][m])
    pal = np.vstack([np.zeros((1, 3), np.float32), pal]).round().clip(0, 255).astype(np.uint8)
    luma = pal[:, 0] * 0.299 + pal[:, 1] * 0.587 + pal[:, 2] * 0.114
    pal = pal[np.argsort(luma, kind="stable")]  # exact black stays at index 0
    return pal, u, colors


def encode(frames, hold):
    n, h, w, _ = frames.shape
    pal, u, colors = build_palette(frames)
    lut = nearest(colors, pal.astype(np.float32))      # unique colour -> palette index

    def to_index(rgb):
        return lut[np.searchsorted(u, pack(rgb))].reshape(h, w)

    held = np.zeros((h, w, 3), np.int16)               # what the decoder currently shows
    prev = np.zeros((h, w), np.uint8)
    planes, chunks = [], []
    for t in range(n):
        src = frames[t].astype(np.int16)
        upd = np.abs(src - held).max(axis=2) > hold
        held = np.where(upd[..., None], src, held)
        idx = to_index(held.astype(np.uint8))
        delta = (idx.astype(np.int16) - prev.astype(np.int16)).astype(np.uint8)  # wraps mod 256
        raw = delta.tobytes()
        best = None
        for strat in (zlib.Z_DEFAULT_STRATEGY, zlib.Z_FILTERED, zlib.Z_RLE):
            c = zlib.compressobj(9, zlib.DEFLATED, 15, 9, strat)
            z = c.compress(raw) + c.flush()
            if best is None or len(z) < len(best):
                best = z
        chunks.append(best)
        planes.append(idx)
        prev = idx
    return pal, np.stack(planes), chunks


def write_asset(path, w, h, fps, pal, chunks):
    with open(path, "wb") as f:
        f.write(MAGIC)
        f.write(struct.pack("<5I", w, h, fps, len(chunks), FORMAT_PAL8_ZDELTA))
        f.write(struct.pack("<H", len(pal)))
        f.write(pal.tobytes())
        for z in chunks:
            f.write(struct.pack("<I", len(z)))
            f.write(z)


def decode_asset(path):
    """Mirror of MitosvDecoder (src/video/decoder.rs) for format 1."""
    data = open(path, "rb").read()
    assert data[:8] == MAGIC
    w, h, fps, frames, fmt = struct.unpack_from("<5I", data, 8)
    assert fmt == FORMAT_PAL8_ZDELTA
    off = 28
    (npal,) = struct.unpack_from("<H", data, off)
    off += 2
    pal = np.frombuffer(data, np.uint8, npal * 3, off).reshape(npal, 3)
    off += npal * 3
    full = np.zeros((256, 3), np.uint8)
    full[:npal] = pal
    idx = np.zeros(w * h, np.uint8)
    out = []
    for _ in range(frames):
        (zlen,) = struct.unpack_from("<I", data, off)
        off += 4
        delta = np.frombuffer(zlib.decompress(data[off:off + zlen]), np.uint8)
        off += zlen
        assert len(delta) == w * h
        idx = (idx + delta).astype(np.uint8)
        out.append(idx.reshape(h, w).copy())
    assert off == len(data), "trailing bytes"
    return np.stack(out), full, (w, h, fps, frames)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("src", nargs="?", default="assets/splash/owl/owl.mp4")
    ap.add_argument("out", nargs="?", default="assets/splash/owl/owl.mitosv")
    ap.add_argument("--crop", default="704:432:16:408", help="W:H:X:Y of the face band in the source")
    ap.add_argument("--hold", type=int, default=6, help="closed-loop hold threshold (0-255)")
    a = ap.parse_args()

    crop = tuple(int(v) for v in a.crop.split(":"))
    fps = probe_fps(a.src)
    frames = load_frames(a.src, crop)
    n, h, w, _ = frames.shape
    print(f"source: {n} frames @ {fps} fps, cropped to {w}x{h}")

    pal, planes, chunks = encode(frames, a.hold)
    write_asset(a.out, w, h, fps, pal, chunks)

    dec, dpal, hdr = decode_asset(a.out)
    if not np.array_equal(dec, planes):
        sys.exit("error: decoded indices differ from encoder output")
    rec = dpal[dec].astype(np.float64)
    mse = np.mean((rec - frames.astype(np.float64)) ** 2)
    size = len(open(a.out, "rb").read())
    print(f"wrote {a.out}: {size:,} bytes ({size / n:,.0f} B/frame), "
          f"{len(pal)} colours, PSNR vs source {10 * np.log10(255 ** 2 / mse):.2f} dB")
    print(f"decoder memory: {2 * w * h / 1024:.0f} KiB index planes + {w * h * 4 / 1024:.0f} KiB pixels per frame")


if __name__ == "__main__":
    main()
