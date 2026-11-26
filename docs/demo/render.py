#!/usr/bin/env python3
"""Draws the frames that record.sh captured (tmux capture-pane -e) as a GIF, as a terminal would: a cell grid, the
colors of each cell, and box lines, braille and blocks drawn by hand so that they join as in a real terminal.

    python3 render.py <frames dir> <out.gif>

<frames dir>/list.tsv has one line per frame: file, milliseconds, caption ("keys|what they do", or empty).
Needs Pillow."""
import os
import re
import sys

from PIL import Image, ImageDraw, ImageFont

CELL_W, CELL_H, PAD, BAR = 8, 17, 14, 30
BG, FG = (15, 17, 20), (232, 234, 237)
MUTED, LINE, KEY_BG = (154, 161, 169), (74, 82, 92), (42, 47, 54)
# the signs of the agents, in the colors of orcux's theme
SIGNS = {"●": (138, 184, 245), "?": (224, 176, 96), "■": (163, 211, 156), "!": (240, 138, 120)}
FONTS = [
    ("/System/Library/Fonts/Menlo.ttc", 0, 1),
    ("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf", 0, None),
]
FALLBACK = ["/System/Library/Fonts/Apple Symbols.ttf", "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"]
SIZE = 13.3


def xterm(n):
    base = [(0, 0, 0), (205, 49, 49), (13, 188, 121), (229, 229, 16), (36, 114, 200), (188, 63, 188), (17, 168, 205),
            (229, 229, 229), (102, 102, 102), (241, 76, 76), (35, 209, 139), (245, 245, 67), (59, 142, 234),
            (214, 112, 214), (41, 184, 219), (229, 229, 229)]
    if n < 16:
        return base[n]
    if n < 232:
        n -= 16
        steps = [0, 95, 135, 175, 215, 255]
        return steps[n // 36], steps[n // 6 % 6], steps[n % 6]
    v = 8 + (n - 232) * 10
    return v, v, v


class Style:
    def __init__(self):
        self.reset()

    def reset(self):
        self.fg = self.bg = None
        self.bold = self.dim = self.italic = self.under = self.rev = False

    def copy(self):
        s = Style()
        s.__dict__.update(self.__dict__)
        return s

    def sgr(self, params):
        p = [int(x) if x else 0 for x in re.split("[;:]", params)] if params else [0]
        i = 0
        while i < len(p):
            c = p[i]
            if c == 0:
                self.reset()
            elif c == 1:
                self.bold = True
            elif c == 2:
                self.dim = True
            elif c == 3:
                self.italic = True
            elif c == 4:
                self.under = True
            elif c == 7:
                self.rev = True
            elif c == 22:
                self.bold = self.dim = False
            elif c == 23:
                self.italic = False
            elif c == 24:
                self.under = False
            elif c == 27:
                self.rev = False
            elif 30 <= c <= 37:
                self.fg = xterm(c - 30)
            elif 90 <= c <= 97:
                self.fg = xterm(c - 90 + 8)
            elif 40 <= c <= 47:
                self.bg = xterm(c - 40)
            elif 100 <= c <= 107:
                self.bg = xterm(c - 100 + 8)
            elif c == 39:
                self.fg = None
            elif c == 49:
                self.bg = None
            elif c in (38, 48) and i + 1 < len(p):
                if p[i + 1] == 5 and i + 2 < len(p):
                    col, i = xterm(p[i + 2]), i + 2
                elif p[i + 1] == 2 and i + 4 < len(p):
                    col, i = tuple(p[i + 2:i + 5]), i + 4
                else:
                    col = None
                if c == 38:
                    self.fg = col
                else:
                    self.bg = col
            i += 1


def parse(text):
    """Rows of (char, style). The style goes on from row to row, as tmux writes it."""
    rows, st = [], Style()
    for line in text.split("\n"):
        row, i = [], 0
        while i < len(line):
            if line[i] == "\x1b":
                m = re.match(r"\x1b\[([0-9;:]*)([A-Za-z])", line[i:])
                if m:
                    if m.group(2) == "m":
                        st.sgr(m.group(1))
                    i += m.end()
                    continue
                m = re.match(r"\x1b\][^\x07\x1b]*(\x07|\x1b\\)", line[i:])
                i += m.end() if m else 1
                continue
            row.append((line[i], st.copy()))
            i += 1
        rows.append(row)
    return rows


def blend(a, b, t):
    return tuple(int(a[k] + (b[k] - a[k]) * t) for k in range(3))


class Glyphs:
    def __init__(self):
        for path, regular, bold in FONTS:
            if os.path.exists(path):
                self.regular = ImageFont.truetype(path, SIZE, index=regular)
                self.bold = ImageFont.truetype(path, SIZE, index=bold) if bold is not None else self.regular
                break
        else:
            sys.exit("no monospace font found: edit FONTS in render.py")
        self.fallback = [ImageFont.truetype(p, SIZE) for p in FALLBACK if os.path.exists(p)]
        self.cache = {}

    def missing(self, font, ch):
        def mask(c):
            m = font.getmask(c)
            return m.size, bytes(m)
        return mask(ch) == mask("\U0010fffd")

    def font(self, ch, bold):
        key = (ch, bold)
        if key not in self.cache:
            first = self.bold if bold else self.regular
            found = first
            if self.missing(first, ch):
                found = next((f for f in self.fallback if not self.missing(f, ch)), first)
            self.cache[key] = found
        return self.cache[key]


# box drawing: which arms a character has (left, right, up, down), and rounded corners
ARMS = {
    "─": "lr", "│": "ud", "┌": "rd", "┐": "ld", "└": "ru", "┘": "lu", "├": "rud", "┤": "lud", "┬": "lrd",
    "┴": "lru", "┼": "lrud", "╭": "rd", "╮": "ld", "╰": "ru", "╯": "lu", "╴": "l", "╶": "r", "╵": "u", "╷": "d",
}
ROUND = set("╭╮╰╯")


def box(d, x, y, ch, col):
    cx, cy = x + CELL_W // 2, y + CELL_H // 2
    arms = ARMS[ch]
    if ch in ROUND:
        r = CELL_W // 2
        sx = 1 if "r" in arms else -1
        sy = 1 if "d" in arms else -1
        # the arc from the vertical arm to the horizontal one, through the cell's center
        ox, oy = cx + sx * r, cy + sy * r
        bbox = [ox - r, oy - r, ox + r, oy + r]
        start = {(1, 1): 180, (-1, 1): 270, (1, -1): 90, (-1, -1): 0}[(sx, sy)]
        d.arc(bbox, start, start + 90, fill=col, width=1)
        if sx == 1:
            d.line([cx + r, cy, x + CELL_W - 1, cy], fill=col)
        else:
            d.line([x, cy, cx - r, cy], fill=col)
        if sy == 1:
            d.line([cx, cy + r, cx, y + CELL_H - 1], fill=col)
        else:
            d.line([cx, y, cx, cy - r], fill=col)
        return
    if "l" in arms:
        d.line([x, cy, cx, cy], fill=col)
    if "r" in arms:
        d.line([cx, cy, x + CELL_W - 1, cy], fill=col)
    if "u" in arms:
        d.line([cx, y, cx, cy], fill=col)
    if "d" in arms:
        d.line([cx, cy, cx, y + CELL_H - 1], fill=col)


def braille(d, x, y, ch, col):
    bits = ord(ch) - 0x2800
    order = [(0, 0), (0, 1), (0, 2), (1, 0), (1, 1), (1, 2), (0, 3), (1, 3)]
    for i, (c, r) in enumerate(order):
        if bits & (1 << i):
            px = x + 1 + c * 4
            py = y + 2 + r * 4
            d.rectangle([px, py, px + 1, py + 1], fill=col)


def block(d, x, y, ch, col):
    o = ord(ch)
    if o == 0x2580:
        d.rectangle([x, y, x + CELL_W - 1, y + CELL_H // 2 - 1], fill=col)
    elif 0x2581 <= o <= 0x2588:  # lower eighths up to full
        h = round(CELL_H * (o - 0x2580) / 8)
        d.rectangle([x, y + CELL_H - h, x + CELL_W - 1, y + CELL_H - 1], fill=col)
    elif 0x2589 <= o <= 0x258F:  # left blocks, from seven eighths to one
        w = max(1, round(CELL_W * (0x2590 - o) / 8))
        d.rectangle([x, y, x + w - 1, y + CELL_H - 1], fill=col)
    elif o == 0x2590:
        d.rectangle([x + CELL_W // 2, y, x + CELL_W - 1, y + CELL_H - 1], fill=col)


def draw(rows, caption, glyphs, cols, nrows):
    w = cols * CELL_W + 2 * PAD
    h = nrows * CELL_H + 2 * PAD + BAR
    img = Image.new("RGB", (w, h), BG)
    d = ImageDraw.Draw(img)
    for r, row in enumerate(rows[:nrows]):
        y = PAD + r * CELL_H
        for c, (ch, st) in enumerate(row[:cols]):
            x = PAD + c * CELL_W
            fg, bg = st.fg or FG, st.bg or BG
            if st.rev:
                fg, bg = bg, fg
            if st.dim:
                fg = blend(fg, bg, 0.45)
            if bg != BG:
                d.rectangle([x, y, x + CELL_W - 1, y + CELL_H - 1], fill=bg)
            if ch == " ":
                continue
            o = ord(ch)
            if ch in ARMS:
                box(d, x, y, ch, fg)
            elif 0x2800 <= o <= 0x28FF:
                braille(d, x, y, ch, fg)
            elif 0x2580 <= o <= 0x2590:
                block(d, x, y, ch, fg)
            else:
                f = glyphs.font(ch, st.bold)
                d.text((x + CELL_W / 2, y + CELL_H / 2 + 1), ch, font=f, fill=fg, anchor="mm")
            if st.under:
                d.line([x, y + CELL_H - 2, x + CELL_W - 1, y + CELL_H - 2], fill=fg)
    if caption:
        keys, _, what = caption.partition("|")
        y = PAD + nrows * CELL_H + PAD // 2 + 2
        x = PAD
        for k in [k for k in keys.split(" ") if k]:
            tw = max(len(k), 1) * CELL_W + 10
            d.rounded_rectangle([x, y, x + tw, y + CELL_H + 4], radius=4, fill=KEY_BG, outline=LINE)
            for i, ch in enumerate(k):
                d.text((x + 5 + i * CELL_W + CELL_W / 2, y + 2 + CELL_H / 2 + 1), ch, font=glyphs.font(ch, True), fill=FG,
                       anchor="mm")
            x += tw + 6
        for i, ch in enumerate(what):
            if ch != " ":
                d.text((x + 6 + i * CELL_W + CELL_W / 2, y + 2 + CELL_H / 2 + 1), ch, font=glyphs.font(ch, False),
                       fill=SIGNS.get(ch, MUTED), anchor="mm")
    return img


def main():
    src, out = sys.argv[1], sys.argv[2]
    glyphs = Glyphs()
    frames = []
    with open(os.path.join(src, "list.tsv"), encoding="utf-8") as f:
        for line in f:
            name, ms, caption = (line.rstrip("\n").split("\t") + ["", ""])[:3]
            with open(os.path.join(src, name), encoding="utf-8") as g:
                text = g.read().rstrip("\n")
            frames.append((parse(text), int(ms), caption))
    cols = max(len(r) for rows, _, _ in frames for r in rows)
    nrows = max(len(rows) for rows, _, _ in frames)
    images, times = [], []
    for rows, ms, caption in frames:
        img = draw(rows, caption, glyphs, cols, nrows)
        if images and list(img.getdata()) == list(images[-1].getdata()):
            times[-1] += ms
            continue
        images.append(img)
        times.append(ms)
    # one palette for every frame, so that only what changes is written again
    sheet = Image.new("RGB", (images[0].width, images[0].height * len(images)))
    for i, im in enumerate(images):
        sheet.paste(im, (0, i * im.height))
    palette = sheet.quantize(colors=int(os.environ.get("COLORS", "96")), method=Image.Quantize.MEDIANCUT)
    gif = [im.quantize(palette=palette, dither=Image.Dither.NONE) for im in images]
    if os.environ.get("PNGS"):
        os.makedirs(os.environ["PNGS"], exist_ok=True)
        for i, im in enumerate(images):
            im.save(os.path.join(os.environ["PNGS"], "%03d.png" % i))
    gif[0].save(out, save_all=True, append_images=gif[1:], duration=times, loop=0, optimize=False, disposal=1)
    print("%s: %d frames, %.1f s, %d KB" % (out, len(gif), sum(times) / 1000, os.path.getsize(out) // 1024))


if __name__ == "__main__":
    main()
