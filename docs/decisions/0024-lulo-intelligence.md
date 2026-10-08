# ADR 0024 — Lulo Intelligence: a small local model, loaded on demand, behind typed actions

- **Status:** accepted; phase 1 in progress on `op/ai-phase1` (2026-10-07): the
  `org.rmac.Intelligence1` service, prompt-prefix caching, Spotlight "Lulo can do this" rows and
  the Settings ▸ Lulo Intelligence pane. Fine-tuning is approved by the owner (2026-10-07) and is
  a planned phase. Any rented-GPU spend still needs the owner's account and payment.
- **Scope:** new crates `rmac-intelligence` (types, prompts, hardware gate, action registry,
  client), `rmac-intelligence-service` (the model process), `rmac-voice` (speech, phase 3);
  a new System Settings pane `crates/system-settings/src/controller/intelligence`; hooks in
  Spotlight, Notes, Text Editor, Mail, Terminal, Preview, Files and the Help menu; session
  units in `crates/rmac-session/units/`; `scripts/intelligence/` for dataset and evaluation.
- **Builds on:** ADR 0006 (shell/app split), ADR 0011 (consent-first clipboard service, the
  pattern for an opt-in D-Bus service), ADR 0022 (accounts; Mail and Calendar data), ADR 0023
  (Windows seam, named pipes).
- **Changes a design rule:** none. Spotlight stays "Search" with no assistant row
  (`docs/parity.md` § Spotlight). Requests in plain language get their own panel (decision 9).

## The question

The owner wants "Lulo Intelligence": everyday AI help like Apple Intelligence, but entirely
on the PC. Later requests added voice ("hey Lulo, do this") and an agent that carries out
multi-step tasks across apps. Agreed direction: no training from scratch; a small,
permissively licensed open model, fine-tuned on Lulo tasks; off by default; loaded only when
used and unloaded when idle (zero idle RAM and CPU); offered only on hardware that can run it;
nothing leaves the PC unless the user deliberately adds their own cloud model.

Lulo targets low-spec PCs. The reference laptop is an i5-5300U (Broadwell, 2 cores and 4
threads, AVX2, no AVX-512) with 6.7 GB RAM and an HD 5500 GPU. A model that makes the
desktop swap or stutter breaks Lulo's main promise, so speed and memory budgets decide
everything below.

