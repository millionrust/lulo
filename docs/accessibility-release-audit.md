# Accessibility release audit

I3 is an evidence gate, not a claim that detectable Orca prerequisites make
rmac accessible. The exact audit contract is
`scripts/accessibility-audit.json`; its verifier binds the audit to the 27
critical visual-suite surfaces and all ten prioritized product journeys.

The audit requires 442 explicit observations. Every interactive application,
Settings pane, and shell surface covers roles and names, states and values,
actions, focus order and restoration, visible focus, keyboard operation,
announcements, Orca reading order, contrast, reduced motion, and native 200%
scaling. Passive desktop and top-bar surfaces use the relevant semantic and
adaptive subset. Ten text-entry surfaces additionally require a real chosen
IME. Every product journey repeats the complete non-IME audit so a collection
of isolated widgets cannot stand in for end-to-end accessibility.

Validate the committed contract without running Rust:

```sh
python3 scripts/verify-accessibility-audit.py
```

## Reference-PC procedure

Use the supported Ubuntu 26.04 rmac/niri session and the exact candidate
revision. First collect the existing bounded environment and F16 evidence:

```sh
scripts/linux/run-accessibility-evidence.sh
```

Create the pending result inventory outside the repository:

```sh
python3 scripts/verify-accessibility-audit.py \
  --print-template \
  --revision "$(git rev-parse HEAD)" \
  > /absolute/review/path/accessibility-audit.json
```

Run the audit with Orca and working speech output. Inspect the AT-SPI tree; do
not infer roles, names, states, values, actions, or announcements from pixels.
Traverse each surface and journey by keyboard only, exercise forward and
reverse focus, cancel and restore focus around menus/dialogs, and validate live
and error announcements. Repeat with increased contrast, reduced motion, a
real 200% output, and the selected IME wherever the inventory requires it.

Change a result to `pass` only after a human has reviewed that exact
observation. `pending`, `blocked`, `skip`, missing, duplicate, extra, reordered,
stale-revision, or source-manifest-drift results do not pass:

```sh
python3 scripts/verify-accessibility-audit.py \
  --evidence /absolute/review/path/accessibility-audit.json \
  --revision "$(git rev-parse HEAD)"
```

The reviewed JSON contains only canonical check identities and pass status.
Keep recordings, screenshots, speech transcripts, AT-SPI dumps, typed text,
host/session identities, paths, bus peers, and raw diagnostics in the ignored
evidence tree. Use synthetic documents and accounts. A passing JSON inventory
still requires the corresponding reviewed raw evidence and H8 hardware
coverage; the verifier cannot prove what a reviewer did not observe.
