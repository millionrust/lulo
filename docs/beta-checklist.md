# Beta release checklist — 0.9.0-beta.1

This is the release-blocking list for shipping **0.9.0-beta.1** as a GitHub
Release: `.deb` packages (`rmac-apps`, `rmac-session`, and the keyring
package when a signing key exists) plus `SHA256SUMS`, an SBOM, build
provenance, and a manual install guide. The signed APT repository comes
after this Beta (it needs the owner's signing-key decision and a real Debian
source package; see [docs/release-process.md](release-process.md)).

Status key: **Pass**, **Fail**, **Not yet run**, **Waived — reason**. Per
`todo.md`'s own direction ("Cut scope, never gates"), data-safety,
accessibility, and security gates are never marked Waived here, even under
release pressure — only Pass, Fail, or Not yet run.

## Read this first: two different things are called "Beta"

This repository already defines a much larger "Beta" gate:
[docs/beta-cohort.md](beta-cohort.md) and `scripts/beta-candidate.json`
describe an **invited daily-driver cohort** — at least 20 participants for
14 days (280 participant-days), a 90%+ pass rate on 20 attempts of each of
the ten product journeys, three certified H8 hardware stations, all 15
promotion checks, and zero open defects across four safety classes (data
loss, privilege escalation, session lockout, critical accessibility). That
gate is nowhere close to met (see below) and this checklist does not claim
otherwise.

