"""Compose promotional heroes that embed the REAL app UI at its TRUE size.

The dashboard is a compact always-on-top widget, NOT a wide dashboard:
src-tauri/tauri.conf.json declares windows[0] as 400 x 680, transparent,
borderless, alwaysOnTop, not resizable (aspect 0.588, portrait).

Screenshot path: assets/generated/ui-harness.html puts the real app in an
iframe of exactly 400x680 and applies `zoom: 2`, so Chromium re-rasterises at
2x instead of stretching a 1x bitmap. Output is therefore an 800x1360 capture
that IS a 400x680 window, crisp, with zero upscaling in this script.

Nothing in the panel is drawn by a model. The backdrop is procedural: its only
job is to sit behind the widget, and atmospheric art would put its own focal
point exactly where the widget belongs.

Usage:
    python compose-ui-hero.py wide       # 16:9  2752x1536
    python compose-ui-hero.py ultrawide  # 21:9  3840x1646
"""
import os
import sys
import numpy as np
from PIL import Image, ImageDraw, ImageFilter, ImageFont

HERE = os.path.dirname(os.path.abspath(__file__))
MODE = sys.argv[1] if len(sys.argv) > 1 else "wide"

# --- design tokens (docs/FRONTEND-DESIGN-STYLE.md) ---
ACCENT = (76, 194, 255)        # #4cc2ff
TEXT_PRIMARY = (230, 232, 234)  # #e6e8ea
TEXT_SECOND = (179, 185, 192)   # #b3b9c0
EYEBROW_C = (156, 165, 174)
FOOTER_C = (152, 161, 170)

F_SANS = r"C:\Windows\Fonts\segoeuib.ttf"
F_CJK = r"C:\Windows\Fonts\msyh.ttc"
F_MONO = r"C:\Windows\Fonts\CascadiaCode.ttf"

WIDGET = HERE + r"\ui-widget-2x.png"      # 800x1360 == 400x680 @2x
ART_B = HERE + r"\token-usage-monitor-hero-02.jpg"   # model-generated data landscape

ART = len(sys.argv) > 1 and sys.argv[1].endswith("-artB")
MODE = sys.argv[1].replace("-artB", "") if len(sys.argv) > 1 else "wide"

OUT = {"wide": HERE + r"\token-usage-monitor-hero-ui.png",
       "ultrawide": HERE + r"\token-usage-monitor-hero-ui-21x9.png"}[MODE]
if ART:
    OUT = OUT.replace(".png", "-artB.png")

if MODE == "ultrawide":
    W, H = 3840, 1646
    X, MAXW = (280, 900) if ART else (280, 1120)
    WX, WY = (1900, 143) if ART else (2800, 143)
    DETAIL_W = 1230
    DETAIL = None if ART else (40, 278, 762, 700)
else:
    W, H = 2752, 1536
    X, MAXW = (200, 900) if ART else (280, 1300)
    # with art B, sit the widget left of the light columns so that feature survives
    WX, WY = (1180, 88) if ART else (1672, 88)
    DETAIL = None