Apple's answer has the same shape. Its on-device model has about 3B parameters, uses 2-bit
quantisation-aware training, and gets LoRA adapters for specific features
([Apple, Foundation Models tech report 2025](https://machinelearning.apple.com/research/apple-foundation-models-tech-report-2025)).

## 1. Features, ranked by everyday use and small-model fit

Small models are good at short, constrained work (classify, fill slots, rewrite a paragraph)
and weak at open-ended reasoning over long inputs. On an old CPU, prompt processing
(prefill) is the slow part, so long documents cost more than short requests.

| # | Feature | Mac equivalent | What the model does | Fit | Hooks (real files) |
|---|---|---|---|---|---|
| 1 | **Settings and actions in Spotlight** ("turn on dark mode", "bluetooth settings", "make the text bigger") | macOS 26 Spotlight actions ([Apple newsroom](https://www.apple.com/newsroom/2025/06/macos-tahoe-26-makes-the-mac-more-capable-productive-and-intelligent-than-ever/)); Siri | Map the query to one intent from a closed list, with slots. Output is grammar-constrained JSON, never free text. A rule-based parser answers the common phrasings first; the model is the fallback. | **Best.** Classification over ~150 intents. | New provider beside `crates/rmac-launcher-providers/src/settings.rs` (`SettingEntry`, `system_settings_entries()`), declared with `rmac_launcher::Privacy { private_content: false, network: false }` (`crates/rmac-launcher/src/model.rs`). Rows render as ordinary results in `crates/launcher-app/src/view/`. Opening a pane uses `Action::OpenSetting`, run by `crates/rmac-launcher-system/src/execution.rs`; a change uses the typed operations in `rmac-quick-settings-system::execute` after confirmation (§5). |
| 2 | **Writing Tools** in Notes, Text Editor and Mail: Proofread, Rewrite, Friendly, Professional, Concise, Summary, Key Points, List | Writing Tools ([Apple Support](https://support.apple.com/guide/mac-help/find-the-right-words-with-writing-tools-mchldcd6c260/15.0/mac/15.0)) | Rewrite a selection of up to about 600 words; stream the result into a preview. | **Good.** Short inputs. Proofread is the most reliable. | `crates/rmac-ui/src/text_assist.rs` (`EditableText`, already used by Notes, Text Editor, Clock and Preview) and `text_transform.rs`; Notes `crates/notes/src/edit_text_assist_controller.rs`; Text Editor `crates/text-editor/src/view/text_assist.rs`; Mail `crates/mail/src/compose.rs` and `compose_window.rs`, which need an `EditableText` implementation. The Edit ▸ Writing Tools submenu goes in the `crates/rmac-app-menu/src/lib.rs` tables, and the same items in `crates/rmac-ui/src/context_menu.rs`. |
| 3 | **Text in images (OCR)** in Preview and Quick Look | Live Text | **No LLM.** Use deterministic OCR: `tesseract` on Linux (Apache-2.0, already planned behind runtime detection in PREV-10/PRV-MENU-014) and `Windows.Media.Ocr` on Windows. The Rust-native [`ocrs`](https://github.com/robertknight/ocrs) (MIT/Apache) is Latin-only and "early preview", so it is something to watch, not the default. | n/a | Preview `crates/preview/src/selection.rs` (Text Selection tool), `view.rs`; Quick Look `crates/rmac-quick-look/src/content.rs`. |
| 4 | **Explain this command** in Terminal | None. This is Lulo-only. | Explain one command line, grounded in the command's own `--help` or `man` text, which the service is given as context. It never suggests running anything (agent mode, §9, handles that last). | **Fair.** It invents flags when not grounded; grounding fixes most of that. | `crates/terminal/src/shell_integration.rs` (OSC 133 command ranges give the exact command text); context menu `crates/terminal/src/controller/renderer/interactions.rs`; popover `overlays.rs`. |
| 5 | **Summaries** in Preview and Files | Summaries in Mail, Safari and notifications | Summarise up to about 3,000 tokens: a PDF's text, or a text file in Quick Look. | **Fair, but slow:** prefill on the reference laptop may take 10–30 s (to be measured in phase 0). Show progress and cap the input. | PDF text from `crates/preview/src/poppler.rs` (`pdftotext -bbox`; Windows `winpdf.rs`); Files `crates/rmac-quick-look/src/content.rs` (`load_regular_text`, `load_pdf`) via `crates/finder/src/view/quick_look_controller/controller.rs`. |
| 6 | **Lulo Help answers** | Help menu search (keywords only) and the Tips app | Retrieve with BM25 over the help pages, no model needed; then the model answers in two or three sentences, citing and linking the page. | **Good** once help text exists. Today there are only 10 pages. | `shell/bins/rmac-menubar/src/menu_model.rs` (`help_menu`), `shell/assets/help/*.md`; later the Settings "?" buttons (SET-55). |

Order of work: 1 and 2 (phase 1); 3–6 (phase 2). Voice (§8) and agent mode (§9) reuse the
action registry that feature 1 creates.

## 2. Model shortlist

Sizes are the Q4_K_M GGUF files on Hugging Face (read through its API on 2026-10-07).
"RAM" is the file plus KV/state and compute buffers at 4K context. The **tok/s** figures are
estimates for decoding on the reference laptop's CPU with no GPU. Decode speed is limited by
memory bandwidth, so the estimate is effective DDR3L bandwidth (about 15 GB/s) divided by the
bytes read per token. Published figures agree: on an ordinary i5 laptop, models under 2B run
at 18–36 tok/s and 3–4B models at 7–10 ([PromptQuorum](https://www.promptquorum.com/local-llms/best-cpu-only-llm));
a 4B model runs at about 2 tok/s on a Core 2 Duo ([llama.cpp #21136](https://github.com/ggml-org/llama.cpp/discussions/21136)).
Phase 0 replaces these estimates with measurements.

| Model | Params | Licence | Q4_K_M file | RAM | Est. tok/s (ref. laptop) | Notes |
|---|---|---|---|---|---|---|
| **Qwen3.5-0.8B** | 0.8B | Apache-2.0 | 0.53 GB | ~0.8 GiB | 20–30 | Released 2026-03-02 ([Artificial Analysis](https://artificialanalysis.ai/articles/qwen3-5-small-models)). Same tokenizer and template as 2B. |
| **Qwen3.5-2B** | 1.88B | Apache-2.0 | 1.28 GB (+0.67 GB vision projector, optional) | ~1.6 GiB | 8–12 | Hybrid design: 18 Gated DeltaNet layers and 6 attention layers ([Spheron](https://www.spheron.network/blog/deploy-qwen-3-5-gpu-cloud/)), so the KV cache is small. Small Qwen3.5 models default to non-thinking mode ([Unsloth](https://unsloth.ai/docs/models/qwen3.5)). |
| Qwen3.5-4B | 4B | Apache-2.0 | 2.74 GB | ~3.2 GiB | 4–6 | Best quality under 5B on the Artificial Analysis index. |
| Granite 4.0 1B / Micro 3B | 1B / 3B | Apache-2.0 ([IBM](https://www.ibm.com/granite/docs/models/granite)) | 1.02 / 2.10 GB | ~1.3 / ~2.5 GiB | 10–15 / 5–7 | A strong second source if Qwen is ever unacceptable. |
| Gemma 4 E2B | ~2B effective | Apache-2.0, a first for Gemma ([Google](https://opensource.googleblog.com/2026/03/gemma-4-expanding-the-gemmaverse-with-apache-20.html)) | 3.11 GB | ~3.4 GiB | 6–9 | Has audio input. Its per-layer embeddings make the file large for its speed. |
| SmolLM3-3B | 3B | Apache-2.0, fully open data | 1.92 GB | ~2.3 GiB | 5–8 | |
| Phi-4-mini | 3.8B | MIT | 2.49 GB | ~3.0 GiB | 4–6 | Strong at maths; no edge for our tasks. |
| Llama 3.2 3B | 3B | **Custom** (Llama 3.2 Community Licence: attribution, an acceptable-use policy, a 700M-MAU clause) | 2.02 GB | ~2.4 GiB | 5–8 | **Excluded** because of the custom licence. |

**Decision:** the two tiers are **Qwen3.5-0.8B** (Tiny) and **Qwen3.5-2B** (Standard). Phase 0
showed that on 2-core PCs like the reference laptop the 0.8B model is the right default; 2B is
offered only where it decodes at 8 tok/s or better (§4's hardware gate).
Reasons: both are Apache-2.0; they are the best scorers at their sizes; the hybrid attention
keeps memory low at long context; and they share one tokenizer and chat template, so one
prompt set and one dataset serve both tiers. **Qwen3.5-4B** is an opt-in "Enhanced" download
for PCs with 12 GB of RAM or more. Granite 4.0 is the documented alternative if a deployment
cannot use a Qwen-family model.

## 3. Runtime

| | **llama.cpp via `llama-cpp-2`** | candle | mistral.rs |
|---|---|---|---|
| Licence | llama.cpp MIT; bindings MIT/Apache-2.0 ([repo](https://github.com/utilityai/llama-cpp-rs)) | MIT/Apache-2.0 ([repo](https://github.com/huggingface/candle)) | MIT ([repo](https://github.com/EricLBuehler/mistral.rs)) |
| CPU SIMD | AVX, AVX2, AVX-512, AMX, NEON ([repo](https://github.com/ggml-org/llama.cpp)). `GGML_BACKEND_DL` with `GGML_CPU_ALL_VARIANTS` builds every CPU variant and picks one at run time ([ggml CMake](https://raw.githubusercontent.com/ggml-org/llama.cpp/master/ggml/CMakeLists.txt)), so one package is fast on every x86 PC. | gemm kernels, optional MKL; quantised matmul is slower (7–8 tok/s against llama.cpp's 11 in one 7B comparison, [Medium](https://medium.com/@zaiinn440/apple-mlx-vs-llama-cpp-vs-hugging-face-candle-rust-for-lightning-fast-llms-locally-5447f6e9255a)) | candle kernels plus MKL/Accelerate |
| GPU | **Vulkan** (Intel, AMD, NVIDIA on Linux and Windows), CUDA, HIP, SYCL, Metal | CUDA, Metal. No Vulkan. | CUDA, Metal. No Vulkan. |
| Windows | Yes (MSVC, Vulkan) | Yes | Yes (CPU) |
| New architectures | Qwen3.5 (Gated DeltaNet) and Gemma 4 supported at release | Qwen3.5 not listed | Qwen3.5 and Gemma 4 listed |
| Constrained output | GBNF grammars; `llguidance` feature | No | Yes |
| LoRA at run time | Yes | Manual | Yes, per request |
| Crate status | `llama-cpp-2` 0.1.158, 2026-09-30; features include `vulkan`, `dynamic-backends`, `llguidance`, `mtmd` ([docs.rs](https://docs.rs/crate/llama-cpp-2/latest/features)) | `candle-core` 0.11.0, 2026-06-26 | `mistralrs` 0.8.1 on crates.io, 2026-04-02 |
| Build | Needs CMake and a C++ compiler | Pure cargo | Pure cargo, large dependency graph |
| Security | C++ GGUF parser with a history of overflow CVEs: CVE-2025-53630, CVE-2025-49847, CVE-2026-27940 (escalated to code execution), CVE-2026-33298 ([llama.cpp advisories](https://github.com/ggml-org/llama.cpp/security)) | Memory-safe | Memory-safe |

**Decision: llama.cpp through `llama-cpp-2`**, built with `dynamic-backends`
(`GGML_BACKEND_DL` + `GGML_CPU_ALL_VARIANTS`, `GGML_NATIVE=OFF`) and pinned to a llama.cpp
build that includes every advisory fix. It wins on the two things that matter for Lulo: CPU
speed on old x86, and Vulkan on both Linux and Windows. whisper.cpp also uses ggml, so speech
can share the same kernels. We vendor and pin it; we do not use Ubuntu 26.04's
`libggml0` 0.9.11 ([packages.ubuntu.com](https://packages.ubuntu.com/search?keywords=libggml&searchon=names&suite=all&section=all)),
because model architectures change faster than an LTS can follow.

The parser risk is contained in three ways: the service only opens files whose SHA-256
matches the built-in manifest; it runs as its own sandboxed process; and llama.cpp
advisories are added to `docs/dependency-policy.md` reviews. The engine sits behind a small
`Engine` trait in `rmac-intelligence`, so candle or mistral.rs can replace it later if
memory safety outweighs speed. Binary sizes (the CPU variants and the Vulkan backend's
shaders) are measured in phase 0. The Vulkan backend is a separate `.so`/`.dll`, loaded only
when calibration shows the GPU is faster than the CPU. On an HD 5500 it probably is not.

## 4. Architecture

### Process and IPC

```
app (Notes, Spotlight, …) ──D-Bus──▶ org.rmac.Intelligence1 ──▶ rmac-intelligence-service
   rmac-intelligence client          (D-Bus activated,             llama.cpp, model mmap,
   via rmac_dbus::session()           systemd user unit)            no network
```

- **A separate process; the model never runs inside an app.** It follows the
  existing pattern: a zbus `#[interface]` service (as in
  `crates/rmac-clipboard-linux/src/service.rs`), a `Type=dbus` user unit
  `crates/rmac-session/units/rmac-intelligence.service`, and a D-Bus activation file
  `org.rmac.Intelligence1.service.in` with `SystemdService=` (as in
  `crates/rmac-focus-linux/install/org.rmac.Focus1.service.in`). It is **not** in
  `rmac-session.target`'s `Wants=`, so nothing starts at login. Clients use the shared
  connection from `rmac-dbus`.
- **Interface** `org.rmac.Intelligence1` at `/org/rmac/Intelligence1`:
  `State` property (`Off | Unavailable | NotDownloaded | Downloading | Ready | Loading | Working`),
  `Run(task: s, args: a{sv}, text: h) -> o`, `Cancel(o)`, and the signals `Delta(o, s)` and
  `Done(o, s outcome, s json)`, sent only to the caller. `task` is a **closed enum**
  (`proofread`, `rewrite`, `summarise`, `explain_command`, `intent`, `help_answer`, `plan`).
  There is no free-form prompt API for other processes. Text arrives in a sealed memfd, so
  large inputs never pass through the bus.
- **Caller checks:** the caller must be the same user (the `authenticated_sender` pattern),
  and its executable must be a Lulo binary under `/usr/libexec/rmac` or `/usr/bin`. The path
  check is defence in depth, not a security boundary.
- **Threads:** physical cores only (2 on the reference laptop), with `CPUWeight=50`, so the
  UI wins any contention. Model loads use `IOSchedulingClass=idle`.

### Load, unload and memory

- **Load on first request.** The model is mmapped. Spotlight and voice start loading as soon
  as the panel opens or the talk key is pressed, so the load overlaps the user's typing or
  speech. Each task's fixed system prompt is evaluated once per load and its state cached,
  so a request only processes the user's own tokens.
- **Unload** 60 s after the last request, or 60 s after a client's lease ends (an open
  Writing Tools panel holds a lease). The process **exits** 5 s after unloading. Idle cost is
  then exactly zero: no process, no RSS, no wake-ups. D-Bus activation starts it again.
- **Unload at once** on memory pressure (a kernel PSI trigger on `/proc/pressure/memory`,
  which is event-driven, not polled), on screen lock, and on logind `PrepareForSleep`.
- **Limits:** `MemoryHigh` is the tier's budget (§4 gate), `MemoryMax` is the budget plus
  25 %, and `MemorySwapMax=0`, so a runaway model is killed instead of swapping the desktop
  to a crawl. Phase 0 checks that Ubuntu 26.04's user manager delegates the memory
  controller. Before each load the service also requires `MemAvailable` ≥ budget + 768 MiB;
  if not, the UI says "Not enough free memory right now" and nothing loads.

### Hardware gate (adapts to each PC)

| Tier | Model | Static requirements | Measured requirement |
|---|---|---|---|
| Not offered | — | x86-64 without AVX2, FMA, F16C and BMI2 (the packaged llama.cpp baseline), fewer than 2 physical cores, or MemTotal < 3.5 GiB | — |
| **Tiny** (default, and the reference laptop) | Qwen3.5-0.8B | AVX2 (or NEON), MemTotal ≥ 3.5 GiB | Tiny decode ≥ 10 tok/s, otherwise "not available on this PC" |
| Standard (offered, never the default) | Qwen3.5-2B | MemTotal ≥ 6 GiB, ≥ 2 physical cores | 2B decode ≥ 8 tok/s: measured on 2B when it is on disk, otherwise predicted as 0.51 × the Tiny rate |
| Enhanced (opt-in, later phase) | Qwen3.5-4B | MemTotal ≥ 12 GiB and ≥ 4 cores, or a Vulkan GPU with ≥ 4 GiB VRAM | decode ≥ 6 tok/s |

Revised after phase 0 (`crates/rmac-intelligence/src/gate.rs`). Phase 0 measured the 2B model at
7.3 tok/s and 0.8B at 14.4 tok/s on the reference laptop's 2 cores: decode is memory-bandwidth
bound, so 2B runs at about 0.51× the 0.8B rate on any PC. The Standard floor is phase 0's own 2B
budget, 8 tok/s; the reference laptop (predicted 7.3) therefore gets Tiny and is not offered
Standard, which phase 0 recommended. Settings measures the Tiny rate once, after the download
(`Calibrate`, 32 decode steps), and keeps it with the CPU model and memory size it was measured
on; new hardware makes it stale.

Facts come from `/proc/cpuinfo` (physical cores from `core id`, the CPU flags) and
`/proc/meminfo`. Settings always explains why a tier was chosen ("This PC has 6.7 GB of memory;
Lulo uses the Tiny model, which answers fastest here").

### Model download and storage

- **Not in the main `.deb`.** Bundling would add 1.3 GB for everyone, including PCs that
  never enable the feature or cannot run it. Settings ▸ Lulo Intelligence offers
  "Download (1.3 GB)" after the hardware check, and warns first on a metered connection
  (NetworkManager's `Metered` property).
- **The fetcher is its own process:** `rmac-intelligence-fetch` is a one-shot helper and the
  only intelligence component allowed to use the network. It downloads from a
  **revision-pinned** URL (Hugging Face `resolve/<commit>/<file>`, for example Qwen3.5-2B at
  `15852e8c…`), with a mirror on Lulo's GitHub release (under the 2 GiB per-asset limit). It
  resumes with HTTP Range, streams a SHA-256 over the file and checks its size, then renames
  atomically. The manifest (URL, size, SHA-256, licence) is compiled into
  `rmac-intelligence`, so it is covered by the signed apt repository.
- **Storage:** `$XDG_DATA_HOME/lulo/intelligence/models/<sha256>.gguf` (mode 0600), with the
  licence text beside it. Re-hashing 1.3 GB takes seconds on an old laptop, so later loads
  check a (size, mtime, inode) stamp and re-hash only when it changes. An optional
  `lulo-intelligence-model-standard` `.deb` installs to `/usr/share/lulo/intelligence/` for
  offline and OEM images; it is never a dependency. "Remove Model" frees the space.

### Windows (ADR 0023)

The same service binary, `lulo-intelligence.exe`. The transport is a named pipe,
`\\.\pipe\lulo-intelligence-<user SID>`, with the same messages, following ADR 0023's
`AppInstance` pipe and its plan for `rmac-app-menu`. When the pipe is missing, the client
starts the process. A Job Object enforces `JOB_OBJECT_LIMIT_PROCESS_MEMORY` and kills the
process on close. Models go to `%LOCALAPPDATA%\Lulo\Intelligence\models`. The service runs
in an **AppContainer without the `internetClient` capability**, which blocks the network at
the OS level. The fetcher uses WinHTTP. GPU offload uses the same Vulkan backend. OCR uses
`Windows.Media.Ocr` and TTS uses `Windows.Media.SpeechSynthesis`.

## 5. Privacy and safety

- **No network in the model process.** On Linux: `RestrictAddressFamilies=AF_UNIX`
  (seccomp, which works in user units), `NoNewPrivileges`, `ProtectSystem=strict`, and no
  HTTP or TLS crates in its dependency graph (a `deny.toml` ban scoped to the crate).
  `PrivateNetwork=` is **not** relied on: Ubuntu's `apparmor_restrict_unprivileged_userns`
  breaks namespace sandboxing in user units
  ([Ubuntu spec](https://discourse.ubuntu.com/t/spec-unprivileged-user-namespace-restrictions-via-apparmor-in-ubuntu-23-10/37626),
  [example failure](https://github.com/NVIDIA/OpenShell/issues/1895)).
- **Off by default.** Nothing downloads, listens or loads until the user turns it on in
  Settings ▸ Lulo Intelligence (the Mac's "Apple Intelligence & Siri" pane). Turning it off
  stops the service and offers to remove the model.
- **Clear UI states** wherever the feature appears: *Not available on this PC* (with the
  reason), *Download required*, *Downloading 42 %*, *Preparing…* (loading), *Working…* (with
  Stop), and *Done*. Each panel's footer says "Processed on this PC." The cloud option (§9)
  says "Sent to <provider>" in a different colour.
- **Nothing is applied automatically.** Writing Tools shows a preview with **Replace**,
  **Copy** and **Discard**, and Replace is a single undo step. Terminal explanations never run
  anything. A Settings change asked for in plain language appears as a Spotlight row ("Turn
  On Dark Mode") and takes effect only when the user presses Return or clicks it. Changes
  that affect security or connectivity (firewall, lock screen, sharing, forgetting a
  network) open the pane instead of changing anything.
- **No retention.** Prompts and outputs are not logged. The agent's action log (§9) records
  actions, not content. There is no telemetry.

## 6. Fine-tuning (approved; planned phase 2)

### Dataset, generated from the repository

A script, `scripts/intelligence/gen_dataset.py`, reads Lulo's own sources and writes JSONL in
the Qwen3.5 chat template, with tool schemas. No user data is used, and no Mac captures
(they never enter git).

| Task | Source in the repo | Generation | Size |
|---|---|---|---|
| Settings intents and parameters | `system_settings_entries()` (pane ids, titles, keywords); the typed operations in `rmac-quick-settings-system`; the pane controllers in `crates/system-settings/src/controller/*`; `tests/inventory/lulo/System Settings.json` | Templates plus teacher paraphrases (formal, casual, misspelt, voice-style); negatives ("none") and requests that must only open a pane | ~8k |
| App menu commands | The menu tables in `crates/rmac-app-menu/src/lib.rs` (label, action id, shortcut); `tests/inventory/lulo/*.json` | "In Notes, make a checklist" → `{app, action}` | ~4k |
| Help and docs | `shell/assets/help/*.md`; the user-facing pages in `docs/` (for example `install.md`, `launcher.md`; internal audits excluded) | Question → answer with a citation, from retrieved chunks | ~2k |
| Terminal explanations | The commands Ubuntu installs by default, grounded in their `--help`/`man` text at generation time; [tldr-pages](https://github.com/tldr-pages/tldr) (CC BY 4.0, which needs attribution in NOTICE) | Teacher explains; output never copies man text verbatim | ~3k |
| Writing tools | Teacher-written original paragraphs; **Proofread** pairs are made by injecting errors deterministically into clean text, so the label is exact | Rewrite/tone/summary pairs from the teacher | ~4k |
| Agent plans (phase 4) | The action registry's schemas (§9) | Multi-step plans with references between steps | ~3k |

- **Teacher:** Qwen3.6-35B-A3B (Apache-2.0 on Hugging Face, 2026-04-15), run on the rented
  GPU. Outputs of an Apache-licensed model carry no use restrictions.
- **Checks:** every target is parsed into the Rust registry types by a test binary
  (`rmac-intelligence-dataset-check`), so a misspelt pane id or action fails generation;
  every flag mentioned in a terminal explanation must appear in that command's `--help`;
  near-duplicates are removed (MinHash); 200 random samples are reviewed by hand per release.
- **Held-out evaluation set**, frozen by hash: about 1,500 examples, split **by item** (whole
  panes, apps and commands are held out, so the set measures generalisation, not memory);
  300 hand-written requests in the owner's own phrasing; and an adversarial set (injected
  instructions inside documents and email text, destructive requests, out-of-scope requests).
  Release gates: intent exact match ≥ 95 % on seen items and ≥ 85 % on unseen items;
  wrong-action rate (a confident wrong intent that would run) ≤ 0.5 %; zero tool calls
  triggered from injected content; Writing Tools judged as not worse than the base model by a
  larger local judge model, with exact match on Proofread.

### Recipe (Qwen3.5-2B; the same data trains 0.8B)

LoRA rank 16, alpha 32, dropout 0.05, on every linear projection (attention, Gated DeltaNet
and MLP); learning rate 1e-4 with cosine decay; 2–3 epochs; maximum sequence length 1,024;
effective batch 16; loss on assistant tokens only; non-thinking template. QLoRA (a 4-bit
base) on the Mac; bf16 LoRA on a GPU.

### Compute options

- **(a) The owner's M2 Mac, 8 GB, with MLX-LM.** This is feasible for 0.8B and 2B as
  **QLoRA**: batch 1–2, `--grad-checkpoint`, `--num-layers 16`, `--max-seq-length 1024`.
  Reported peaks are 3.9 GB for Qwen3.5-0.8B and about 5.9 GB for Qwen3.5-2B with a
  full-precision base ([sciences44/mlx-lora-finetune](https://github.com/sciences44/mlx-lora-finetune));
  a 4-bit 3B base peaks at about 5 GB ([InsiderLLM](https://insiderllm.com/guides/fine-tuning-mac-lora-mlx/)).
  So a 4-bit 2B base should fit in 8 GB with other apps closed. 4B does not fit. Training
  runs at about 200–250 tok/s on an M2, so ~24k examples × ~250 tokens (6M tokens per epoch)
  take about 7 h per epoch. A 2B run is one or two nights; 0.8B is two to three times
  faster. **Disk** (about 19 GB free): the venv ~1.5 GB, the bf16 base ~4.5 GB, a 4-bit copy
  ~1.3 GB, adapters < 0.1 GB, a fused bf16 model ~4 GB, a Q8_0 GGUF ~2 GB and the final
  Q4_K_M 1.3 GB. The peak is about 13 GB, so it fits only if each intermediate is deleted
  before the next step. Training uses the Mac for hours, so it runs only when the owner
  agrees (overnight).
- **(b) A rented GPU** (one H100 80 GB, about $1.50–$4 an hour on Vast, RunPod or Lambda in
  October 2026; [Akash price survey](https://akash.network/the-bid/h100-rental-price-2026-cost-per-hour/)).
  Teacher data generation takes 2–3 h. LoRA for 2B and 0.8B, three sweeps each, takes under
  30 min a run with Unsloth; one run of the same kind is reported at under 18 minutes for
  Qwen3-4B on one H100 ([Spheron](https://www.spheron.network/blog/llm-fine-tuning-cost-2026-api-vs-renting-gpus/)).
  Conversion and evaluation take about 1 h. **Total: about 6–8 GPU-hours, roughly $15–35; we
  propose a $60 cap.** The spend needs the owner's account and payment.

**Recommendation:** generate data with the teacher on the rented GPU (option b); train the
first adapters there too, because it is fast and repeatable; and use the Mac (option a) for
cheap iterations on 0.8B.

### Merge, quantise, ship

1. Merge the adapter into the bf16 base (`mlx_lm.fuse --dequantize`, or PEFT
   `merge_and_unload`).
2. Convert with llama.cpp's `convert_hf_to_gguf.py`, then run `llama-quantize` to Q4_K_M,
   with an importance matrix computed on the Lulo dataset.
3. Publish `lulo-2b-v1-Q4_K_M.gguf` and `lulo-0.8b-v1-Q4_K_M.gguf`, with their SHA-256 in the
   manifest. We ship **merged** models (no LoRA overhead at run time) and keep the adapter
   GGUF for experiments.
4. **Licence obligations (Apache-2.0):** ship the licence text; keep any NOTICE; state that
   the model was modified ("fine-tuned for Lulo by the Lulo project"); include a model card.
   Use a descriptive name that does not imply endorsement ("Lulo 2B, based on Qwen3.5-2B"),
   because Apache-2.0 grants no trademark rights. Add the tldr-pages CC BY 4.0 attribution
   if tldr data is used.

## 7. Spotlight, the Lulo panel and the design rule

Spotlight stays a search field. Intents appear as ordinary result rows next to the
deterministic ones, after a short typing pause (250 ms; 150 ms since phase 1.1), and only when the query reads like a sentence
(three or more words) with no strong deterministic hit. They never delay the deterministic
results. Conversational requests (help questions, voice, agent tasks) go to a separate small
**Lulo panel**, which works like Type to Siri: the same HUD as voice, with a text field.

## 8. Voice: "Hey Lulo"

### Pipeline

```
talk key held (or wake word) → mic (PipeWire / WASAPI) → VAD → streaming STT
  → rule parser ─┬─ hit → typed action
                 └─ miss → LLM intent (grammar-constrained) → typed action → confirm if needed
  → short reply: HUD text + TTS
```

### Components (permissive licences only)

| Stage | Choice | Licence | Cost on low-end x86 | Notes |
|---|---|---|---|---|
| Runtime | [sherpa-onnx](https://lib.rs/crates/sherpa-onnx) (official Rust API) | Apache-2.0 | ONNX Runtime ~15–20 MB library, loaded only in `rmac-voice` | Also provides keyword spotting, VAD and TTS. Windows supported. |
| VAD | Silero VAD | MIT | < 1 % of one core | Detects the end of speech in hands-free mode. |
| STT (English) | **Moonshine v2 Small** (123M); Tiny (34M) for the Tiny tier | MIT for the English models ([repo](https://github.com/moonshine-ai/moonshine)) | Linux x86 latency 165 ms (Small) and 69 ms (Tiny); WER 7.84 % and 12.01 % ([Moonshine v2 paper](https://arxiv.org/html/2602.12241v1)) | **Flag:** Moonshine's older non-English models use the non-commercial "Moonshine Community Licence" and must not ship. |
| STT (other languages) | whisper.cpp `base`/`small`, quantised | MIT (code and Whisper weights) | `small` ~2.9 s encode on a Ryzen 3900X ([whisper.cpp #89](https://github.com/ggml-org/whisper.cpp/issues/89)), so expect several seconds on the reference laptop | Shares ggml with the LLM. Offered only on the Standard tier and above. |
| Wake word (opt-in) | A custom **"Hey Lulo"** [openWakeWord](https://github.com/dscripka/openWakeWord) model, trained by us | Code Apache-2.0. **Flag:** its *pre-trained* models are CC BY-NC-SA 4.0 and are not used. Our model is ours, if the training data is commercial-safe. | 15–20 models in real time on one Raspberry Pi 3 core, so one model costs very little; budget below | Train on synthetic positives from Apache/MIT TTS voices (Kokoro voices; Piper voices only where the voice's own licence permits) and negatives from CC0/CC BY corpora (Common Voice, MUSAN). Targets: false rejects < 5 % and false accepts < 0.5 per hour (openWakeWord's own targets). Check the licence of openWakeWord's shared feature-extractor models before shipping; if they fail, the fallback is sherpa-onnx's open-vocabulary keyword spotting, which needs no training. |
| TTS (default) | Linux `spd-say` (already used by `crates/rmac-ui/src/speech.rs`); Windows `SpeechSynthesis` | System packages | ~0 | Nothing to download. |
| TTS (natural, optional) | Kokoro-82M via sherpa-onnx | Apache-2.0 ([model card](https://huggingface.co/hexgrad/Kokoro-82M)) | ~100–300 MB RAM while speaking | **Flag:** phonemisation uses espeak-ng (GPL-3.0). Run it as a separate system `espeak-ng` process; never link it into Lulo's MIT binaries. **Piper** is rejected: the maintained repository is GPL-3.0 ([OHF-Voice/piper1-gpl](https://github.com/OHF-Voice/piper1-gpl)) and voice licences vary. |

### Defaults

- **Hold-to-talk is the default**, with zero idle cost: no process and no open microphone
  until the key goes down. The key is set in Settings like the Mac's Siri "Keyboard shortcut"
  pop-up. The default is the Copilot key on keyboards that have one, otherwise "hold ⌥Space".
  Spotlight's ⌘Space press is unchanged. Press and release come from the portal
  GlobalShortcuts `Activated`/`Deactivated` signals that `rmac-shortcuts` already handles
  (`crates/rmac-shortcuts/src/portal.rs`). The niri fallback binding sees only the press, so
  there the end of speech is detected by VAD.
- **The "Hey Lulo" wake word is opt-in**, offered on the Standard tier and above, and only
  after the menu-bar microphone indicator exists. That indicator is BAR-09 in
  `docs/parity.md`, currently Missing, and is a **hard prerequisite**. A small separate
  process, `rmac-wake` (audio, VAD and the wake model only), pauses while the screen is
  locked, in Low Power Mode and while the mic is muted. Audio stays in memory, is never
  written to disk, and never leaves the PC.

### Action layer (shared with Spotlight intents and agent mode)

A typed registry in `rmac-intelligence`: each action has a Rust type, a JSON schema generated
from it (for constrained decoding), a **tier**, and an executor that calls the service Lulo
already uses for that job:

| Action | Existing route | Tier |
|---|---|---|
| Open an app or a Settings pane | `rmac_launcher::Action::{LaunchApplication, OpenSetting}` → `rmac-launcher-system::execute` | Safe: runs |
| Search files | The `rmac-launcher-providers` Files provider / `rmac-search` (`Operation::SearchFiles`) | Read-only: runs |
| Timer or alarm | `rmac_clock::store` (locked read-modify-write) + `rmac_clock::schedule` (the systemd timer that rings without Clock running) | Safe and undoable: runs |
| App menu command | `rmac_app_menu::activate(app_id, action)` over `org.rmac.AppMenu1/2`, the route the menu bar uses; `AppMenu2` validates enabled state first | Safe if the item is listed as safe; otherwise confirm |
| Focus on/off | `org.rmac.Focus1` | Safe: runs |
| Settings change (toggles, volume, brightness, appearance) | `rmac-quick-settings-system::execute` | **Confirm** |

### Voice latency budgets (reference laptop)

- Hold-to-talk, key release → final transcript ≤ 400 ms.
- Key release → a safe action done, or a confirmation shown: ≤ 600 ms by the rule parser;
  ≤ 1.5 s with the LLM warm; ≤ 3.5 s cold (the model starts loading at key press).
- Wake word → listening HUD ≤ 300 ms; end of speech → spoken reply starts ≤ 2.0 s warm.
- Wake word idle cost ≤ 2 % of one core averaged over 10 minutes, ≤ 60 MiB RSS.
  Hold-to-talk idle cost: zero.

## 9. Agent mode

The owner wants an agent like OpenClaw: "find last week's invoice, email it to X, remind me
tomorrow".

### What OpenClaw is, and what went wrong

OpenClaw (formerly Clawdbot, then Moltbot) runs an always-on **Gateway** daemon (default port
18789). The Gateway bridges chat channels (Telegram, Slack and others) to an agent loop: call
the model, run tools, feed the results back, repeat. It wakes on a heartbeat every 30 minutes
([Milvus guide](https://milvus.io/blog/openclaw-formerly-clawdbot-moltbot-explained-a-complete-guide-to-the-autonomous-ai-agent.md),
[Markaicode](https://markaicode.com/architecture/openclaw-tool-calling-architecture/)).
Skills are `SKILL.md` files of natural-language instructions, shared through the ClawHub
registry, which the agent can search and install by itself. Shell commands go through
"exec approvals" ([docs](https://docs.openclaw.ai/tools/exec-approvals)). Documented
incidents in early 2026:

- **Exposed instances:** about 1,000 Gateways reachable through Shodan with no
  authentication. One researcher obtained API keys, bot tokens and months of chat history
  ([Kaspersky](https://www.kaspersky.com/blog/openclaw-vulnerabilities-exposed/55263/)).
- **CVE-2026-25253 (CVSS 8.8):** a one-click remote code execution. The Control UI trusted a
  `gatewayUrl` query parameter and sent the auth token to the attacker. It worked even with
  the Gateway bound to loopback ([SOCRadar](https://socradar.io/blog/cve-2026-25253-rce-openclaw-auth-token/)).
- **Malicious skills:** the ClawHavoc campaign, 335 of 341 malicious ClawHub skills
  ([Antiy](https://www.antiy.net/p/clawhavoc-analysis-of-large-scale-poisoning-campaign-targeting-the-openclaw-skill-market-for-ai-agents/)).
  Snyk's ToxicSkills study found flaws in 36.8 % of 3,984 skills and 76 confirmed malicious
  payloads, 91 % of which also used prompt injection
  ([Snyk](https://snyk.io/blog/toxicskills-malicious-ai-agent-skills-clawhub/)).
- **Prompt injection:** an email made the agent reveal its config and keys; a Google Doc made
  it add an attacker's Telegram bot as a persistent control channel
  ([The Register](https://www.theregister.com/2026/02/05/openclaw_skills_marketplace_leaky_security/)).

These are Simon Willison's "lethal trifecta": access to private data, exposure to untrusted
content, and a way to communicate externally
([Willison](https://simonwillison.net/2025/Jun/16/the-lethal-trifecta/)).

**Lulo adopts:** a loop of model, tools and results; skills as described tools; approval
before risky execution; local-first operation.
**Lulo rejects:** an always-on daemon or network listener (Lulo uses only the session bus, and
is started on demand); remote chat channels; heartbeat or self-started runs (the agent acts
only on a user request); skills written as natural-language files; a skill marketplace or
self-installed skills; and secrets in the model's context.

### Lulo's design

- **Built-in, vetted skills only.** Each skill is Rust code in the action registry (§8),
  reviewed like any other code. No third-party skills in phase 4; a marketplace is out of
  scope.
- **Skills by app**, built in this order: Files (search, reveal, move to Trash), Calendar
  (read; create an event, via `rmac-calendar-store`/`calendar-agent`), Clock, Settings, Notes
  (create, append, via `rmac-notes-store`), Mail (find a message; compose with attachments
  through the compose window in `crates/mail/src/compose_window.rs`/`mailto.rs`), app menus,
  and **Terminal last** (§ tiers). "Remind me" needs Reminders (APP-02, Missing); until
  then it becomes a Calendar event with an alert, or a Clock alarm.
- **Plan, then act** (after [CaMeL](https://arxiv.org/abs/2503.18813)): the planner sees only
  the user's trusted request and the tool schemas, and outputs a typed plan. Arguments can
  refer to earlier steps' outputs (`$1.files[0]`). The user sees the plan before anything
  runs.
- **Untrusted content is data, never instructions.** Email bodies, file contents and web text
  are returned as **opaque, tagged values** that the planner never reads. When a value must be
  read (for example, "the date in this email"), a *quarantined* call of the same model, with
  no tools, extracts it into a narrow type (a date, an enum, a file handle). It cannot return
  free text that becomes a plan. A data-flow policy then blocks private values from flowing to
  an outbound sink (Mail send) unless the recipient came from the user's own words or
  contacts, never from content. Every outbound step shows the exact recipient and attachment.
- **Permission tiers:**
  - **Runs:** read-only actions (search, read the calendar, list windows) and safe, reversible
    local actions (open an app, start a timer, create a note).
  - **Confirm, showing the exact action:** sending, deleting (to the Trash), changing Settings,
    overwriting files, inviting others.
  - **Confirm every time, in a visible Terminal tab:** shell commands, shown verbatim with
    an explanation; never with `sudo`. Built last.
  - **Never:** reading passwords or keyrings, polkit/admin actions, installing software,
    permanent deletion, turning off the lock screen or firewall.
- **Action log with undo.** `$XDG_STATE_HOME/lulo/intelligence/actions.jsonl` (mode 0600)
  records each executed step's summary (not content), and Settings ▸ Lulo Intelligence ▸
  Activity shows it. Undo where the tool supports it: a Settings toggle returns to its
  previous value, Trash restores the file, a timer is cancelled, a note or event is deleted.
  Mail cannot be unsent, which is why sending always asks first.

### Model capability, and the optional cloud model

On BFCL v4, Qwen3.5-0.8B scores 25.3 %, 2B 43.6 %, 4B 50.3 % and 9B 66.1 %
([llm-stats BFCL-v4](https://llm-stats.com/benchmarks/bfcl-v4)). For Qwen3-4B, multi-turn
accuracy is far below single-turn (35 % against 82 % non-live AST in one evaluation,
[arXiv 2508.05118](https://arxiv.org/html/2508.05118v4)). Conclusion: the local 2B, made
reliable by grammar-constrained decoding against our own schema and by fine-tuning, is good
enough for **single-step intents**. **Multi-step planning** needs a stronger model. So:

- **Local default:** single-step requests and short, fixed-shape plans (2–3 steps from
  fine-tuned patterns). On Enhanced PCs, 4B may plan more; phase 4 measures this against
  the held-out agent set.
- **Bring your own cloud model** (off by default): the user adds their own API key (stored in
  the Secret Service keyring; Credential Manager on Windows) for an OpenAI-compatible or
  Anthropic endpoint. A separate process, `rmac-intelligence-cloud`, is the only component
  with network access. Only the **planner** goes to the cloud: the request text and tool
  schemas, never file contents or email bodies, which stay as local opaque handles. The UI
  marks every cloud step "Sent to <provider>" in a distinct colour, and Settings shows exactly
  what is sent.

## 10. Phased plan

| Phase | Content | Exit criteria |
|---|---|---|
| **0. Spike** (about 1 week) | A non-shipping `rmac-intelligence-spike` bin with `llama-cpp-2` CPU and Vulkan builds. Qwen3.5-2B/0.8B and Granite 4.0 1B as a control, on the reference laptop. Measure binary sizes, cgroup memory delegation and the calibration logic. A second step (not run at the same time, per `AGENTS.md`) measures Moonshine v2 Small/Tiny and openWakeWord CPU cost. | **Budgets (pass/fail):** zero idle cost (no process, no RSS, no wake-ups when off, and 65 s after the last use); cold start to first token ≤ 3.0 s (2B) / ≤ 1.5 s (0.8B) with a warm page cache, ≤ 6 s cold; warm intent first token ≤ 400 ms with the cached prefix; Writing Tools first token for 200 words ≤ 2.5 s; decode ≥ 8 tok/s (2B) / ≥ 20 tok/s (0.8B) on 2 threads; prefill measured (a 1,500-token summary should take ≤ 15 s); peak RSS ≤ 1.9 GiB (2B) / ≤ 1.0 GiB (0.8B) at 4K context; no new frame over 16.7 ms in Notes during generation (`run_frame_timing.py`); Spotlight typing p95 unchanged. If 2B misses 6 tok/s, the reference laptop's tier drops to Tiny. |
| **1. Service and first features** | `rmac-intelligence`, the service, the fetcher, D-Bus activation, the Settings pane (off by default, hardware gate, download, states); Spotlight intents (feature 1) with the confirmation rows; Writing Tools in Notes, Text Editor and Mail (feature 2). Linux only. Behaviour scenarios in `tests/behavior/` with a fake engine. | Phase 0 budgets hold in nested runs; with the feature on and unused, idle cost is still zero. |
| **2. Fine-tune and more features** | The dataset, the evaluation set, LoRA v1 (§6); ship `lulo-2b-v1`/`lulo-0.8b-v1`; OCR (tesseract) in Preview and Quick Look; Terminal Explain; summaries; Help answers. | Evaluation gates in §6 met; the fine-tuned model does at least as well as the base on every task. |
| **3. Voice** | `rmac-voice` with hold-to-talk; the action registry's safe and confirm tiers; HUD; `spd-say` replies. Then BAR-09 (mic indicator); then the opt-in wake word and Kokoro voice. | The voice budgets in §8 on the reference laptop; a false-accept soak (8 h of TV and talk audio) ≤ 0.5/h. |
| **4. Agent mode** | The planner, quarantined extraction, the data-flow policy, the action log and undo; skills Files → Calendar → Clock → Settings → Notes → Mail → menus; then the optional cloud planner; Terminal skill last. | The adversarial set: zero injected actions; every outbound or destructive step confirmed in behaviour tests. |
| **5. Windows** (alongside ADR 0023 phases 2–3) | Named-pipe transport, AppContainer, Job Object limits, WinHTTP fetcher, `Windows.Media.Ocr`/`SpeechSynthesis`, WASAPI capture. | The same budgets on the Windows test PC. |

## Phase 0 results (2026-10-07, measured)

Measured on the reference laptop (`jacob@192.168.18.52`, i5-5300U, 2 physical
cores / 4 threads, AVX2+FMA, 6.7 GiB RAM, Ubuntu 26.04 LTS "Resolute
Raccoon", systemd 259) once the owner cleared the build cache (114 GB free
at the start, 105 GB free at the end — the models and build are kept, not
deleted, per the "delete only below 15 GB free" rule). An earlier attempt
the same day was correctly blocked and abandoned when `df -h /` showed only
2.3 GB free; that account is preserved in git history on this branch.

### Build: practical, with one gap

`tools/ai-spike` built and ran against **`llama-cpp-2` 0.1.158** (the exact
version the ADR names) with a plain CPU build (no `vulkan` feature — the
ADR itself expects the HD 5500 to lose to the CPU path, and Vulkan was not
built or measured in this pass; that is a gap for phase 1 if GPU offload is
ever reconsidered, not a blocker here). The C++ build (CMake + g++ 15.2.0,
compiling `ggml`/`llama.cpp` from source) was **practical**, no fallback to
candle/mistral.rs was needed: **but the laptop had no `cmake` installed and
no `pip`, and the task rules forbid `sudo`.** Worked around with Kitware's
portable `cmake-4.4.4-linux-x86_64.tar.gz` (sha256
`e5bb807f7728cb60cd8b27ebc97a2edb469b68655f21e844a600c3575b76f5bb`, verified
after download), unpacked under `~/rmac-ai-spike/tools/` and prepended to
`PATH` for the build only — nothing installed outside the spike's own
directory. **Phase 1 should add `cmake` to the laptop's normal package set**
so the real service build does not need this workaround. The final
`ai-spike` binary is 5.9 MB; the static libs it links
(`libllama.a` 10.2 MiB, `libggml-base.a` 1.5 MiB, `libggml-cpu.a` 1.7 MiB,
plus a 14.8 MiB `libllama-common.a`) are from a **native-only** build, not
the ADR's `GGML_BACKEND_DL`+`GGML_CPU_ALL_VARIANTS` multi-variant build
that ships one binary fast on every x86 PC — that build's size is still
unmeasured and should be checked before phase 1 ships anything.

### Models downloaded and verified

| Model | Source (revision-pinned) | Size | SHA-256 |
|---|---|---|---|
| Qwen3.5-2B-Q4_K_M | `huggingface.co/unsloth/Qwen3.5-2B-GGUF/resolve/f6d5376be1edb4d416d56da11e5397a961aca8ae/Qwen3.5-2B-Q4_K_M.gguf` | 1,280,835,840 B (1.28 GB / 1.19 GiB) | `aaf42c8b7c3cab2bf3d69c355048d4a0ee9973d48f16c731c0520ee914699223` |
| Qwen3.5-0.8B-Q4_K_M | `huggingface.co/unsloth/Qwen3.5-0.8B-GGUF/resolve/6ab461498e2023f6e3c1baea90a8f0fe38ab64d0/Qwen3.5-0.8B-Q4_K_M.gguf` | 532,517,120 B (0.53 GB / 0.50 GiB) | `bd258782e35f7f458f8aced1adc053e6e92e89bc735ba3be89d38a06121dc517` |

Both sizes match the ADR's §2 table almost exactly, confirming these are
the right files. **Lesson for the real fetcher:** curl's own progress
percentage is not proof of a complete file — an interrupted/resumed
download (`curl -C -` across what turned out to be a re-signed CDN URL)
produced a 0.8B file that read back as "100% done" at the wrong size and
failed `llama_model_load` with "tensor data is not within the file
bounds". The fetcher must always compare the final byte count (and ideally
the SHA-256) against the manifest, never trust the downloader's own
completion signal. `bartowski/Qwen_Qwen3.5-2B-GGUF` remains a second
source if Unsloth's revision ever moves.

### Methodology notes (read before the numbers)

- **"Warm" here is not the ADR's cached-prefix warm.** This harness calls
  `ctx.clear_kv_cache()` before every prompt (required — see the
  architecture note below) and reprocesses the full system+user prompt
  from scratch every single time. There is no prompt-prefix cache in this
  minimal spike. So "ttft cold" and "ttft warm median" below are
  deliberately near-identical: they are both measuring **repeated full
  prefill**, which is the worst case the ADR's real design (§4, "each
  task's fixed system prompt is evaluated once per load and its state
  cached") exists specifically to avoid. Treat every TTFT number below as
  "without prefix caching"; it is not evidence against caching helping,
  it is the reason caching is necessary.
- **Architecture finding for §4's design:** Qwen3.5's hybrid Gated
  DeltaNet/attention layers keep a **recurrent memory module per sequence
  that requires strictly increasing positions**. Reusing one context
  across two *different* prompts without clearing it fails outright
  ("the tokens for sequence 0 ... have inconsistent sequence positions").
  This means §4's plan to cache **multiple** per-task system prompts and
  reuse them needs either one llama.cpp sequence ID per cached task (not
  just one shared context) or a separate context per task — a single
  shared KV cache holding several tasks' cached prefixes will not work
  as-is for this model family. Worth a line in §4 before phase 1's service
  is built.
- **TTFT = prefill time.** An early version of this harness measured only
  the post-prefill sampling step and reported TTFT near 0 ms, because
  llama.cpp's prefill decode already computes the first token's logits.
  Fixed before any of the numbers below were taken.
- **The base models think.** Despite the Unsloth docs describing
  non-thinking as the small Qwen3.5 models' default, this hand-built
  prompt (no HF `apply_chat_template`, no `enable_thinking=False`) gets a
  `<think>...</think>` block before every JSON answer. The first quality
  pass used a 64-token budget and silently truncated mid-thought, which
  looked like wrong answers but was really a budget problem; fixed by
  raising the quality harness's budget to 300 tokens. The **bench**
  numbers below still use the shorter, more realistic per-feature budgets
  (64/220/160 tokens) and so include real thinking-token cost — this is
  honest default-model behaviour, not a harness bug, and fine-tuning or an
  explicit non-thinking template flag (phase 1/2) should remove it.
- **The shared laptop was not reliably idle.** Twice during this run,
  unrelated `rustc` processes (another job on the shared box) pegged all
  4 threads mid-benchmark and visibly corrupted the numbers (prefill/decode
  dropping by 2–4×, one "warm slower than cold" inversion). Both
  contaminated runs were discarded and redone after confirming
  `ps -eo pcpu,comm | awk '$1+0>30'` was empty. The numbers below are from
  those clean, idle-CPU re-runs; the discarded runs are kept on disk
  (`bench-2b-run1.log`, `bench-08b-run1.log`) as a record of why the
  idle-CPU check matters.

### Measurements

Each `bench` invocation runs 3 reps per prompt internally and reports the
median, per the ADR's instructions; the table below is the clean re-run for
each model.

| Model | cold load | prompt | ttft cold | ttft warm (median, no cache) | decode (median) | prefill (median) | peak RSS |
|---|---|---|---|---|---|---|---|
| **2B** | 1.12 s | spotlight_intent | 5.50 s | 5.50 s | 7.3 tok/s | 25.5 tok/s | 1896 MiB |
| 2B | | writing_rewrite (~150 w) | 9.05 s | 9.05 s | 7.5 tok/s | 20.9 tok/s | 1901 MiB |
| 2B | | terminal_explain | 2.15 s | 2.15 s | 7.4 tok/s | 25.6 tok/s | 1901 MiB |
| **0.8B** | 0.85 s | spotlight_intent | 2.50 s | 2.44 s | 14.4 tok/s | 57.3 tok/s | 856 MiB |
| 0.8B | | writing_rewrite (~150 w) | 3.38 s | 3.38 s | 14.9 tok/s | 56.0 tok/s | 859 MiB |
| 0.8B | | terminal_explain | 1.01 s | 1.01 s | 15.0 tok/s | 54.6 tok/s | 859 MiB |

Unload (separate, dedicated run, same idle-CPU conditions): 2B load 1120 ms,
peak RSS 1896 MiB, RSS right after dropping the model/context/backend in
the same process 80 MiB; 0.8B load 768 ms, peak RSS 856 MiB, RSS after drop
80 MiB. In both cases the process then exits and no `ai-spike` process or
RSS remains (`pgrep` confirmed clean) — the strongest a one-shot CLI can
show toward the real service's "0 after exit" requirement; the 65-second
idle-unload timer itself is phase 1's D-Bus service behaviour, not
something this binary has.

**Quality (20-case intent set, closed tool schema, 300-token budget to let
thinking finish):** **2B: 17/20 (85%). 0.8B: 16/20 (80%).** Both models
missed only `open_setting` for two Settings-pane phrasings ("open bluetooth
settings", "i need to change my wallpaper" → both models answered
`open_app` instead) and "what's the capital of France" (0.8B tried to
`open_app` a browser for it; 2B correctly said `none`); 0.8B also missed
the third `open_setting` case 2B got right. Full transcripts: the
`PASS`/`FAIL` lines with raw model output are in `quality-2b.log` and
`quality-08b.log` under `~/rmac-ai-spike/` on the laptop. For context, this
is a much easier task than full BFCL tool-calling (single-field action
match against six intents, not multi-argument exact match), so these
numbers are not directly comparable to the ADR §9 BFCL figures
(0.8B 25.3%, 2B 43.6%) — they show the models can pick the right *action*
reliably even before fine-tuning, which the ADR's closed intent list (§1,
feature 1) is specifically designed around.

**Prefill at ~1,500 tokens:** not directly tested — the longest prompt used
here (`writing_rewrite`) is only ~200 tokens of input. Extrapolating from
the measured short-prompt prefill rates (2B ~21-26 tok/s, 0.8B ~55-57 tok/s)
gives roughly 2B 60-70 s and 0.8B 26-28 s for 1,500 tokens — both far over
the 15 s budget — but this is an extrapolation, not a measurement, and
prefill throughput is not always flat with length. A real long-prompt test
belongs in phase 1/2 before relying on this number.

### Budgets: pass/fail

| Budget (§10 phase 0 row) | 2B | 0.8B |
|---|---|---|
| Zero idle cost (no process/RSS when off) | **PASS** — process exits, 0 RSS/CPU confirmed (65 s idle-unload timer itself untested, belongs to phase 1's service) | **PASS** (same caveat) |
| Cold start to first token, warm page cache: ≤ 3.0 s (2B) / ≤ 1.5 s (0.8B) | **FAIL** (load 1.12 s + ttft 5.50 s = 6.6 s) | **FAIL** (0.85 s + 2.50 s = 3.3 s) |
| ... or ≤ 6 s truly cold (more lenient, and page cache was in fact warm here) | **FAIL** (6.6 s, i.e. over budget even though the page cache was warm) | **PASS** (3.3 s < 6 s) |
| Warm intent first token ≤ 400 ms with cached prefix | **Not measurable with this harness** — no prefix cache implemented (see methodology notes); raw repeated-prefill warm is the same as cold above, i.e. nowhere near 400 ms, which only shows caching is necessary, not that the target is unreachable |
| Writing Tools first token (~200 words) ≤ 2.5 s | **FAIL** (9.05 s, 3.6×) | **FAIL** (3.38 s, 1.4×) |
| Decode ≥ 8 tok/s (2B) / ≥ 20 tok/s (0.8B) on 2 threads | **FAIL** (7.3-7.5 tok/s — but above the ADR's own "drop to Tiny" trigger of < 6 tok/s) | **FAIL** (14.4-15.0 tok/s — but comfortably above the Tiny tier's own ≥ 10 tok/s hardware-gate floor, §4) |
| Prefill ≈1,500 tokens ≤ 15 s | **FAIL (extrapolated, not measured directly)** | **FAIL (extrapolated, not measured directly)** |
| Peak RSS ≤ 1.9 GiB (2B) / ≤ 1.0 GiB (0.8B) at 4K context | **PASS** (1901 MiB = 1.857 GiB, ~2 % margin) | **PASS** (859 MiB = 0.839 GiB, ~16 % margin) |
| No new frame > 16.7 ms in Notes; Spotlight typing p95 unchanged | **Not measured** — no app integration exists; this standalone CLI has no UI thread to regress |
| User-unit `MemoryMax`/`RestrictAddressFamilies` | **Untested; reasoned PASS** for both — cgroup `memory` controller delegated to `user@<uid>.service` (`Delegate=yes`, confirmed via `systemctl show`, no `systemctl --user` used); `RestrictAddressFamilies` is seccomp-based (`+SECCOMP` in `systemctl --version`) and independent of the cgroup/namespace issue (`apparmor_restrict_unprivileged_userns`) that breaks `PrivateNetwork=` on Ubuntu 26.04, which §5 already avoids |

**Overall: both models miss most of phase 0's own speed budgets on this
specific reference laptop, by margins ranging from small (0.8B's cold
start, 0.8B's decode vs. the Tiny-tier floor) to large (2B's Writing Tools
latency). Neither model fails outright; both sit in a degraded-but-usable
zone relative to the ADR's own fallback thresholds** (2B stays above the
"drop to Tiny" trigger; 0.8B clears the Tiny tier's own calibration floor
comfortably). RSS is fine for both. Quality, on this simplified intent
task, is good for both and close between them (17/20 vs 16/20).

### Recommendation

**Keep the two-tier design; narrow what "Standard" means on this exact
hardware, and treat prefix caching as the load-bearing fix, not an
optimisation.**

1. **Prefix caching (ADR §4) is not optional polish — build it first.**
   Every TTFT number above is a full, uncached prefill. The single biggest
   lever for every budget in this table (cold start, warm intent, Writing
   Tools) is caching each task's fixed system prompt once per load, which
   this spike deliberately does not implement. Phase 1 should treat this
   as a correctness requirement for the service, not a later optimisation,
   and should account for the recurrent-memory finding above (separate
   sequence IDs or contexts per cached task) when designing it.
2. **On the reference laptop specifically, 2B performs closer to a Tiny
   experience than a Standard one.** It clears the hardware gate's own
   enable-time calibration floor (≥ 6 tok/s decode, §4) by a narrow margin
   (7.3-7.5 measured) and gives meaningfully better quality on this test
   (17/20 vs 16/20, a smaller gap than the ADR's cited BFCL scores would
   suggest), but its Writing Tools and cold-start latency are 1.4-3.6×
   over budget. 0.8B is faster everywhere, comfortably passes RSS and the
   Tiny tier's own floor, and is barely behind on quality. **Recommend
   re-examining the Standard tier's CPU requirement (§4's hardware-gate
   table) specifically for 2-core/4-thread Broadwell-class CPUs like this
   one** — either raise the physical-core requirement for Standard so this
   class of hardware defaults to Tiny, or accept that "Standard" on this
   hardware means "usable but slower than the budgets assume" until
   prefix caching and/or fine-tuning close the gap. This is a tier-
   boundary rethink, not a rejection of Qwen3.5-2B/0.8B as the model
   family (§2's licence/quality reasoning there is untouched by anything
   measured here).
3. Re-measure after (1) and after fine-tuning (§6, phase 2) — both should
   move these numbers, and a cheap re-run of this exact harness (now that
   it builds and the models are already on the laptop) is the way to
   check.
4. Add `cmake` to the laptop's normal toolchain and measure the
   `GGML_BACKEND_DL`/`GGML_CPU_ALL_VARIANTS` dynamic build's size before
   phase 1 ships anything — this phase 0 build used a native-only build
   and a portable `cmake` binary as a workaround, neither of which should
   carry over unexamined into the real service build.

## Phase 1 (2026-10-07): the service, the prefix cache and Spotlight rows

Built on `op/ai-phase1`. Scope was cut to what could be proven well: the intent task only;
Writing Tools, voice and the Lulo panel stay in later phases.

### What was built

- **`rmac-intelligence`** (no model runtime): the closed task list (`Task::Intent` only; other
  names are refused), the typed `Intent` with its strict JSON wire form, row titles and
  confirmation tier, the prompt, the schema-guided decoder, the pinned manifest (size and
  SHA-256 of both GGUF files, revision-pinned URLs), the hardware gate, the settings file
  (`$XDG_CONFIG_HOME/rmac/intelligence.json`, off unless it says on; corrupt reads as off), the
  checksum-verified fetcher and the session-bus client (feature `client`).
- **`rmac-intelligence-service`** (`org.rmac.Intelligence1` at `/org/rmac/Intelligence1`):
  `Prepare()`, `Run(task, text) -> s` and `Calibrate() -> s`, and a `State` property. Phase 1
  answers `Run` directly instead of through the `Delta`/`Done` signals of §4: an intent is one
  short JSON reply, and Writing Tools will add streaming. Text arrives as a string capped at
  800 bytes (the prompt uses at most 200); the memfd path arrives with long inputs.
  - Activated on demand (`org.rmac.Intelligence1.service` → `SystemdService=
    rmac-intelligence.service`), never in `rmac-session.target`, never supervised.
  - Exits 60 s after its last call. The main loop waits on one deadline and the call queue;
    nothing polls. A request while turned off, unsupported, short of memory or without a model
    is answered with that error and the process exits a second later.
  - Re-reads the on/off setting on every request, so turning it off takes effect at once.
  - Caller check (revised 2026-10-08, see "The caller-check fix" below): one
    `GetConnectionCredentials` call gives the uid, pid and, where the bus has one, a pidfd;
    same uid only. The caller's `/proc/<pid>/exe` must then be one of an explicit list:
    `/usr/libexec/rmac/rmac-launcher` (Spotlight), `/usr/bin/rmac-system-settings`, or
    `rmac-launcher`, `rmac-system-settings` and the unpackaged `rmac-intelligence-bench` beside
    the service itself. No other `/usr/bin` program passes. Every refusal fails closed and
    logs the sender and the reason.
  - Loads only a model whose size and SHA-256 match the manifest; a verified-stamp (size,
    mtime, inode) avoids re-hashing on every load.
  - Requires `MemAvailable` ≥ the tier's budget + 768 MiB before loading.
  - The unit sets `MemoryHigh=2G`, `MemoryMax=2560M`, `MemorySwapMax=0`,
    `RestrictAddressFamilies=AF_UNIX`, `NoNewPrivileges=yes`, `CPUWeight=50`,
    `IOSchedulingClass=idle`, plus `LockPersonality`, `RestrictRealtime`,
    `SystemCallArchitectures=native` and `UMask=0077`, and no mount-namespace option (no
    `PrivateTmp`; see the caller-check fix below). One static unit cannot
    follow the tier, so the caps are the Standard tier's; Tiny stays far below them, and the
    free-memory gate uses the chosen tier's own budget. A per-tier drop-in is phase 2.
  - llama.cpp is built from source by `llama-cpp-2` 0.1.158 (CPU only, no OpenMP, no "common"
    library) on Linux only; macOS and Windows builds of the workspace never compile it.
    Packaged builds set an AVX2/FMA/F16C/BMI2 baseline (`build-native-inputs.sh`), which the
    hardware gate requires, so a package never runs the build machine's own instructions.
    `cmake` and `libclang-dev` are now declared build dependencies (CI, `release.yml`,
    `packaging/rmac-source/debian/control`). The models are never packaged.
- **Settings ▸ Lulo Intelligence** (SET-115): header, the on/off switch (off by default), the
  gate's verdict, the model (a pop-up only where Standard is allowed), the download row
  (Download / Stop / Resume at N % / Remove Model) driven by the one-shot
  `rmac-intelligence-fetch` helper, and the one-time speed check after a download.
- **Spotlight "Lulo can do this" rows** (`crates/launcher-app/src/view/assist.rs`): see below.

### Prompt-prefix caching (Qwen3.5's hybrid layers)

Phase 0 found that a Qwen3.5 context cannot be rewound by trimming the KV cache: its Gated
DeltaNet layers keep a recurrent state per sequence that only moves forward. The service
therefore saves the *whole* sequence state:

1. at load, the fixed prefix (system prompt, action schema, 19 worked examples; 693 tokens) is evaluated
   once on sequence 0;
2. `llama_state_seq_get_data_ext` captures sequence 0 — attention KV and recurrent state
   together — into memory;
3. every request clears the context, restores that state with
   `llama_state_seq_set_data_ext`, and evaluates only its own tokens from the prefix's end.

The same state is also written to `$XDG_CACHE_HOME/lulo/intelligence/prefix-<hash>.state`
(`llama_state_seq_save_file`), keyed by the model file, prompt version, prompt text, context
size and llama.cpp version. A later service start reads it back instead of evaluating the
prefix again, after checking that the saved tokens are exactly the prefix's. Separate
sequence ids per task (phase 0's other option) are not needed while there is one task.

### Schema-guided decoding

The answer always starts `{"intent":"`, written into the prompt. From there
`rmac_intelligence::decode` only lets the model choose what the schema allows: intent names,
enumerations (`dark`/`light`, `true`/`false`, units) by comparing just those tokens' logits;
numbers digit by digit within their range (volume and brightness 0–100, timers 1–999 and at
most 23 hours); app names and file queries as quote-free text up to 64 bytes. Literal JSON
between choices is never generated token by token: it is queued and fed in one batch the next
time a logit is needed, and the closing `}` is never fed at all. "Turn on dark mode" costs the
request's tokens plus two forward passes. A unit test drives the decoder with 3,000 random
logit streams and every result parses as a valid intent; invalid output is impossible.
llama.cpp's own GBNF sampler (`rmac-intelligence-bench --decoder gbnf`, grammar in
`llama.rs`) is kept for comparison.

### Spotlight rows

- Asked only when Lulo Intelligence is on, every search provider has answered, none matched
  confidently (no answer card; no result whose name starts with the query or one of its
  words), the query has two or more words with letters, and typing paused 250 ms (150 ms since
  phase 1.1). The model
  starts loading (`Prepare`) as soon as a query qualifies, so loading overlaps the rest of the
  typing. The request runs on the blocking pool; a newer keystroke drops it.
- The answer is one ordinary result row in its own "Lulo Intelligence" section above the
  others: "Turn On Dark Mode — Lulo can do this". Nothing changes until it is picked.
  Opening an app (resolved against the installed apps; no row for an app that is not
  installed), searching files and starting a timer run when picked. Settings changes
  (appearance, volume, brightness, Wi-Fi, Bluetooth, Do Not Disturb) ask on the row itself:
  the first Return or click changes the subtitle to "Press Return again to confirm".
- Each intent uses the service Lulo already has for the job: the rmac theme store and the
  toolkit sync (appearance, as Settings ▸ Appearance), Control Centre's typed operations
  (volume, Wi-Fi, Bluetooth, Focus), the OSD's logind backlight call (brightness), Clock's
  locked store and systemd ring timer (timers), and the launcher's own app launch and
  "Search in Files" actions.

### Evaluation sets

`tests/intelligence/intents-dev.jsonl` (66 cases: phase 0's 20, the brief's examples, typos,
Indian-English phrasing, refusals, one injection attempt) is the set the prompt was tuned on.
`tests/intelligence/intents-heldout.jsonl` (38 cases) was written before any measurement and is
frozen by SHA-256 in a unit test; it was never used for tuning. No request appears in both sets
or in the prompt's examples.

### Phase 1 results

Measured on the reference laptop (i5-5300U, 2 cores / 4 threads, 6.7 GiB) on 2026-10-07, with
the `iterate` build of `op/ai-phase1`, 2 threads (physical cores), the List prompt (version 4)
and the schema-guided decoder unless stated. Every run held the shared build lock, so no build
competed for the CPU. Tools: `rmac-intelligence-bench eval|latency|service`; the D-Bus runs
used a private session bus that activated the real service with the real model.

**Latency, Tiny (Qwen3.5-0.8B)**

| | First token | Request → parsed intent |
|---|---|---|
| Warm, "turn on dark mode" (first request after load) | 243 ms | 354 ms |
| Warm, the four brief requests × 10 (dark mode, 10-minute timer, open Notes, volume 30 %) | p50 259 ms, p90 316 ms | p50 487 ms, p90 658 ms |
| Warm, through D-Bus as Spotlight calls it (same 40 requests, client clock) | — | p50 486 ms, p90 659 ms |
| Warm, the 104 evaluation requests | p50 239–259 ms | p50 407–419 ms, p90 ≈ 600 ms |
| Cold: service activated, prefix state read from disk | — | 1.56 s (model load 0.85 s, prefix restore 27 ms) |
| Cold, first ever (693-token prefix evaluated, then saved) | — | 16.5 s through the service; 12.6 s in-process |

The ≤ 400 ms target holds for the **first token** on every request (p90 316 ms) and for the
whole answer to short settings requests ("turn on dark mode": 354 ms). It does **not** hold
end to end across all requests: the p50 is 0.41–0.49 s and the p90 0.6–0.66 s, because a timer,
an app name or a number needs two to four more forward passes at 15 tok/s. Phase 0's uncached
first token was 2.44 s; caching the prefix made it 9.4× faster. Four threads instead of two
changed nothing measurable (warm p50 474 against 487 ms). The first-ever cold start is slow
because the prefix is evaluated at 45 tok/s; it happens once per model and prompt version,
and Settings' speed check after the download pays it, not the first Spotlight request.

**Latency, Standard (Qwen3.5-2B)**: warm first token p50 553 ms, end to end p50 1,026 ms (brief
requests) and 813–879 ms (evaluation sets); cold with the saved prefix 1.13 s to load; the
first-ever prefix evaluation 26.9 s. Decode 7.66 tok/s, below Standard's 8 tok/s floor, as the
gate predicted from the Tiny rate (15.1 × 0.51 = 7.7): the reference laptop is offered Tiny
only.

**Accuracy with constrained decoding** (no fine-tuning; 0 invalid outputs in every run)

| Model | Decoder | Dev set (66, tuned on) | Held-out (38, frozen) | Held-out wrong actions | End to end p50 |
|---|---|---|---|---|---|
| 0.8B | schema-guided | 61/66 (92.4 %) | 33/38 (86.8 %) | 2 | 407–419 ms |
| 0.8B | llama.cpp GBNF | 61/66 (92.4 %) | 33/38 (86.8 %) | 2 | 964–1,020 ms |
| 2B | schema-guided | 61/66 (92.4 %) | 34/38 (89.5 %) | 1 | 813–879 ms |
| 2B | llama.cpp GBNF | 61/66 (92.4 %) | 34/38 (89.5 %) | 1 | 1,608–1,708 ms |

The two decoders choose the same answers; the schema-guided one is 2.3× faster because it
never decodes forced JSON one token at a time. The prompt was tuned on the dev set only
(the Chat style scored 52/66 against the List style's 56/66 at version 3; version 4 added five
examples and two rules). Held-out misses for 0.8B: "make everything light again" →
brightness 100 and "turn of the wifi" → Wi-Fi on (wrong actions, both Settings changes that
still ask before running), and three harmless "none" answers ("can you launch preview",
"drak mod", "do the needful and off the bluetooth"). The ADR §6 gates (≥ 95 % seen, ≤ 0.5 %
wrong actions) are not met before fine-tuning, which is phase 2's job.

**Memory**: service RSS 0.87 GiB with Tiny loaded (bench peak 0.85–0.89 GiB); 2B peak
1.89–1.94 GiB. Both inside the unit's caps.

**Idle cost**: through the real service on a private bus, from 5 s to 50 s after the last
request the service's threads made **0** context switches (no timer, no polling), and it
exited **60 s** after its last request. A request while turned off is answered "off" and the
process exits within a second without loading anything; with the feature off Spotlight never
asks, so nothing starts at all.

**Nested behaviour scenario** (`scripts/behavior/run_spotlight_intents.py`, private headless
Sway + nested niri, fixture model, packaged shell with this branch's Spotlight and service):
off — no row, service never started; on — "Turn On Dark Mode, Lulo can do this" shown, first
Return armed it ("Press Return again to confirm") with the appearance still Light, second
Return switched it to Dark and closed Spotlight, and the service exited once idle. It passed
3 of 3 runs after one fix: the first two runs linked Spotlight from the build directory
instead of placing it beside the service, as installs do, and no row appeared then; the cause
was not fully pinned down (the service and Spotlight now log why an answer is missing). It
also runs in Lulo runtime CI as the `spotlight-intents` check.

**The caller-check fix (2026-10-08).** The installed build (dev 952bb9ab) never answered: every
request from Spotlight and System Settings was refused with "the caller cannot be checked".
Cause: the unit's `PrivateTmp=yes`. In a user unit any mount-namespace option makes systemd
run the service in its own user namespace (`PrivateUsers=self`; the kernel log shows
`userns_create … comm="(rmac-intellig)"`, and on Ubuntu the process then runs under the
`unprivileged_userns` AppArmor profile). Reading another process's `/proc/<pid>/exe` is a
ptrace read, and the kernel grants it to a same-user reader only from the target's own user
namespace or from an ancestor that owns it, so from the service's namespace every caller gave
EACCES. AppArmor was not the cause: `unprivileged_userns` allows ptrace and logged no denial.
Proven on the laptop with the installed binary on a private bus: started plainly it refuses
`gdbus` as "not a Lulo program"; started in a `PrivateUsers=self`-style namespace (made the way
systemd-executor makes it) it refuses everything as "cannot be checked", and a test reader in
such a namespace gets `Permission denied` on an init-namespace process's exe. The phase 1 tests
started the service directly, without the unit, so they never saw it. Fix: `PrivateTmp=` is
gone, a unit test forbids every namespace-creating option, and the check itself was tightened
as described above (pidfd re-checked after reading the exe, explicit program list, logged
reasons). Spotlight may stay in its own namespace: a reader in the session's namespace owns it.
Regression check: `scripts/behavior/run_intelligence_unit.py`, the `intelligence-unit` Lulo
runtime check on GitHub's runners. It installs the real unit and activation file under a real
`systemd --user` manager with Ubuntu's user-namespace AppArmor restriction on, and checks that
`rmac-intelligence-bench` (a listed program) is answered both unconfined and in a transient
unit with Spotlight's `PrivateTmp=yes`; that `gdbus` and a copy of the bench named
`rmac-launcher` outside the list are refused; that the service shares the manager's user
namespace; and that a `PrivateTmp=yes` drop-in brings back "cannot be checked". Its first
run (runtime run 37739008334, ubuntu-26.04, dbus 1.16.2, restriction on) passed: the shipped unit
ran in the manager's namespace, unconfined, and answered both callers (the confined one in
its own namespace under `unprivileged_userns`); gdbus was refused as "not a Lulo program
(pid …, /usr/bin/gdbus)"; with the drop-in, the service ran under `unprivileged_userns` in its
own namespace and refused with "/proc/<pid>/exe: Permission denied", as in production.
End to end on the laptop (private nested session, the installed Lulo programs with this
branch's service, Spotlight started in its own user namespace under `unprivileged_userns` as
its unit puts it, the owner's verified Tiny model linked read-only): "turn on dark mode"
showed "Turn On Dark Mode, Lulo can do this", two Returns switched the session to Dark, and
the service exited once idle (cold first request 14.9 s, with the prefix evaluated for the first time). The laptop
cannot run the unit check itself: a nested `systemd --user` there has no delegated cgroup, and its own manager is
the owner's live session.

**Not done in phase 1**: a per-tier memory-cap drop-in; unloading on memory pressure (PSI);
a metered-connection warning before the download; the `systemd-analyze security` review of
the unit in a real user manager; streaming replies and the other tasks (Writing Tools, Help
answers, Terminal explanations); fine-tuning (phase 2, needed for the §6 accuracy gates).

## Phase 1.1 results (2026-10-08): the hitch, the cold start, latency and quality

Built on `op/ai-phase1-1`. Measured on the reference laptop with the `iterate` build, the
owner's verified Tiny model linked read-only into private nested sessions
(`scripts/behavior/run_spotlight_intents.py --real-model`, 22 typed requests), and
`rmac-intelligence-bench` under the shared build lock with a private state cache.

**1. The "hitch" was the harness, not Spotlight.** Spotlight's frame trace now marks
`assist_reply`, `assist_row_applied` and `launcher_render`, and every `draw_start` names its
surface (`draw_window:<app id>`). The 400–620 ms "slow frames", one per request, were each
request's closing Escape: that key closes the window, so it draws no frame, and the harness
paired it with the first frame of the *next* request's new window, opened about 0.55 s later
by the harness itself. All 22 such pairs crossed an `open_window`; the 690 same-window
keystrokes had p50 20 ms, p95 32 ms. The frame that inserts the "Lulo Intelligence" section is
ordinary: from the row being applied, p50 2.5–3.0 ms (max 9 ms) of UI-thread layout, shaping
and paint, then 10–35 ms to present (the swapchain wait every frame has). No D-Bus reply, icon
load or accessibility rebuild runs on the UI thread there. The harness now drops inputs
followed by `open_window`, reports Return keys that carry out a row apart from typing
(11–34 ms), adds `assist_frame_ms` and fails if a row insertion takes over 16 ms of UI-thread
work (it never did). Two more harness faults were fixed: it typed before Spotlight had
keyboard focus ("open notes" arrived as "en notes"; it now waits for the launcher's
`focus_in`), and it walked the whole accessibility tree five times a second while the model
ran (it now waits on the trace's `assist_reply`/`assist_row_applied` marks and reads the row
once). The nested session itself is still pessimistic: its niri composites in software and
used about two cores whenever Spotlight drew, so service times there run 1.2–2× the bench's.
Found while tracing, not fixed: Spotlight redraws its window about 10 times a second while a
request is in flight, with no view re-render; the source is not pinned down yet.

**2. The cold start.** Settings ▸ Lulo Intelligence now warms the model up whenever the
feature is on, the model is on disk and its saved prefix state is missing: after the
download, after turning the feature on, after choosing another model, and on opening the pane
after an update changed the prompt or the model. It calls `Calibrate` when this PC has no
speed check yet (that also loads the model and saves the state) and `Prepare` otherwise, and
the download row reads "Downloaded · 533 MB · Getting ready…" until the state is on disk.
Spotlight also asks for `Prepare` as soon as it opens when the model is present but its state
is not, so even an unwarmed first request overlaps the evaluation with typing. The state file
name now lives in `rmac_intelligence::prefix_state`: a digest of the model's SHA-256 (not its
path), the prompt version, style and text, the context size and the llama.cpp binding's
version, which a unit test holds equal to `Cargo.lock`. Settings and Spotlight check it with one
`stat`. The service creates the file 0600 before llama.cpp writes it (it was 0664 outside the
unit's `UMask=0077`), in a 0700 `$XDG_CACHE_HOME/lulo/intelligence/`, and deletes older
`prefix-*.state` files once a new one is in place, so an update leaves no stale 24–27 MB files.

| | Before | After |
|---|---|---|
| First-ever request, no saved state | 15.0 s | 7.9 s (prefix 693 → 392 tokens) |
| Settings warm-up after the download | — | 13.8–15.1 s in the background, "Getting ready…" shown and cleared; state 23.9 MB, 0600 |
| First Spotlight request after warm-up (service started fresh, state read from disk) | — | 1.08–1.22 s keystroke to row |

**3. Warm latency.** Profiling (bench `latency`, 20 runs) showed every token costs: the
request's own tokens at about 21 ms each (43 tok/s), each decoding pass about 45 ms plus
9 ms per token it feeds, and the state restore 9 ms. Thread counts 2, 3 and 4 measured the same.
The JSON prompt spent six template tokens per request (`\nJSON: {"intent":"`) and fed a
JSON fragment (`","mode":"`) on every pass. The new default prompt style, `compact`
(`PROMPT_VERSION` 5), keeps the same instructions and 19 examples but writes each answer as a
short action line — `switch to dark mode => appearance dark`, `timer for 5 mins => timer 5
minutes` — and `rmac_intelligence::decode` (`Syntax::Compact`) reads that line under the same
schema and still builds the strict wire JSON, so invalid output stays impossible (3,000-stream
random-logit test, now in both syntaxes). A request costs its own words plus one template
token (` =>`); a pass feeds one or two tokens. Decoding already stopped as soon as the answer
was complete (the closing token is never fed). The service now also drops a stale request:
each `Run` from a caller gets a ticket, and a newer one from the same caller makes the older
answer `Cancelled` before it starts or at its next forward pass, so a pause mid-typing never
makes the final request queue behind a dead one (`queued_ms` p50 1 ms). That bounds what a
shorter pause costs, so Spotlight's debounce is now 150 ms instead of 250 ms. The model stays
loaded across keystrokes as before (60 s idle exit).

| | Before (list, JSON) | After (compact) |
|---|---|---|
| Bench, four brief requests × 5, warm | p50 490 ms, p90 663 ms | p50 348 ms, p90 422 ms |
| — prefill / decode p50 | 251 ms (10 tokens) / 230 ms | 149 ms (5 tokens) / 189 ms |
| Bench, dev set (70) end to end | p50 421 ms, p90 618 ms | p50 240 ms, p90 366 ms |
| Bench, held-out set (38) end to end | p50 406 ms, p90 585 ms | p50 220 ms, p90 377 ms |
| Real Spotlight, nested session, service time p50 | 737–815 ms (JSON prompt, this branch's marks, before the prompt change) | 403–425 ms |
| Real Spotlight, nested session, keystroke to row p50 (trace clock) | 1,108 ms (1,344 ms by AT-SPI polling) | 617–662 ms (3 runs; min 144, max 1,073) |

The 0.8 s keystroke-to-row target holds through the real Spotlight even in the nested
session; the 0.4 s service target holds on the bench and is at 0.40–0.43 s in the nested
session, whose software compositor competes for the same two cores.

**4. Quality.** `rmac_intelligence::guard` applies deterministic checks to every answer (bench
`--no-guard` turns them off to measure the model alone): a timer read out of a clock time
("at 5", "5pm", "6:30", "tomorrow", "tonight", weekdays) or from a request with no number at
all is dropped, since Lulo has no reminders or alarms, while a length ("remind me in 5
minutes") stays a timer; a Settings change with a strict clock time ("at 7 pm") is dropped;
an explicit "off"/"disable"/"disconnect" or "on"/"enable"/"connect" overrides a contradicting
switch; a file search for "delete …"/"wipe …" is dropped. `rmac_intelligence::fuzzy` corrects
light typos in app names (optimal string alignment distance: none up to 3 letters, one up to
7, two beyond; ties are no answer): Spotlight answers "open verb + installed app name"
requests itself, without the model ("opn notse" → Notes in 0.2 s), and corrects an app name
the model got slightly wrong. The dev set gained "remind me to call mum at 5", "wake me up at
6:30", "remind me tomorrow to pay the rent" (none) and "opn notse" (Notes; the eval scores the
model alone, which still misses it — Spotlight's own pass does not). The held-out set is
unchanged and was run only for these two final measurements.

| | Dev (70, tuned on) | Wrong actions | Held-out (38, frozen) | Wrong actions |
|---|---|---|---|---|
| Before: list prompt, no guard | 62 (88.6 %) | 6 | 33 (86.8 %) | 2 |
| List prompt + guard | 65 (92.9 %) | 3 | — | — |
| Compact prompt, no guard | 64 (91.4 %) | 4 | — | — |
| **After: compact prompt + guard** | **66 (94.3 %)** | **2** | **34 (89.5 %)** | **2** |

Remaining misses after: dev "open bluetooth settings" → open "Bluetooth" (no such app, so
Spotlight shows no row), "do one thing, open the files" → file search, "where is my passport
scan" → none, "opn notse" (model only); held-out "can you launch preview", "start mail" (both
none; Spotlight's own pass opens them when installed), "darken the display" → Dark Mode and
"turn of the wifi" → Wi-Fi On (both Settings changes that still ask before running).

Through the real Spotlight (22 requests): before 20/22 correct ("opn notse" no row, "remind me
to call mum at 5" → a 5-minute timer); after 21/22 in every run, the one miss being "turn of
the wifi" (the harness marks it lenient, a known base-model gap). A one-off "Getting ready…"
check, the 0600 state and the idle exit (service gone within its timeout, the session at
5–10 clock ticks over 2 s afterwards) passed in every run.

## 11. Open risks

1. **Speed on the oldest PCs.** Prefill on a 2-core Broadwell may make summaries feel slow.
   Mitigations: cached prompt prefixes, input caps, progress, and Tiny-tier fallbacks. A PC
   that cannot meet the budgets is not offered the feature.
2. **Memory pressure on 4–8 GB PCs.** A 1.6 GiB model plus apps can push the desktop into
   swap: the exact slowness Lulo exists to avoid. Mitigations: the `MemAvailable` gate,
   `MemorySwapMax=0`, PSI-triggered unloading, and fast exit.
3. **Small-model quality.** Hallucinated flags, a meaning changed in a rewrite, a wrong
   intent. Mitigations: grounding, constrained output, previews and confirmation, and
   evaluation gates.
4. **Native code security.** The llama.cpp GGUF parser and ONNX Runtime are C++. Mitigations:
   checksum-pinned models, sandboxed processes, and advisory tracking.
5. **Licence drift.** Future Qwen or Gemma releases may change terms, and voice data
   licences vary. Each manifest entry records its licence, and CI rejects entries that are
   not on an allowlist (Apache-2.0, MIT, CC0, CC BY).
6. **Model origin.** Some users or organisations avoid Qwen-family models. Granite 4.0 is a
   ready alternative under the same licence; the manifest can offer it.
7. **Sandboxing on Ubuntu.** User-namespace restrictions stop `PrivateNetwork=` in user units;
   we rely on seccomp address-family filtering instead (§5). Verify on 26.04 in phase 0.
8. **Build cost.** CMake and C++ add CI time and disk (see `AGENTS.md`). The service is a
   separate crate, so app builds never compile llama.cpp.
9. **Prompt injection** cannot be fully solved by prompting. The design does not depend on
   the model resisting it (§9).
10. **Wake-word false accepts and battery.** Always-on audio is costly on laptops and
    unsettling if it triggers by mistake. It is opt-in, gated by tier and indicator, paused
    on battery saver, and measured.
11. **Hosting.** Hugging Face may change URLs; pinned revisions plus our own mirror cover
    this.

## What the owner must provide

- Approval for **rented-GPU spend** (proposed cap $60) and an account with payment on one
  provider (RunPod, Lambda or Vast).
- Agreement on **when** the M2 Mac may train overnight (it uses the whole machine for hours).
- For phase 4's optional cloud planner, a **decision** on which providers to support.

## Consequences

- With the feature off, Lulo's idle cost does not change: no process, no unit started at
  login, no model on disk.
- A new class of session service exists: started on demand and exiting when idle. Voice and
  agent processes follow the same pattern.
- The action registry becomes the one way Spotlight, voice and the agent change the system,
  so every new skill gets a type, a tier and a test.
- `docs/parity.md`'s coverage row "Apple Intelligence & Siri … n/a (no substitute)" is
  superseded by SET-115.
