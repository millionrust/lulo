#!/usr/bin/env python3
"""Build crates/rmac-emoji/assets/emoji.tsv from the Unicode emoji data.

Inputs (both Unicode-3.0 licensed, fetched by hand, never at build time):
  emoji-test.txt   https://unicode.org/Public/emoji/latest/emoji-test.txt
  en.xml           https://raw.githubusercontent.com/unicode-org/cldr/main/common/annotations/en.xml

Each output line is TAB separated:
  emoji, category, name, keywords (" | " separated), skin-tone variants (space separated)

Only fully-qualified sequences are kept. Skin-tone forms of an emoji become the
"variants" of its base form instead of lines of their own. Categories are the
Mac picker's: People, Nature, Food, Activity, Travel, Objects, Symbols, Flags.

Usage: generate_emoji_data.py emoji-test.txt en.xml > crates/rmac-emoji/assets/emoji.tsv
"""
import re
import sys
import xml.etree.ElementTree as ET

CATEGORY = {
    "Smileys & Emotion": "People",
    "People & Body": "People",
    "Animals & Nature": "Nature",
    "Food & Drink": "Food",
    "Activities": "Activity",
    "Travel & Places": "Travel",
    "Objects": "Objects",
    "Symbols": "Symbols",
    "Flags": "Flags",
}
TONES = {"\U0001F3FB", "\U0001F3FC", "\U0001F3FD", "\U0001F3FE", "\U0001F3FF"}
VS16 = "️"


def keywords(path):
    found = {}
    names = {}
    for node in ET.parse(path).getroot().iter("annotation"):
        cp = node.get("cp", "").replace(VS16, "")
        if node.get("type") == "tts":
            names[cp] = node.text or ""
        else:
            found[cp] = [word.strip() for word in (node.text or "").split("|") if word.strip()]
    return found, names


def main(test_path, annotations_path):
    words, tts = keywords(annotations_path)
    group = None
    rows = []
    bases = {}
    for line in open(test_path, encoding="utf-8"):
        line = line.rstrip("\n")
        if line.startswith("# group: "):
            group = line[len("# group: "):]
            continue
        match = re.match(r"^([0-9A-F ]+?)\s*;\s*fully-qualified\s*#\s*(\S+)\s+E[0-9.]+\s+(.*)$", line)
        if not match or group not in CATEGORY:
            continue
        emoji = match.group(2)
        name = match.group(3)
        toned = [c for c in emoji if c in TONES]
        if toned:
            stripped = "".join(c for c in emoji if c not in TONES)
            key = stripped.replace(VS16, "")
            bases.setdefault(key, []).append(emoji)
            continue
        key = emoji.replace(VS16, "")
        cldr = tts.get(key, name)
        keys = [w for w in words.get(key, []) if w.lower() != cldr.lower()]
        rows.append([emoji, CATEGORY[group], cldr, " | ".join(keys), key])
    for row in rows:
        variants = bases.get(row[4], [])
        row[4] = " ".join(variants)
    # Multi-person tone variants (couples, handshakes) have several modifiers
    # and would not map to a base; they are simply not offered.
    sys.stdout.write("\n".join("\t".join(row) for row in rows) + "\n")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
