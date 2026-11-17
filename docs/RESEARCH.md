# MD++ — Research summary

*2026-11-17 · inputs for [PLAN.md](PLAN.md)*

MD++ (repo/crate/binary name `mdpp`) is a free, open-source, lightweight Markdown editor for Linux and
Windows written in Rust — "Notepad++ for Markdown" with built-in Markdown intellisense.

This file is the **verified** digest. The five raw research reports it draws on are in
[`research/`](research/); they were produced by research agents and contain errors, so anything that
matters was re-checked here.

**Evidence legend:** ✅ verified by us (measured on the dev machine, crates.io / GitHub API, or read in
the source) · ⚠️ from the raw reports, plausible, not independently checked · ❌ refuted (see §11).

Dev machine for all measurements: AMD Ryzen 5 5500U (12 threads), 22 GB RAM, AMD Lucienne iGPU,
CachyOS (Linux 7.3), KDE Plasma 6 on Wayland, rustc 1.97.1.

---

## 1. Method

| Track | Question | Raw report |
|---|---|---|
| A | Which Rust GUI framework can carry a real text editor? | [01-gui-frameworks.md](research/01-gui-frameworks.md) |
| B | What do Zed, Helix, Lapce, xi, Notepad++, Typora, Obsidian… teach us? | [02-existing-editors.md](research/02-existing-editors.md) |
| C | Rust performance: buffers, parsing, rendering, build profiles, profiling | [03-performance.md](research/03-performance.md) |
| D | Markdown intellisense, linting, spelling, preview, export | [04-intellisense-preview-export.md](research/04-intellisense-preview-export.md) |
| E | UX patterns, platform integration, packaging, CI, licensing | [05-ux-platform-delivery.md](research/05-ux-platform-delivery.md) |

On top of the reports we ran two experiments on the dev machine (code in [`../spikes/`](../spikes/)):
a **GUI bake-off** (§2) and a **Markdown parser benchmark** (§3), and verified versions, licences and
APIs against crates.io, GitHub and the crates' source.

---

## 2. GUI bake-off — measured ✅

Three equivalent minimal apps, each opening a file into the framework's own multi-line editor widget
(no highlighting, default features). Test file: 1 MB of realistic Markdown (42,372 lines: headings,
lists, tasks, fenced code, tables, links, Turkish characters). Build: `release`, thin LTO, stripped.
RSS read from `/proc` after 6 s and 16 s; idle CPU over 10 s; "first frame" = time from `main()` to the
first render/view call.

| | egui/eframe 0.37 | iced 0.14 | gpui-kit 0.10 (GPUI) |
|---|---|---|---|
| Binary (release, thin LTO, stripped) | 20.4 MB | 15.7 MB | 40.1 MB |
| Binary (ship profile: fat LTO, cgu=1, panic=abort) | 14.3 MB | 10.7 MB | 28.0 MB |
| Clean release build time | ~140 s | 155 s | 242 s |
| RSS, empty document | 86 MB | 76 MB | 95 MB |
| **RSS, 1 MB document** | **188 MB** | **406 MB** | **104 MB** |
| **Growth for 1 MB of text** | **+102 MB** | **+330 MB** | **+9 MB** |
| First frame, empty doc | 79 ms | 86 ms | 97 ms |
| First frame, 1 MB doc (warm) | 197 ms | 529 ms | 101 ms |
| First frame, 1 MB doc (first run) | 554 ms | 1,334 ms | 104 ms |
| Idle CPU (cursor blinking) | 0.0 % | 1.6 % | 1.8 % → 0 % with blink idle-timeout¹ |

¹ gpui-kit's `CursorBlink` defaults to "blinks every 500 ms without an idle timeout" but exposes
`with_idle_timeout(..)` (read in `gpui-base` source) — MD++ sets one, so a quiet window costs 0 % CPU.

