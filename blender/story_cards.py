"""CDN brand-kit story visuals: headline cards, lower thirds, and the brand loop.

Runs with the system Python (needs Pillow; the loop needs FFmpeg), not inside Blender:
    py story_cards.py card "Studio Ghibli opens a restoration lab" [--kicker "ANIMATION"] [--out PATH]
    py story_cards.py lower-third "Elena" "CDN News" [--out PATH]
    py story_cards.py loop            # builds the CDN brand loop used when a story has no visual

Colors, fonts (Inter) and sizes come from slide_templates in studio_manifest.json.
"""
import hashlib
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
if HERE not in sys.path:
    sys.path.insert(0, HERE)

import studio_api as studio


def _pil():
    from PIL import Image, ImageDraw, ImageFont
    return Image, ImageDraw, ImageFont


def _rgb(hex_color):
    hex_color = hex_color.lstrip("#")
    return tuple(int(hex_color[i:i + 2], 16) for i in (0, 2, 4))


def _font(manifest, key, size):
    _, _, ImageFont = _pil()
    path = studio.resolve(manifest["slide_templates"]["fonts"][key], manifest)
    try:
        return ImageFont.truetype(path, size)
    except OSError:
        return ImageFont.truetype("segoeuib.ttf" if key == "headline" else "segoeui.ttf", size)


def _wrap(draw, text, font, width):
    words, lines, current = text.split(), [], ""
    for word in words:
        trial = (current + " " + word).strip()
        if draw.textlength(trial, font=font) <= width or not current:
            current = trial
        else:
            lines.append(current)
            current = word
    if current:
        lines.append(current)
    return lines


def _out_path(manifest, prefix, text, out):
    if out:
        return out
    folder = studio.resolve(manifest["slide_templates"]["output_dir"], manifest)
    os.makedirs(folder, exist_ok=True)
    digest = hashlib.sha1(text.encode("utf-8")).hexdigest()[:10]
    return os.path.join(folder, "{}_{}.png".format(prefix, digest))


def headline_card(title, kicker=None, out=None, manifest=None):
    """Midnight card with a Signal Cyan kicker, Inter Bold headline, and the reverse CDN icon."""
    Image, ImageDraw, _ = _pil()
    manifest = manifest or studio.load_manifest()
    spec = manifest["slide_templates"]
    colors = {k: _rgb(v) for k, v in spec["palette"].items()}
    width, height = spec["size"]
    card = Image.new("RGB", (width, height), colors["midnight"])
    draw = ImageDraw.Draw(card)
    draw.rectangle([0, int(height * 0.72), width, height], fill=colors["ink"])
    draw.rectangle([120, 300, 136, 300 + 420], fill=colors["signal_cyan"])
    kicker_font = _font(manifest, "kicker", 52)
    draw.text((180, 296), (kicker or spec["kicker"]).upper(), font=kicker_font, fill=colors["signal_cyan"])
    size = 112
    while size > 64:
        font = _font(manifest, "headline", size)
        lines = _wrap(draw, title, font, width - 360)
        if len(lines) <= 3:
            break
        size -= 8
    y = 380
    for line in lines[:3]:
        draw.text((180, y), line, font=font, fill=colors["white"])
        y += int(size * 1.18)
    draw.rectangle([180, y + 24, 420, y + 32], fill=colors["signal_cyan"])
    logo_path = studio.resolve(spec["logo"], manifest)
    if os.path.isfile(logo_path):
        logo = Image.open(logo_path).convert("RGBA")
        logo.thumbnail((180, 180))
        card.paste(logo, (width - 120 - logo.width, 110), logo)
    path = _out_path(manifest, "card", title, out)
    card.save(path)
    return path


def lower_third(name, subtitle="", out=None, manifest=None):
    """Transparent 1920x1080 lower third in brand colors (for monitors or compositing)."""
    Image, ImageDraw, _ = _pil()
    manifest = manifest or studio.load_manifest()
    spec = manifest["slide_templates"]
    colors = {k: _rgb(v) for k, v in spec["palette"].items()}
    width, height = spec["size"]
    image = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    top = int(height * 0.74)
    draw.rectangle([96, top, 1180, top + 150], fill=colors["midnight"] + (235,))
    draw.rectangle([96, top, 112, top + 150], fill=colors["signal_cyan"] + (255,))
    draw.text((144, top + 18), name, font=_font(manifest, "headline", 66), fill=colors["white"] + (255,))
    if subtitle:
        draw.text((144, top + 96), subtitle.upper(), font=_font(manifest, "kicker", 34), fill=colors["signal_cyan"] + (255,))
    path = _out_path(manifest, "lower_third", name + subtitle, out)
    image.save(path)
    return path


def brand_loop(out=None, seconds=8, manifest=None):
    """Seamless CDN brand loop (gentle breathing zoom on the standby card) for monitors without a story visual."""
    manifest = manifest or studio.load_manifest()
    source = studio.resolve(manifest["story_visuals"]["fallback_still"], manifest)
    out = out or studio.resolve(manifest["story_visuals"]["fallback"], manifest)
    os.makedirs(os.path.dirname(out), exist_ok=True)
    fps = manifest["render"]["fps"]
    frames = seconds * fps
    zoom = "zoompan=z='1.02+0.02*sin(2*PI*on/{})':x='iw/2-(iw/zoom/2)':y='ih/2-(ih/zoom/2)':d=1:s=1920x1080:fps={}".format(frames, fps)
    subprocess.run(["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-loop", "1", "-i", source,
                    "-vf", "scale=3840:-2," + zoom, "-frames:v", str(frames),
                    "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "18", out], check=True)
    return out


def main(argv):
    def option(flag):
        return argv[argv.index(flag) + 1] if flag in argv else None
    if not argv or argv[0] not in ("card", "lower-third", "loop"):
        raise SystemExit(__doc__)
    if argv[0] == "card":
        print(headline_card(argv[1], kicker=option("--kicker"), out=option("--out")))
    elif argv[0] == "lower-third":
        print(lower_third(argv[1], argv[2] if len(argv) > 2 and not argv[2].startswith("--") else "", out=option("--out")))
    else:
        print(brand_loop(out=option("--out")))


if __name__ == "__main__":
    main(sys.argv[1:])
