# Mac and Lulo visual comparison records

Use this record format when reviewing a Lulo screenshot against a Mac capture.
The Linux visual suite in `docs/visual-suite.md` validates the shape and review
status of Linux reference images; it does not compare Lulo with macOS. Mac
captures in `target/evidence/reference-mac/` are the measurement authority.
Keep captures and records in ignored `target/evidence/` storage.

## Capture pair

Create one directory per surface and state, for example
`target/evidence/visual-comparisons/dock-idle-dark/`, containing the original
files `mac.png`, `lulo.png`, and `comparison.md`. Do not crop, resize, recolor,
or recompress either original. If annotated crops help explain a finding, save
them as separate files and label them as derivatives.

After filling in the record, audit all pair directories with:

```sh
python3 scripts/audit-visual-comparisons.py
```

Or provide another evidence root as the first argument. The audit reports each
missing pair, verifies the recorded SHA-256 and PNG dimensions, and requires
all provenance and review fields to be populated. It exits nonzero if the root
is empty or any pair is incomplete or inconsistent. It does not inspect visual
similarity or assign a pixel score. Its focused fixtures can be run with
`python3 scripts/test_audit_visual_comparisons.py`.

Copy this record for each pair:

```markdown
# <surface> — <state>

Mac image: mac.png
Mac SHA-256: <hash>
Mac pixel dimensions: <width> × <height>
Mac OS/build: <version and build>
Mac display: <resolution, scale, color profile if known>
Mac appearance: <light/dark, accent, contrast, transparency>
Mac locale/region: <locale and region>
Mac state: <window/surface, selection, focus, pointer, open panels>

Lulo image: lulo.png
Lulo SHA-256: <hash>
Lulo revision: <full git revision>
Lulo display: <resolution, scale, color profile if known>
Lulo appearance: <light/dark, accent, contrast, transparency>
Lulo locale/region: <locale and region>
Lulo state: <window/surface, selection, focus, pointer, open panels>

Review: <pass | mismatch | not comparable>
Findings: <specific visible matches, differences, and unknowns>
Reviewer/date: <name or handle, YYYY-MM-DD>
```

Record the actual setup on both sides. A matching nominal resolution does not
establish matching physical scale, font rasterization, color management, or
window state. If a condition is unknown, write `unknown`; do not fill it from
assumption. The image hashes identify the reviewed bytes and make later edits
visible.

## Interpreting the result

`pass` means a human reviewed the listed visible properties for the stated
surface and state. It does not claim whole-product parity. `mismatch` records
the differences that remain. `not comparable` is appropriate when scale,
state, or source provenance prevents a meaningful side-by-side judgment.

Use pixel differences only when both originals have the same pixel dimensions,
capture scale, state, and color treatment, and are aligned without resizing.
Even then, treat a difference image or score as a locator for inspection, not
as an acceptance threshold: native controls, font rasterization, and platform
rendering can differ while layout and behavior are correct. Never report
“pixel parity” from visual inspection, a resized overlay, or a score alone.

## Current evidence boundary

The checked-in inventory and ignored captures serve different purposes. The
27-screen Linux visual manifest defines 432 theme/scale variants, but the
reference directory is intentionally ignored and requires human review. The
Mac capture notes in `docs/reference-captures-2026-09-18.md` describe a broader
set than the local `target/evidence/reference-mac/` directory may contain.
Before citing a surface as screenshot-compared, confirm both original files
and a completed pair record exist for that surface and state. Behavioral
scenarios under `tests/behavior/` are separate evidence and do not establish
visual similarity.