# ---------------------------------------------------------------- backdrop
def make_backdrop(W, H, glows):
    yy, xx = np.mgrid[0:H, 0:W]
    g = (yy / H)[..., None]
    img = (np.array([9, 10, 13], np.float32) * (1 - g)
           + np.array([15, 17, 23], np.float32) * g)

    for cx, cy, rx, ry, strength in glows:
        rad = np.sqrt(((xx - cx) / rx) ** 2 + ((yy - cy) / ry) ** 2)
        m = (np.clip(1.0 - rad, 0.0, 1.0) ** 2.2 * strength)[..., None]
        img += np.array(ACCENT, np.float32)[None, None, :] * m

    step = 56
    img[step // 2:H:step, step // 2:W:step] += 4.0
    img[step // 2 - 1:H:step, step // 2:W:step] += 2.0
    img[step // 2:H:step, step // 2 - 1:W:step] += 2.0

    # clip BEFORE the fractional power: a negative base raised to 1.5 yields
    # NaN, which silently poisons those pixels instead of darkening them
    rad = np.sqrt(((xx - W / 2) / (W * 0.66)) ** 2 + ((yy - H / 2) / (H * 0.78)) ** 2)
    vig = np.clip(rad - 0.50, 0.0, None) ** 1.5
    img *= (1.0 - np.clip(vig, 0.0, 1.0) * 0.92)[..., None]

    return Image.fromarray(np.clip(img, 0, 255).astype(np.uint8)).convert("RGBA")


def paste_panel(bg, shot, x, y, radius=28, glow=0.5, shadow=0.72):
    w, h = shot.size

    gl = Image.new("L", (w, h), 0)
    ImageDraw.Draw(gl).rounded_rectangle([0, 0, w - 1, h - 1], radius=radius, fill=110)
    gcol = Image.new("RGBA", (w, h), ACCENT + (0,))
    gcol.putalpha(gl.filter(ImageFilter.GaussianBlur(56)).point(lambda v: int(v * glow)))
    bg.alpha_composite(gcol, (x, y))

    pad = 70
    sh = Image.new("L", (w + pad * 2, h + pad * 2), 0)
    ImageDraw.Draw(sh).rounded_rectangle(
        [pad, pad, pad + w - 1, pad + h - 1], radius=radius, fill=200)
    scol = Image.new("RGBA", (w + pad * 2, h + pad * 2), (0, 0, 0, 0))
    scol.putalpha(sh.filter(ImageFilter.GaussianBlur(26)).point(lambda v: int(v * shadow)))
    bg.alpha_composite(scol, (x - pad, y - pad + 20))

    mask = Image.new("L", (w, h), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, w - 1, h - 1], radius=radius, fill=255)
    panel = shot.convert("RGBA")
    panel.putalpha(mask)
    bg.alpha_composite(panel, (x, y))

    ImageDraw.Draw(bg).rounded_rectangle(
        [x, y, x + w - 1, y + h - 1], radius=radius, outline=(255, 255, 255, 38), width=2)


widget = Image.open(WIDGET)
WW, WH = widget.size


def make_art_bg(W, H):
    """Model-generated data landscape, framed to the target ratio.

    Art B is 2752x1536. For a wider target we take a native-width 21:9 band
    (keeping the plane and the light columns) rather than stretching; the
    remaining upscale is harmless because the art contains no lettering.
    """
    art = Image.open(ART_B).convert("RGB")
    aw, ah = art.size
    if W / H > aw / ah:
        ch = aw / (W / H)
        top = int((ah - ch) * 0.62)          # bias down: keep the glowing plane
        art = art.crop((0, top, aw, top + int(ch)))
    else:
        cw = ah * (W / H)
        left = int((aw - cw) * 0.30)
        art = art.crop((left, 0, left + int(cw), ah))
    art = art.resize((W, H), Image.LANCZOS)
    a = np.asarray(art).astype(np.float32)

    # guarantee copy contrast over the art: darken the copy column only, and
    # keep it transparent by 0.42W so the plane still reads through
    yy, xx = np.mgrid[0:H, 0:W]
    scrim = (np.clip(1.0 - xx / (W * 0.42), 0.0, 1.0) ** 1.7) * 0.72
    foot = np.clip((yy - H * 0.84) / (H * 0.16), 0.0, 1.0) ** 1.5 * 0.42
    k = np.clip(scrim + foot, 0.0, 1.0)[..., None]
    a = a * (1.0 - k) + np.array([8, 9, 12], np.float32) * k
    return Image.fromarray(np.clip(a, 0, 255).astype(np.uint8)).convert("RGBA")


if ART:
    canvas = make_art_bg(W, H)
else:
    glows = [(WX + WW / 2, WY + WH / 2, WW * 1.5, WH * 0.85, 0.42)]
    canvas = make_backdrop(W, H, glows)

if DETAIL:
    det = widget.crop(DETAIL)
    dw = DETAIL_W
    dh = int(det.height * dw / det.width)
    det = det.resize((dw, dh), Image.LANCZOS)
    dx, dy = 1470, (H - dh) // 2
    glows = [(WX + WW / 2, WY + WH / 2, WW * 1.5, WH * 0.85, 0.42),
             (dx + dw / 2, dy + dh / 2, dw * 0.8, dh * 1.1, 0.16)]
    canvas = make_backdrop(W, H, glows)   # rebuild so the new glow is included
    paste_panel(canvas, det, dx, dy, radius=22, glow=0.30, shadow=0.62)

paste_panel(canvas, widget, WX, WY)

layer = Image.new("RGBA", (W, H), (0, 0, 0, 0))
d = ImageDraw.Draw(layer)


def fit(path, text, max_w, start, floor=10):
    size = start
    while size > floor:
        f = ImageFont.truetype(path, size)
        if f.getlength(text) <= max_w:
            return f
        size -= 2
    return ImageFont.truetype(path, floor)


def spaced(draw, xy, text, font, fill, tracking):
    x, y = xy
    for ch in text:
        draw.text((x, y), ch, font=font, fill=fill)
        x += font.getlength(ch) + tracking
    return x - tracking


# ---------------------------------------------------------------- copy block
cursor = (H - 470) // 2

ef = ImageFont.truetype(F_MONO, 28)
d.ellipse([X, cursor + 11, X + 13, cursor + 24], fill=ACCENT + (235,))
spaced(d, (X + 33, cursor), "AI TOKEN USAGE MONITOR", ef, EYEBROW_C + (255,), 3.6)
cursor += 62

t_head, t_tail = "TokenUsage", "Monitor"
hf = fit(F_SANS, t_head + t_tail, MAXW, 138, 60)
asc, desc = hf.getmetrics()
baseline = cursor + asc
d.text((X, baseline), t_head, font=hf, fill=TEXT_PRIMARY + (255,), anchor="ls")
d.text((X + hf.getlength(t_head), baseline), t_tail, font=hf, fill=ACCENT + (255,), anchor="ls")
cursor = baseline - asc + (asc + desc) + 36

bar_w, bar_h = int(W * 0.150), 5
rule = np.zeros((bar_h, bar_w, 4), np.float32)
for i in range(bar_w):
    rule[0, i] = (ACCENT[0], ACCENT[1], ACCENT[2], 255 * (1.0 - (i / bar_w) ** 2.2))
layer.alpha_composite(Image.fromarray(rule.astype(np.uint8)), (X, cursor))
cursor += bar_h + 48

tag = "多 Provider 聚合的 AI Token 用量透明看板"
tf = fit(F_CJK, tag, MAXW + 40, 52, 28)
d.text((X, cursor), tag, font=tf, fill=TEXT_SECOND + (255,))
cursor += tf.getbbox(tag)[3] + 52

cf = ImageFont.truetype(F_CJK, 30)
cx = X
for c in ["用量热力图", "透明置顶常驻", "安装包约 10MB"]:
    w = cf.getlength(c) + 58
    d.rounded_rectangle([cx, cursor, cx + w, cursor + 62], radius=31,
                        fill=(255, 255, 255, 16), outline=(255, 255, 255, 48), width=2)
    d.text((cx + 29, cursor + 31), c, font=cf, fill=TEXT_SECOND + (255,), anchor="lm")
    cx += w + 16

mf = ImageFont.truetype(F_MONO, 27)
spaced(d, (X, int(H * 0.905)), "TAURI 2  ·  SVELTE 5  ·  RUST  ·  WINDOWS",
       mf, FOOTER_C + (255,), 2.3)

out = Image.alpha_composite(canvas, layer).convert("RGB")
out.save(OUT, "PNG", optimize=True)
print(f"saved {OUT}  {out.size}  mode={MODE}  widget={WW}x{WH} (=400x680 @2x) @({WX},{WY})")