**Reading:** every GPU toolkit idles at ~75–95 MB, so "Notepad++-level RAM" (≈20 MB) is not on the
table for any of them. What separates them is **how cost grows with the document**: gpui-kit's editor
is a virtualized rope (only visible lines are laid out) — 1 MB of text costs 9 MB and does not delay the
first frame. egui's `TextEdit` lays out the whole string; iced's `text_editor` keeps a shaped
`cosmic-text` buffer for every line. Both would need a custom editor widget to compete. gpui-kit's
weakness is binary size: 28 MB with the ship profile (default features, nothing trimmed yet) versus
11–14 MB — still a fraction of Electron editors, and the budget in PLAN.md §5 is set from this figure.
(Sizes are MiB, `stat` bytes / 1,048,576.)

---

## 3. Markdown parser benchmark — measured ✅

Same 1 MB file; single-character inserts at 20 positions with correct `InputEdit` byte **and** row/col
points; medians. Idle machine (a first run during background compiles was ~2× slower across the board,
same ranking).

| Parser | 1 MB full parse | 50 KB full parse | Re-parse after 1-char edit (1 MB) |
|---|---|---|---|
| `pulldown-cmark` 0.13.6 (`into_offset_iter`, all extensions) | **8.3 ms** | **0.5 ms** | full re-parse: 8.3 ms |
| `tree-sitter-md` 0.5.4 — block grammar | 258 ms | — | **34 ms median, 124 ms max** (incremental) |
| `tree-sitter-md` 0.5.4 — block + inline | 739 ms | — | — |
| `markdown` (markdown-rs) 1.0 — `to_mdast`, GFM | **2,103 ms** | **21.2 ms** | full re-parse |

**Consequences:**
- A full `pulldown-cmark` re-parse on a background thread is **4× cheaper than tree-sitter's
  *incremental* re-parse** and gives byte offsets for every event. Tree-sitter-md is the wrong tool for
  MD++'s editor loop (its external scanners force wide re-parses). This contradicts two of the raw
  reports (D: "< 0.5 ms", B: "< 100 µs" — ❌) and confirms the direction of C (whose own benchmark
  passed wrong row/col points).
- gpui-kit's built-in Markdown view parses with markdown-rs (✅ `gpui-base/src/text/format/markdown.rs`),
  **250× slower than pulldown-cmark** and a second parser whose offsets differ — MD++ renders its
  preview from pulldown-cmark events instead (PLAN D6).

`pulldown-cmark` 0.13 supports every extension MD++ needs ✅ (read in source): tables, footnotes,
strikethrough, task lists, heading attributes, YAML/`+++` front matter, **math**, **GFM alerts**,
definition lists, super/subscript and **wikilinks**.

---

## 4. GUI framework landscape

Versions from crates.io on 2026-11-17 ✅.