**0.9.0-beta.1, as scoped for this release, is a much smaller thing**: a
downloadable early-access build for people willing to run pre-release
software on a disposable machine, published so real users outside the
project can start reporting problems. Calling it "Beta" in the GitHub
Release matches the owner's naming decision, but it should not be read or
announced as satisfying `docs/beta-cohort.md`'s cohort gate — that gate, and
the H8 hardware matrix and 1.0-candidate gate above it, still apply before
any wider or "recommended" claim is made. Consider a subtitle on the
release itself (e.g. "0.9.0-beta.1 — early access, not the daily-driver
Beta cohort") if that distinction needs to survive outside this document.

## 1. Product journeys (todo.md)

| # | Journey | Script | Last result | Status |
|---|---|---|---|---|
| 1 | Log in, launch from Dock/Spotlight, switch, close | `scripts/linux/run-journey-launch.py` | Ran live on the reference laptop. Dock tiles and Spotlight rows were fixed to expose a real AT-SPI `click`/`Text` surface (see `docs/journey-suite.md`). Spotlight's search field still can't be **typed into** over AT-SPI: the pinned `accesskit_unix` bridge has no `EditableText` implementation at all (upstream gap, not fixable here). | **Fail** — one real, live, unfixable-in-repo accessibility gap (no-keyboard-injector query typing); everything else in journey 1 passes |
| 2 | Find, preview, copy, move, rename, trash a file; undo | `scripts/linux/run-journey-files.py` | Ran live 2026-09-24. No file/folder/sidebar row is exposed over AT-SPI at all (only toolbar controls); `rename` fails for real (no Rename menu item, inline-only editing needs a keyboard injector that isn't installed); Trash and Undo pass reliably once the window has focus; Copy/Move now work after fixing the Wayland clipboard no-op. See `docs/journey-suite.md`. | **Fail** — no accessible file list, no working rename path |
| 3 | Terminal: run, scroll, select, copy/paste, tabs | `scripts/linux/run-journey-terminal.py` | Ran live. The terminal grid publishes **no AT-SPI text, caret, or selection** — the accessibility module exists but isn't compiled into the running binary. Typing a command over AT-SPI is impossible on this build. Tab strip works (unnamed but clickable). | **Fail** — no accessible text-entry surface; this is journey 3's headline finding |
| 4 | Notes: create, search, edit, recover after crash | `scripts/linux/run-journey-notes.py` | Ran live. Deliberately stopped before creating any note: no text entry surface at all (search/title/tags/body expose no `Text`/`EditableText`), and the note list/sidebar is entirely absent from the AT-SPI tree (not just unnamed). A test note could be created but never re-identified or cleaned up, so the script refused to risk the real Notes library. | **Fail** — same class of gap as journey 3, one step earlier |
| 5 | Open/edit/save a text file through the portal | none | No live acceptance script exists yet. | **Not yet run** |
| 6 | Inspect resource use, safely stop a process | none | No live acceptance script exists yet. `docs/journey-suite.md` only covers 1–4. | **Not yet run** |
| 7 | Wi-Fi, Bluetooth, audio output, battery | none (code trace only) | `docs/journey-7-trace.md` traces every step through source reading (no cargo, no live laptop UI run in that pass) and fixed one small gap (menubar quick Wi-Fi join now shows "Connecting…"/failure state). Backends are confirmed real (`docs/settings-backend-audit.md`). | **Not yet run** as a live acceptance test — code-level trace only |
| 8 | Journeys 1–7, keyboard only | none | No dedicated script. Partial evidence only: the Dock is keyboard-reachable via Control-F3; Control-F2 (menu bar keyboard focus) is explicitly **not implemented** (`docs/known-limitations.md`). | **Fail** — Control-F2 is a known, documented gap, and journeys 3/4 above are keyboard-blocked at the content level regardless |
| 9 | Core of 1–7 with Orca at 200% | none | No dedicated script. The formal I3 accessibility-release audit (`docs/accessibility-release-audit.md`, `scripts/accessibility-audit.json`) requires 442 explicit Orca observations across every surface and journey; no evidence file for that audit exists in this repository. | **Not yet run** |

The package-scoped automated fixture suite (`scripts/run-journey-suite.py`,
`docs/journey-suite.md`'s "I1", 10 journeys mapped to Cargo package tests)
was not run in this session — it needs `cargo test`, which this session
does not have. The pure-Python logic tests for the four live scripts above
(`test_journey_launch.py`, `test_journey_files.py`, `test_journey_terminal.py`,
`test_journey_notes.py`) do pass, but they only cover the scripts' own
JSON/parsing logic, not a live run.

## 2. Performance budgets (todo.md)

| Metric | Budget | Status |
|---|---|---|
| Warm launch to interactive | p95 ≤ 500 ms (900 ms Files/Terminal) | **Not yet run** |
| Idle CPU | ≤ 0.3%/app, ≤ 1% shell combined | **Not yet run** |
| Idle wake-ups | none while nothing changes | **Not yet run** |
| Input to visible response | p95 ≤ 50 ms | **Not yet run** — no frame-timing harness exists yet |
| 60/120 Hz animation frame budget | ≥ 99% / ≥ 95% within budget | **Not yet run** — `docs/performance-baseline.md` notes no per-frame trace is available yet |
| Memory (8-hour soak) | per-app budget, no leak | **Not yet run** |
| Repeat on an NVIDIA system | required before Beta | **Not yet run** — no NVIDIA station in the matrix yet |

Evidence: [docs/perf/reference-laptop-2026-09-24.md](perf/reference-laptop-2026-09-24.md)
is explicitly a **smoke validation only** of the new `measure-budgets.py`
harness — the reference laptop was mid-`cargo build` for the whole session,
so every number in it (idle CPU as high as 12.99%, 190 wake-ups/s) reflects
build contention, not the product, and the doc says so in its own words.
The real measurement run (idle laptop, `pgrep -c rustc` reporting 0) has not
been done. **Status: Fail** — the todo.md budget table has no real
measurement behind any row yet.

## 3. Accessibility gates (todo.md)

| Gate | Status | Evidence |
|---|---|---|
| Stable identity/role/name/state/actions on every `rmac-ui` control | **Fail** (partial) | `docs/accessibility-audit.md`: Toggle/Checkbox/Radio/dialogs/menus/toasts/traffic lights fixed this pass; `PopUpButton`, `ContextMenu` items, `ListRow`/`TreeRow` still wrap `gpui_component::Button` and are stuck at `Role::Button` until either an upstream `gpui-component`/`gpui-kit` change or a rewrite off `Button` (Blocked, not Gap, per that doc) |
| Correct tab order, visible focus ring | **Fail** (partial) | Same doc: a shared 3pt focus ring now exists and is used on the rewritten controls; everything still wrapping `Button` keeps a hardcoded 1.5px ring it cannot override (Blocked) |
| Full keyboard operation, no pointer-only controls | **Fail** | Journeys 3 and 4 above are the sharpest evidence: Terminal and Notes have **no accessible content surface** at all, live-confirmed, not merely a style gap |
| Announcements for async status/errors | **Fail** (partial) | Toast fixed this pass; `EmptyState`'s error variant and list Loading/Empty/Error messages still have no role (Gap) |
| No clipping at 200% | **Not yet run** (S) | `docs/accessibility-audit.md`: static read-through only, no rendered check at 200% |
| Usable high-contrast colours | **Pass** (token level) | Theme contrast-ratio tests pass; no shared component hardcodes a raw color |
| Reduced motion from the Settings portal | **Pass** (plumbing) / **Not yet run** (exercised) | Portal → theme wiring is tested end to end, but no shared component currently animates anything, so the gate has nothing live to violate yet |
| Verify every release journey with Orca | **Fail** | See journeys 1–4 above; 5–9 not run at all |
| Formal I3 accessibility-release audit (442 observations) | **Not yet run** | `docs/accessibility-release-audit.md` / `scripts/verify-accessibility-audit.py`; no evidence file committed |

**This is the release-blocking category.** `todo.md` states accessibility
gates are never waived. Terminal and Notes shipping with zero accessible
content surfaces, and Files shipping with no accessible file list, are real,
live-confirmed, user-facing accessibility failures in three of the eleven
apps in this Beta — not measurement gaps.

## 4. Code rules (todo.md)

| Rule | Status | Evidence |
|---|---|---|
| No destructive-operation error dropped with `let _ = ...` | **Pass** (fixed) | `docs/code-rules-audit.md`: 5 real violations found and fixed (window/Finder/media-player state saves now log; clipboard payload delete and an unconfirmed permanent-delete path now refuse instead of silently dropping/deleting) |
| Persisted data: versioned serde, temp-sibling + fsync + atomic rename via `rmac-storage` | **Fail** (partial) | Same doc: 5 low-severity settings/cache writers (`rmac-screenshot`, `rmac-compositor` parking cache, `rmac-gtk-settings` stub writer, `rmac-dock-runtime` recents, `rmac-clipboard-linux`) still hand-roll their own write path instead of `rmac-storage`; none touch high-value user content, but none are fixed yet (needs a Linux `cargo check` this session didn't have) |
| User-visible failures: typed errors with a recovery action | **Pass** | No violations found in the crates audited |
| Logs never include secrets or document contents | **Pass** | No violations found |
| Domain crates never import GPUI/Wayland/D-Bus/platform FFI | **Pass** (with 2 naming nits) | `rmac-app-menu`/`rmac-osd` carry `zbus` without a `-linux`/`-system` suffix — functionally fine, a naming-convention note only |
| Files safety rules (symlinks, conflict policy, cancel, Trash-first, zip-slip) | **Pass** (1 bug found and fixed) | `docs/code-rules-audit.md` Rule 6: `DeleteItem`'s unconfirmed permanent-delete path (dead code, nothing bound to it) now refuses instead of deleting |
| Never parse human-readable CLI output on Linux | **Pass** (fixed) | `docs/code-rules-audit.md` Rule 7: UFW/Samba/PipeWire status now read structured state; full repo grep found no remaining violation on the Linux build |

This audit's scope explicitly excluded `crates/system-settings/**`,
`shell/bins/rmac-wallpaper/**`, `shell/crates/rmac-shell-ui/**`, and the
network/Bluetooth/audio/power/top-bar/quick-settings crates (other agents
were actively editing them) — those are **not yet audited** against these
rules at all.

## 5. Packaging: install / remove / upgrade

| Check | Status | Evidence |
|---|---|---|
| Native package contract tests | **Pass** | `python3 -m pytest scripts/test_native_packages.py scripts/test_session_package.py scripts/test_application_package.py scripts/test_keyring_packages.py` all pass in this session |
| Release workflow structural checks | **Pass** | `python3 -m pytest scripts/test_release_workflows.py` passes; `release.yml` now tags pre-release builds correctly (this pass) and needs no APT-signing secrets to build/attach `.deb`s, `SHA256SUMS`, the SBOM, and provenance |
| Real install/remove/upgrade on a clean Ubuntu 26.04 VM or the reference laptop | **Not yet run** | Needs an actual `apt install ./rmac-apps_*.deb ./rmac-session_*.deb`, a second install as an upgrade, and `apt purge`, on a real machine — this session had no cargo and no laptop UI access for a packaging run |
| GDM session-selection, crash-loop safe mode, TTY repair, recovery, GNOME-fallback journey (H4/H6) | **Not yet run** | `docs/session-journey-evidence.md` is a runbook; no evidence file from an actual run is committed |
| Real GitHub Actions release run (untested runner plumbing) | **Not yet run** | `docs/release-process.md` "Known gaps": the non-root-build-user-in-a-container pattern and container disk space have never been exercised against a real GitHub Actions run — expect to debug the first tag push |
| Reproducible build (two independent byte-identical package assemblies) | **Not yet run** (this session) | Contract exists and is unit-tested (`check-native-reproducibility.sh`); not executed here (no cargo) |
| Signed APT repository install/upgrade/rollback | **Not applicable to this Beta by design** | Explicitly deferred to after Beta per the owner's scope decision; `apt-repository`/`keyring` jobs stay off until the signing-key and source-package decisions in `docs/release-process.md` are made |

## 6. Documentation

| Check | Status |
|---|---|
| `python3 -m pytest scripts/test_documentation.py` | **Pass** |
| `python3 scripts/verify-documentation.py` | **Pass** — "rmac documentation set verified (15 required topics)" |
| CHANGELOG.md | **Pass** — added this pass |
| docs/release-notes.md (required doc) + docs/release-notes/0.9.0-beta1.md | **Pass** — added/updated this pass |
| docs/known-limitations.md reflects current reality | **Pass**, but see §7 — it already names the release blockers honestly and should stay accurate as the gaps above close |
| docs/install.md / README.md readiness banners | **Needs an owner decision** — both currently say the product "is not released for general installation yet" / "isn't ready for daily use yet." Shipping 0.9.0-beta.1 as a GitHub Release means someone outside the project can now install it; those banners should be reworded (early-access Beta, not "not released yet") once the owner confirms this release is going out, rather than left contradicting the Release page. Not changed in this pass since it's a product-readiness claim, not a mechanical doc-set fix. |

## 7. Known limitations

`docs/known-limitations.md` is current and, cross-checked against the live
journey evidence above, accurate. It already names the Dock/menu-bar
keyboard gaps, the AT-SPI `EditableText` upstream gap, the placeholder
top-bar mark, and the absence of a signed APT repository. It does **not**
yet name Terminal's and Notes' complete absence of an accessible content
surface, or Files' missing accessible file list — those are more severe
than what's currently listed under "Accessibility limits" and should be
added before this Beta ships, since `docs/known-limitations.md` is the doc
this checklist and the release notes both point readers to.

## 8. CI

Not independently verified in this session — this environment has no `gh`/
GitHub API access and no cargo, so "CI green on `dev`/`beta-release`" could
not be checked directly. **Confirm on GitHub before tagging:**
`dependency-policy`, `linux`, `macos`, `upstream-gpui-linux` on `ci.yml`, and
`msrv`/`linux-aarch64` (non-blocking) on `ci-quality.yml`. Structural
self-checks that *can* run without CI access all pass in this session (see
§5/§6 tables and the pytest command below).

`linux-2604` (the Ubuntu 26.04 hosted-runner trial) is still explicitly
non-blocking per `todo.md`; it has not yet replaced `linux`/
`upstream-gpui-linux` as the plan calls for once its `apt` package list is
verified.

## 9. Versioning and release engineering (this pass)

- Both workspace manifests (`Cargo.toml`, `shell/Cargo.toml`) now version
  `0.9.0-beta.1` (Cargo/semver pre-release syntax, so `0.9.0` final sorts
  higher under Cargo's own ordering).
- `scripts/linux/native_package_contract.py` and
  `scripts/linux/keyring_package_contract.py` translate that into the
  Debian tilde convention (`0.9.0~beta.1`) for the actual `.deb` `Version`
  field, so `dpkg`/APT also order a Beta below its eventual final release;
  `scripts/linux/run-package-lifecycle.py`'s manifest-version check was
  updated to accept the same pattern.
- The 13 app AppStream `metainfo.xml` files carry a `0.9.0-beta.1` release
  entry; `scripts/linux/verify-application-package.py`'s expected value
  matches.
- `release.yml`'s `attach-release` job now publishes any tag with a
  pre-release suffix (e.g. `v0.9.0-beta.1`) as a GitHub pre-release.
- Every pinned-version Python test that reads the real repository was
  updated and passes (see the command below).

## Required test run (this session)

```sh
python3 -m pytest -q scripts/test_native_packages.py scripts/test_session_package.py \
  scripts/test_release_workflows.py scripts/test_documentation.py
python3 scripts/verify-documentation.py
```

All pass. (Also re-checked, not in the required list but touched by the
versioning changes: `scripts/test_keyring_packages.py`,
`scripts/test_application_package.py`, `scripts/test_alpha_candidate.py`,
`scripts/test_beta_candidate.py`, `scripts/test_one_dot_zero_candidate.py`,
`scripts/test_package_lifecycle.py` — all pass.)

## What actually blocks shipping 0.9.0-beta.1 today

In order of severity:

1. **Accessibility (never waived).** Terminal and Notes have no accessible
   content surface at all; Files has no accessible file list or working
   rename. These are live-confirmed on the reference laptop, not projected.
2. **No real packaging install/remove/upgrade run** on a clean VM or the
   reference laptop, and no exercised GitHub Actions release run — the
   pipeline is well-gated and unit-tested but has never produced a real
   `.deb` on real CI infrastructure.
3. **No real performance measurement.** The only recorded numbers are an
   explicitly-invalid smoke run taken during a concurrent build.
4. **No security review evidence.** `scripts/security-review.json`'s 80
   checks have no completed evidence file in this repository.
5. **Journeys 5, 6, and 8/9 have no acceptance test at all**, live or
   automated; journey 7 has only a source-code trace.
6. **`docs/install.md`/`README.md` still say the product isn't ready to
   install** — an owner-level messaging decision, not a mechanical fix.

None of these are release-engineering plumbing problems — the tagging,
packaging contracts, versioning, and pre-release workflow gating are ready
(§5, §9). What's missing is the evidence that the product itself is safe
and usable enough to hand to someone outside the project, which is exactly
what `todo.md`'s accessibility/security/data-safety gates exist to prove.
