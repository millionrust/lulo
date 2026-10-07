# ADR 0024 — Lulo Intelligence: a small local model, loaded on demand, behind typed actions

- **Status:** proposed 2026-10-07. Fine-tuning is approved by the owner (2026-10-07) and is a
  planned phase. Any rented-GPU spend still needs the owner's account and payment. Nothing is
  built yet; phase 0 is a measurement spike on the reference laptop.
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

**Decision:** the default is **Qwen3.5-2B** and the tiny fallback is **Qwen3.5-0.8B**.
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

| Tier | Model | Static requirements | Calibration (10 s, at enable time) |
|---|---|---|---|
| Not offered | — | x86-64 without AVX2+FMA, fewer than 2 physical cores, or MemTotal < 3.5 GiB | — |
| Tiny | Qwen3.5-0.8B | AVX2 (or NEON), MemTotal ≥ 3.5 GiB | decode ≥ 10 tok/s, otherwise "not available on this PC" |
| **Standard** (reference laptop) | Qwen3.5-2B | MemTotal ≥ 6 GiB, ≥ 2 physical cores | decode ≥ 6 tok/s, otherwise offer Tiny |
| Enhanced (opt-in) | Qwen3.5-4B | MemTotal ≥ 12 GiB and ≥ 4 cores, or a Vulkan GPU with ≥ 4 GiB VRAM | decode ≥ 6 tok/s |

Facts come from `rmac-system-info` (`facts.rs` already reads `MemTotal`), plus CPUID. The
calibration result is stored and re-run when the CPU model or RAM size changes. Settings
always explains why a tier was chosen ("This PC has 6.7 GB of memory; Lulo uses the
Standard model").

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
deterministic ones, after a 250 ms typing pause, and only when the query reads like a sentence
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