| Candidate | Version | Editor widget for 10k–200k lines | Verdict |
|---|---|---|---|
| **gpui-kit** (Longbridge, on Zed's GPUI) | 0.10.0 (2026-11-16), Apache-2.0 | ✅ rope (`ropey`) + display map + virtual rendering; LSP-shaped provider traits; pluggable highlighter | **Chosen** — see §2 |
| iced (+ libcosmic) | 0.14.0 (2025-12-07), MIT | `text_editor` works, but stores a shaped buffer per line (+330 MB per MB of text, §2); single cursor | Runner-up; would need a custom editor widget |
| egui / eframe | 0.37.0, MIT/Apache | `TextEdit` lays out the whole string every change | Out for an editor of this kind |
| Slint | 1.19.0 | Plain `TextEdit` only; rich-text editor out of scope ([#2723](https://github.com/slint-ui/slint/issues/2723)) ⚠️ | Out |
| Floem (Lapce) | 0.2.0 (2024-11-14) | Good editor core, but stale release; IME issue [lapce#319](https://github.com/lapce/lapce/issues/319) open since 2022 ✅ | Out |
| Xilem / Masonry | 0.4.0 (2025-10) | Plain-text `TextArea` | Too early |
| Freya / Makepad / Blitz | 0.5.2 / 1.0.0 / beta | Early or non-standard editors | Too early |
| Tauri 2 + CodeMirror 6 | 2.12.6 | Best-in-class editor (CM6) | Not a Rust UI; WebKitGTK on Linux; kept only as the fallback of last resort |

What gpui-kit gives MD++ out of the box (✅ read in source / README): editor with `ropey` storage,
soft-wrap and folding maps, multi-cursor, `InputHighlighter` (parser-independent highlighting seam),
`CompletionProvider`, `HoverProvider`, `DefinitionProvider`, `CodeActionProvider`,
`DocumentColorProvider`, semantic tokens — all using `lsp-types`; plus dock, tabs, sidebar, tree,
command list, title bar, status bar, settings UI, dialogs, notifications, native menus, theme system
(JSON themes), Markdown rendering, AccessKit accessibility and a UI test harness.

Gaps found in gpui-kit's source (✅ `gpui-base` 0.10.0) that MD++ must work around or fix upstream:
- Enter inserts a hardcoded `"\n"` (`input/base/state.rs:2452`) — CRLF documents would get mixed line
  endings unless the buffer is normalized to LF (PLAN §3.3).
- Atomic multi-range editing (`replace_text_in_ranges`) is `pub(crate)`; the public `insert`/`replace`
  act on the primary cursor only → formatting commands over several cursors need an upstream API.
- Built-in search is literal only (`AhoCorasick`) → regex / whole-word / in-selection search is MD++'s
  own (`regex-cursor` 0.1.5 searches ropes; it is what Helix uses ⚠️).
- The Markdown view parses with markdown-rs (§3).
- Clipboard images arrive as **encoded** bytes plus a format (`gpui::Image { format, bytes }`) → pasting
  an image needs no image encoder.

GPUI itself provides ✅ native file dialogs (`prompt_for_paths`, `prompt_for_new_path`), a clipboard
with **image** and **file-path** entries, and window-appearance observation (OS light/dark changes) —
so `rfd`, `arboard` and `dark-light` are **not** needed.

**Risks of the choice** (all ✅ in the manifests): gpui-kit pins pre-release GPUI snapshots
(`gpui-pre =0.3.12`; 0.3.13 shipped the same day) and releases weekly → API churn. GPUI has no CPU
renderer: Windows uses D3D11 (WARP software fallback exists), Linux needs Vulkan (lavapipe as software
fallback). Binary is 2–3× the size of iced/egui. Mitigations are in PLAN.md §9.

---

## 5. Lessons from existing editors

Adopt:
- **Single process; buffer, layout and render on the UI thread; heavy work on snapshots in
  background threads.** xi-editor's multi-process async core is the canonical failure — see Raph
  Levien's [xi retrospective](https://raphlinus.github.io/xi/2020/06/27/xi-retrospective.html).
- **Rope + O(1) snapshot clones** (`ropey`, as in Helix and gpui-kit) for handing text to workers.
- **Display-map layering** (Zed: inlay → fold → tab → wrap → block) — the route to a future
  Typora-style "conceal" mode without touching the buffer. gpui-kit already has wrap and fold maps.
- **The file is the truth.** Never round-trip Markdown through an AST on save — MarkText's rewriting of
  users' files ([marktext#2189](https://github.com/marktext/marktext/issues/2189)) is the cautionary tale.
- **Hot exit** (Notepad++/Sublime): dirty buffers flushed to a backup dir every few seconds; closing
  never nags; everything — including untitled tabs — comes back after a crash or reboot.
- **Notepad++'s utility core**: instant start, tabs, find/replace with regex, find in files,
  encoding/EOL visibility and conversion, large-file tolerance, portable mode.

Avoid:
- Rewriting the GUI toolkit mid-project (Lapce: Druid → Floem).
- Heavy sandboxed plugin systems before there's a community (Lapce WASI). MD++ v1 has **no** plugin API.
- Electron-class footprints (Typora/Obsidian/MarkText/Zettlr idle at hundreds of MB ⚠️).
- WYSIWYG-first editing in v1: inline rendering causes "jumping cursor" layout shifts and is far more
  expensive; source + synced preview first, conceal mode later.

**Direct open-source competitors** ✅:
[Ferrite](https://github.com/OlaProeis/Ferrite) (egui, MIT, ★1.9k, Markdown/JSON/YAML/TOML editor) and
[Velotype](https://github.com/manyougz/velotype) (GPUI, Apache-2.0, ★0.6k, block-based WYSIWYG; also pins
`gpui-pre`). MD++'s differentiation: Notepad++-style workflow (tabs, sessions, hot exit, find in files,
encodings) **plus** language-server-grade Markdown intellisense, source-faithful editing.
Ferrite's README documents Windows Defender false positives on unsigned Rust builds, fixed by
**SignPath Foundation** signing — a lesson MD++ applies from release one.

---

## 6. Markdown intellisense landscape

What the best tools do (VS Code's Markdown language server, Marksman, markdown-oxide, Obsidian):
path completion in links/images, heading-anchor completion (`#`), reference-link and footnote
completion, broken-link/anchor diagnostics, go-to-definition and find-references for links and headings,
rename heading → update links, document symbols/outline, folding, smart selection, paste/drop image →
file + link. Editing assists people expect from Typora/Obsidian: list continuation and renumbering,
smart pairs and wrap-selection, table formatting with Tab navigation, task toggles, emoji
`:shortcode:` completion, paste URL over a selection → link.

Rust building blocks (✅ crates.io, licences checked):

| Need | Crate | Version | Licence | Note |
|---|---|---|---|---|
| Structure / offsets / preview events | `pulldown-cmark` | 0.13.6 | MIT | §3 |
| Markdown lint rules (markdownlint) | `rumdl` (lib target `rumdl_lib`) | 0.3.4 | MIT | API stability to check before use |
| English grammar | `harper-core` | 3.2.0 | Apache-2.0 | English only ⚠️ |
| Spelling (Hunspell-compatible) | `spellbook` | 0.4.2 | MPL-2.0 | From Helix; dictionaries loaded at runtime |
| Emoji data | `emojis` | 0.9.1 | MIT/Apache + Unicode-3.0 | |
| Fuzzy matching | `nucleo-matcher` | 0.3.1 | MPL-2.0 | |
| Math → SVG without JS | `ratex-svg` | 0.1.16 | MIT | + `resvg` 0.50 (Apache-2.0) |
| DOCX export | `docx-rs` | 0.4.22 | MIT | later |
| Mermaid in pure Rust | `flowmaid` | 0.40.0 | **GPL-3.0** | **Not usable** in an MIT/Apache app |

Facts that shape the design:
- **Turkish** needs Hunspell affix morphology (agglutination) and locale-aware casing (`I→ı`, `İ→i`);
  plain `to_lowercase()` is wrong for Turkish ⚠️ (well-known Unicode behaviour).
- Many Hunspell dictionaries are (L)GPL/MPL → **never bundled into the binary**; downloaded/loaded at
  runtime from the data dir.
- In-process intellisense meets a < 50 ms budget easily; external LSP servers (Marksman is a .NET
  binary, ~35 MB ⚠️) are an optional later add-on, not the core.

---

## 7. Performance & build practice (takeaways)

- Event-driven redraw only; stop the cursor blink after idle → 0 % CPU (gpui-kit supports it ✅).
- Generation counter per document: every background result carries the revision it was computed from;
  stale results are dropped. Debounce structural work (~100–150 ms).
- No Tokio in the app core; GPUI has its own foreground/background executors.
- File I/O: detect BOM/encoding (`chardetng` 1.0 + `encoding_rs` 0.8.44 ✅), preserve BOM and EOL on
  save, atomic save (temp file in the same dir + rename; on Windows `ReplaceFileW` to survive AV /
  indexer handles ⚠️), watch files with `notify` 8.2 ✅ and ignore self-triggered events.
- Release profile: `lto="fat"`, `codegen-units=1`, `panic="abort"`, `strip=true`; measure
  `opt-level=3` vs `"s"`; PGO via `cargo-pgo` later ⚠️ (10–25 % in hot loops).
- Dev loop: workspace split so UI edits don't rebuild parsers; `mold`/`lld` linker; `cargo-nextest`.
- Profiling: `samply` (CPU), `tracing` + Tracy (frames), `dhat`/heaptrack (memory), `hyperfine`
  (startup), `divan` (micro-benchmarks). CI regression gates on deterministic counts
  (`iai-callgrind`) rather than wall-clock ⚠️.

---

## 8. UX & platform (takeaways)

- Modern-minimal chrome with Notepad++ ergonomics: tabs with dirty dot, session restore, a rich status
  bar (Ln/Col, words, reading time, encoding, EOL), inline find/replace bar, command palette
  (`Ctrl+Shift+P`), quick open (`Ctrl+P`), outline sidebar, split preview with **source-mapped** scroll
  sync (percentage scrolling drifts), focus/zen and typewriter modes.
- Source-mode Markdown styling: dimmed markup characters, scaled headings, code pills; semantic colour
  tokens → light/dark themes meeting WCAG AA.
- Fonts: bundle one OFL monospace (e.g. JetBrains Mono, ~0.5 MB); use the OS UI font.
- Single instance + "open in running instance" via `interprocess` 2.4.8 ✅ local sockets
  (`single-instance` 0.3.3 is unmaintained since 2021 ✅).
- Settings as a commented TOML file with hot reload, plus a settings UI for the common options.
- Portable mode (marker file next to the exe), crash log via panic hook, **no telemetry**.

---

## 9. Distribution, licensing, quality

- Licence: **MIT OR Apache-2.0** (Rust-ecosystem norm; every chosen dependency is compatible ✅).
  Enforce with `cargo-deny` in CI. Fonts OFL-1.1.
- Windows: per-user installer (no UAC) + zip/portable; **SignPath Foundation** free code signing for
  OSS (needs OSI licence, public repo, CI-built artifacts) ✅ (used by Ferrite); winget, Scoop.
- Linux: AppImage + `.tar.gz` first, then Flathub (portals already used by GPUI dialogs), `.deb`, AUR.
- Updates: "check for updates" against GitHub Releases (opt-in, no background network); delta
  auto-update (Velopack 1.2.x ✅) only if users ask.
- CI: GitHub Actions on `ubuntu` + `windows` runners, `Swatinem/rust-cache`, fmt/clippy/test/deny;
  `release-plz` + `git-cliff` changelog; tests with `insta`, `proptest`, `cargo-fuzz`; gpui-kit's UI
  test harness for the app.

---

## 10. Name

- "Markdown++" collides with [Edditoria/markdown-plus-plus](https://github.com/Edditoria/markdown-plus-plus)
  (★1.6k, a Notepad++ syntax file) and "MarkdownViewer++" (a Notepad++ plugin) ✅ → users would assume a
  Notepad++ plugin.
- **"MD++"** has no notable collision; `mdpp` is free on crates.io ✅. Product name **MD++**, technical
  id **`mdpp`**. Never use Notepad++'s name, logo or colours in branding (its author defends the mark).

---

## 11. Corrections to the raw reports

| Claim (report) | Finding |
|---|---|
| tree-sitter-md incremental re-parse "< 0.5 ms" (D) / "< 100 µs" (B) | ❌ 34 ms median, 124 ms max on 1 MB (§3) |
| tree-sitter-md incremental "132 ms" (C) | Direction right, benchmark passed wrong row/col points; corrected figure 34 ms median (idle machine) |
| egui "~7–10 MB exe, 35–60 MB RSS" (A) | ❌ 20.4 MB; 86 MB empty, 188 MB with 1 MB doc |
| iced "~50–70 MB RSS, 10–15 MB exe" (A) | ❌ 76 MB empty, 406 MB with 1 MB doc; 15.7 MB |
| GPUI "~8–15 MB exe, 50–80 MB RSS" (A) | ❌ 40.1 MB (thin LTO); 95 / 104 MB |
| `syntect` "+15–25 MB RAM" (D) vs "1.1 MB binary, 3.7 ms load" (C) | Conflicting → measure in Phase 0 before committing |
| "syntect 2,500× slower than tree-sitter" (B) | ❌ unsupported; dropped |
| Velopack "1.3.x" (E) | ❌ crates.io latest is 1.2.161 |
| "`rumdl_lib` crate" (D) | It is the library target of the `rumdl` crate |
| "GPUI 0.2.2 (Oct 2025)" (A) | `gpui` 0.2.2 is from 2026-10-30, and gpui-kit actually uses the `gpui-pre` snapshot crates |
| Competitor RAM / start-time tables (B) | Unverified estimates — not used for decisions |
