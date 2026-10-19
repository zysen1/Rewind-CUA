#!/usr/bin/env python3
"""Draw the Rewind-CUA storyboard into an animated GIF.

Reads the JSON written by `rewind demo` and renders one frame per beat.
Requires Pillow. The Chinese captions use WenQuanYi Micro Hei when it is
installed, and fall back to DejaVu Sans.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

W, H = 960, 540
BG = (12, 16, 24)
PANEL = (22, 28, 38)
INK = (232, 236, 244)
MUTED = (154, 164, 178)
TONES = {
    "neutral": (122, 162, 255),
    "checkpoint": (245, 193, 108),
    "bad": (255, 92, 122),
    "rewind": (110, 210, 255),
    "ask": (196, 160, 255),
    "policy": (122, 162, 255),
    "ok": (61, 220, 151),
}


def font(size: int) -> ImageFont.FreeTypeFont:
    for path in (
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ):
        if Path(path).exists():
            return ImageFont.truetype(path, size)
    return ImageFont.load_default()


def wrap(draw: ImageDraw.ImageDraw, text: str, face: ImageFont.ImageFont, width: int) -> list[str]:
    lines: list[str] = []
    for paragraph in text.split("\n"):
        current = ""
        for ch in paragraph:
            trial = current + ch
            if draw.textlength(trial, font=face) <= width:
                current = trial
            else:
                if current:
                    lines.append(current)
                current = ch
        if current:
            lines.append(current)
    return lines or [""]


def render(frame: dict, index: int, total: int, ghosts: list[str]) -> Image.Image:
    image = Image.new("RGB", (W, H), BG)
    draw = ImageDraw.Draw(image)
    tone = TONES.get(frame.get("tone", "neutral"), TONES["neutral"])
    title = font(28)
    body = font(20)
    small = font(16)
    mono = font(18)

    draw.rectangle((0, 0, W, 8), fill=tone)
    draw.text((32, 24), "REWIND-CUA", font=small, fill=tone)
    draw.text((170, 24), "Built on trycua/cua", font=small, fill=MUTED)
    kicker = frame.get("kicker", "")
    draw.text((W - 32 - draw.textlength(kicker, font=small), 24), kicker, font=small, fill=INK)

    draw.rounded_rectangle((32, 64, 420, 470), radius=16, fill=PANEL)
    draw.text((52, 80), "workspace", font=small, fill=MUTED)
    y = 116
    files = list(frame.get("files") or [])
    if not files and not ghosts:
        draw.text((52, y), "(no files yet)", font=body, fill=MUTED)
    for name in files:
        draw.rounded_rectangle((52, y, 76, y + 24), radius=4, fill=tone)
        draw.text((88, y), name, font=mono, fill=INK)
        y += 36
    for name in ghosts:
        draw.rounded_rectangle((52, y, 76, y + 24), radius=4, fill=(90, 36, 48))
        draw.text((88, y), name, font=mono, fill=TONES["bad"])
        length = draw.textlength(name, font=mono)
        draw.line((88, y + 12, 88 + length, y + 12), fill=TONES["bad"], width=2)
        y += 36
    outbox = list(frame.get("outbox") or [])
    if outbox:
        draw.text((52, 400), "outbox  " + ", ".join(outbox), font=small, fill=TONES["bad"])

    draw.rounded_rectangle((444, 64, 928, 470), radius=16, fill=PANEL)
    draw.text((468, 84), frame.get("title_en", ""), font=title, fill=INK)
    y = 132
    for line in wrap(draw, frame.get("detail_en", ""), body, 420):
        draw.text((468, y), line, font=body, fill=MUTED)
        y += 28
    y += 12
    draw.text((468, y), frame.get("title_zh", ""), font=title, fill=tone)
    y += 46
    for line in wrap(draw, frame.get("detail_zh", ""), body, 420):
        draw.text((468, y), line, font=body, fill=INK)
        y += 28

    draw.text((32, 492), f"{index + 1} / {total}", font=small, fill=MUTED)
    gap = 8
    dots = total
    width = dots * 14
    x0 = W - 32 - width
    for i in range(dots):
        color = tone if i == index else (48, 56, 68)
        draw.ellipse((x0 + i * 14, 496, x0 + i * 14 + 8, 504), fill=color)
    return image


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: render_gif.py STORYBOARD.json OUT.gif", file=sys.stderr)
        return 2
    payload = json.loads(Path(sys.argv[1]).read_text(encoding="utf-8"))
    frames_in = payload["frames"]
    images: list[Image.Image] = []
    previous: list[str] = []
    for index, frame in enumerate(frames_in):
        files = list(frame.get("files") or [])
        ghosts = [name for name in previous if name not in files] if previous else []
        images.append(render(frame, index, len(frames_in), ghosts))
        if files:
            previous = files
    durations = []
    for index, frame in enumerate(frames_in):
        if index in (0, len(frames_in) - 1) or frame.get("tone") in {"rewind", "bad", "ok"}:
            durations.append(1100)
        else:
            durations.append(700)
    images[0].save(
        sys.argv[2],
        save_all=True,
        append_images=images[1:],
        duration=durations,
        loop=0,
        disposal=2,
    )
    print(f"wrote {sys.argv[2]} ({len(images)} frames)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
