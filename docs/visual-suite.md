# Deterministic visual references

I2 defines 27 critical app, Settings and shell screens in
`scripts/visual-suite.json`. Each screen is reviewed in light, dark,
light-increased-contrast and dark-increased-contrast at native 100%, 125%,
150% and 200% scale: 432 exact references.

The capture authority fixes the locale, UTC clock/timezone, deterministic seed,
grayscale font antialiasing, Inter UI font and JetBrains Mono code font.
Application windows use 1280×800 logical pixels, Settings 1100×760, and shell
surfaces a 1440×900 output. Service and document fixtures must contain only
deterministic synthetic data.

Validate the inventory without capturing:

```sh
python3 scripts/verify-visual-suite.py
```

On the clean Linux candidate, capture PNGs named:

```text
<screen>--<theme>--<native-scale>.png
```

Do not resize a 100% image and call it native scaling. Restart the relevant
process for each theme/scale, load the exact fixture, wait for authoritative
state to settle, and capture the declared physical dimensions. Inspect layout,
clipping, hierarchy, focus, contrast, typography, icons, empty/loading/error
states and whether controls truthfully match their authority.

PNG references must use canonical noninterlaced 8-bit RGB/RGBA encoding and
contain no EXIF, timestamps, comments or text chunks. `visual-review.json`
binds the exact revision, visual-manifest hash, Inter/JetBrains Mono file
hashes, and an ordered Pass plus SHA-256 for every image. Raw references remain
in the ignored evidence directory.

Verify a reviewed directory:

```sh
python3 scripts/verify-visual-suite.py \
  --references /absolute/path/to/references \
  --revision "$(git rev-parse HEAD)"
```

The verifier rejects missing/extra images, links, metadata, malformed PNG
chunks, wrong dimensions, font/manifest/revision drift, altered hashes and any
result not explicitly reviewed as Pass. A structurally valid image is not
automatically a good design; human review and I3 accessibility evidence remain
mandatory.
