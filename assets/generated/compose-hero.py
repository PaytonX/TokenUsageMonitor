"""Composite the promotional title layer onto a generated hero image.

Usage:
    python compose-hero.py <src> <out> [layout]

layout = "left"  (default) copy block sits mid-left, for art whose subject is a
                  glass panel on the right (hero-01)
        = "upper" copy block sits in the upper-left quadrant, for art whose
                  plane/mass rises from mid-frame (hero-02)

Design tokens are taken verbatim from docs/FRONTEND-DESIGN-STYLE.md so the
banner matches the app's own Fluent Glass surface.
"""
import os
import sys
import numpy as np
from PIL import Image, ImageDraw, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = sys.argv[1] if len(sys.argv) > 1 else HERE + r"\token-usage-monitor-hero-01.jpg"
OUT = sys.argv[2] if len(sys.argv) > 2 else HERE + r"\token-usage-monitor-hero.png"
LAYOUT = sys.argv[3] if len(sys.argv) > 3 else "left"

# --- design tokens (docs/FRONTEND-DESIGN-STYLE.md) ---
ACCENT = (76, 194, 255)        # #4cc2ff
TEXT_PRIMARY = (230, 232, 234)  # #e6e8ea
TEXT_SECOND = (179, 185, 192)   # #b3b9c0
INK = (10, 11, 13)              # near-black scrim base
# small mono copy sits on a lifted glow, so it needs more lightness than the
# raw muted token to clear 4.5:1 at these sizes
EYEBROW_C = (156, 165, 174)
FOOTER_C = (152, 161, 170)

F_SANS = r"C:\Windows\Fonts\segoeuib.ttf"
F_CJK = r"C:\Windows\Fonts\msyh.ttc"
F_MONO = r"C:\Windows\Fonts\CascadiaCode.ttf"

# per-layout placement
if LAYOUT == "upper":
    CURSOR_Y, GLOW_CY, BOTTOM_SCrim = 0.120, 0.30, 0.10
else:
    CURSOR_Y, GLOW_CY, BOTTOM_SCrim = 0.315, 0.44, 0.30

base = Image.open(SRC).convert("RGB")
W, H = base.size
arr = np.asarray(base).astype(np.float32)

# --- 1. soft accent glow so the copy belongs to the same light source ---
yy, xx = np.mgrid[0:H, 0:W]
cx, cy = W * 0.28, H * GLOW_CY
rad = np.sqrt(((xx - cx) / (W * 0.40)) ** 2 + ((yy - cy) / (H * 0.58)) ** 2)
glow = np.clip(1.0 - rad, 0.0, 1.0) ** 2 * 0.13
for i, c in enumerate(ACCENT):
    arr[:, :, i] = 255.0 - (255.0 - arr[:, :, i]) * (1.0 - glow)

# --- 2. left scrim: must fall off BEFORE the focal subject starts (~x=0.45W),
#        otherwise it desaturates the element it is meant to support.
#        NOTE: strongest at x=0, zero by x=0.45W. Inverting this sign crushes
#        the art and leaves the copy unprotected. ---
t = np.clip(1.0 - xx / (W * 0.45), 0.0, 1.0)
scrim = (t ** 1.8) * 0.78
for i in range(3):
    arr[:, :, i] = arr[:, :, i] * (1.0 - scrim) + INK[i] * scrim

# subtle bottom scrim for the footer line, kept clear of the subject
if BOTTOM_SCrim > 0:
    bot = np.clip((yy - H * 0.86) / (H * 0.14), 0.0, 1.0) ** 1.4 * BOTTOM_SCrim
    for i in range(3):
        arr[:, :, i] = arr[:, :, i] * (1.0 - bot) + INK[i] * bot

img = Image.fromarray(np.clip(arr, 0, 255).astype(np.uint8)).convert("RGBA")
layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
d = ImageDraw.Draw(layer)


def fit(path, text, max_w, start, floor=10):
    """Shrink until the string fits max_w, so copy never collides with the art."""
    size = start
    while size > floor:
        f = ImageFont.truetype(path, size)
        if f.getlength(text) <= max_w:
            return f
        size -= 2
    return ImageFont.truetype(path, floor)


def spaced(draw, xy, text, font, fill, tracking):
    """Letter-spaced run; Pillow has no tracking, so advance per glyph."""
    x, y = xy
    for ch in text:
        draw.text((x, y), ch, font=font, fill=fill)
        x += font.getlength(ch) + tracking
    return x - tracking


X = int(W * 0.098)          # left margin
MAXW = int(W * 0.335)       # keep type clear of the subject
cursor = int(H * CURSOR_Y)

# eyebrow: accent dot + mono kicker
eyebrow = "AI TOKEN USAGE MONITOR"
ef = ImageFont.truetype(F_MONO, 27)
d.ellipse([X, cursor + 11, X + 13, cursor + 24], fill=ACCENT + (235,))
spaced(d, (X + 32, cursor), eyebrow, ef, EYEBROW_C + (255,), 3.4)
cursor += 58

# wordmark: two-tone so it echoes the cyan progress rings
t_head, t_tail = "TokenUsage", "Monitor"
hf = fit(F_SANS, t_head + t_tail, MAXW, 122, 60)
w1 = hf.getlength(t_head)
asc, desc = hf.getmetrics()
baseline = cursor + asc
d.text((X, baseline), t_head, font=hf, fill=TEXT_PRIMARY + (255,), anchor="ls")
d.text((X + w1, baseline), t_tail, font=hf, fill=ACCENT + (255,), anchor="ls")
cursor = baseline - asc + (asc + desc) + 34

# accent rule with a soft falloff
bar_w, bar_h = int(W * 0.150), 5
rule = np.zeros((bar_h, bar_w, 4), dtype=np.float32)
for x in range(bar_w):
    a = 1.0 - (x / bar_w) ** 2.2
    rule[0, x] = (ACCENT[0], ACCENT[1], ACCENT[2], 255 * a)
layer.alpha_composite(Image.fromarray(rule.astype(np.uint8)), (X, cursor))
cursor += bar_h + 46

# Chinese tagline
tag = "多 Provider 聚合的 AI Token 用量透明看板"
tf = fit(F_CJK, tag, MAXW + 60, 50, 30)
d.text((X, cursor), tag, font=tf, fill=TEXT_SECOND + (255,))
cursor += tf.getbbox(tag)[3] + 52

# feature chips
chips = ["用量热力图", "透明置顶常驻", "安装包约 10MB"]
cf = ImageFont.truetype(F_CJK, 29)
cx = X
for c in chips:
    tw = cf.getlength(c)
    w = tw + 58
    d.rounded_rectangle([cx, cursor, cx + w, cursor + 60], radius=30,
                        fill=(255, 255, 255, 16), outline=(255, 255, 255, 48), width=2)
    d.text((cx + 29, cursor + 30), c, font=cf, fill=TEXT_SECOND + (255,), anchor="lm")
    cx += w + 16

# footer tech line, bottom-left
mf = ImageFont.truetype(F_MONO, 26)
spaced(d, (X, int(H * 0.905)), "TAURI 2  ·  SVELTE 5  ·  RUST  ·  WINDOWS",
       mf, FOOTER_C + (255,), 2.2)

out = Image.alpha_composite(img, layer).convert("RGB")
out.save(OUT, "PNG", optimize=True)
print(f"saved {OUT} {out.size} layout={LAYOUT} block_bottom_y={cursor + 60}")
