# MD++ — End-to-end development plan

*2026-11-17 · evidence for every decision: [RESEARCH.md](RESEARCH.md)*

> **MD++** — a free, open-source Markdown editor for Linux and Windows.
> Notepad++'s speed and reliability, with Markdown intellisense built in.
> Product name **MD++**; repository, crate and binary **`mdpp`**. Licence **MIT OR Apache-2.0**.

---

## 1. Product

### 1.1 Who it is for
People who live in `.md` files and want them to open instantly: developers (READMEs, docs, ADRs),
technical writers, students and note-takers who prefer plain files over a proprietary vault.

### 1.2 Principles (tie-breakers for every decision)
1. **The file is the truth.** MD++ never rewrites what the user did not touch: no AST round-trip, BOM,
   encoding and line endings preserved. Formatting happens only on an explicit command.
2. **Instant.** Typing never waits for parsing, linting or the preview. Startup and file-open are budgeted
   (§5) and measured in CI.
3. **Never lose work.** Hot exit: closing never nags; a crash or reboot restores every tab, including
   untitled ones.
4. **Simple by default, powerful on demand.** A clean window with tabs, editor and optional preview;
   everything else lives in the command palette, menus and settings.
5. **Offline and private.** No telemetry, no network access unless the user asks for it.
6. **Data over code.** Settings, keymap, themes, snippets, emoji tables and lint rules are data files
   with documented defaults, not hardcoded branches (config- and metadata-driven).

### 1.3 Non-goals for 1.0
WYSIWYG/block editing, knowledge-graph or vault features, sync/cloud, collaboration, a plugin API or
marketplace, macOS, mobile. Each can be revisited after 1.0 with user demand as the trigger.

---

## 2. Decisions

| # | Decision | Why (evidence in RESEARCH.md) | Rejected |
|---|---|---|---|
| D1 | **UI: gpui-kit 0.10 on GPUI**, versions pinned exactly | Only toolkit with a production rope-based, virtualized editor + LSP-shaped provider traits. Measured: +9 MB RAM for a 1 MB document vs +102 MB (egui) and +330 MB (iced); first frame ~100 ms regardless of document size (§2) | iced (needs a custom editor widget), egui (whole-text layout), Slint (no rich editor), Floem (stale, IME bug), Tauri+CM6 (not a Rust UI) |
| D2 | **Text buffer: `ropey`** — the type gpui-kit's editor already uses | One buffer type end to end; O(1) snapshots for workers | A second rope (`crop`) would mean converting on every edit |
| D3 | **Highlighting: own incremental line-state lexer** behind gpui-kit's `InputHighlighter` | Markdown colouring is line-local plus a little state (fences, front matter, HTML/math blocks); re-lex from the edited line until the state converges → microseconds | `tree-sitter-md`: 258 ms full parse, **34 ms median / 124 ms max per keystroke incremental** on 1 MB (§3) |
| D4 | **Structure: `pulldown-cmark` 0.13**, full re-parse on a background thread, debounced, revision-checked | 8 ms per MB, 0.5 ms per 50 KB; byte offsets for every event; supports GFM alerts, math, wikilinks, front matter, footnotes, tables, tasks | comrak (AST we don't need), markdown-rs (250× slower, §3) |
| D5 | **Fenced-code highlighting: syntect via gpui-kit's `syntect` feature** (or `two-face`), lazily, visible fences only | ~200 languages for a few MB; no per-language C grammars | Bundled tree-sitter grammars (MBs each). Footprint to be measured in Phase 0 |
| D6 | **Preview: own renderer from the same `pulldown-cmark` events**, drawn with gpui-kit primitives in a virtual list (only visible blocks laid out); each block carries its source range → exact scroll sync | One parser for editor and preview (DRY); offsets are exact; scales to large documents | gpui-kit's built-in Markdown view (parses with markdown-rs: 2.1 s per MB, 21 ms per 50 KB, different offsets); webview preview |
| D7 | **Intellisense: in-process engine `mdpp-lang`**, UI-agnostic, pure functions over rope snapshots; results carry byte ranges, and thin adapters convert them to gpui-kit's `lsp-types` providers (one conversion, gpui-kit's own) | < 50 ms latency without IPC; testable without a GUI; same types as an LSP so an external server can be added later without new plumbing | External LSP servers as the core (latency, extra binaries) |
| D8 | **Platform: use what GPUI ships** — file dialogs, clipboard (text, image, paths), OS appearance; add `interprocess` (single instance), `notify` (file watching), `chardetng` + `encoding_rs` (encodings) | Fewer dependencies (KISS/DRY) | `rfd`, `arboard`, `dark-light` (duplicates of GPUI), `single-instance` (unmaintained) |
| D9 | **Config as data**: `settings.toml` (commented, hot-reloaded), `keymap.toml`, themes in gpui-kit's JSON theme schema, snippets/alert types/emoji as data | Operators change behaviour without a rebuild; new themes/snippets need no code | Hardcoded menus/keys/colours |
| D10 | **No async runtime in the app core**; GPUI's background executor + revision counters | Smaller binary, no cancel-safety hazards | Tokio |
| D11 | **Licence MIT OR Apache-2.0**, `cargo-deny` allow-list in CI; Hunspell dictionaries never bundled | Every chosen dependency is compatible; (L)GPL dictionaries load at runtime | GPL-3.0 (blocks reuse of `mdpp-lang` etc.) |
| D12 | **Distribution:** GitHub Releases; Windows per-user installer + portable zip, **signed via SignPath Foundation from the first public build**; Linux AppImage + tar.gz, then Flathub, AUR, `.deb`; winget, Scoop | Free signing for OSS; unsigned Rust builds trigger Defender false positives (Ferrite) | Paid certificates; MSIX-only |
| D13 | **Updates: opt-in "check for updates"** (GitHub Releases API, notify only) | Principle 5; KISS | Background auto-update in 1.0 (revisit with Velopack if users ask) |
| D14 | **Toolkit gaps are fixed upstream first.** Known gaps (RESEARCH §4): public atomic multi-range edit + undo-transaction API, EOL-aware newline insertion. PR to gpui-kit (Apache-2.0, active); a `[patch]` fork only while a PR is pending | Keeps MD++ on released versions; others benefit | A permanent fork (upgrade cost multiplies with weekly releases) |
| D15 | **Search: own engine on the `regex` crate** over a text snapshot (regex, case, whole word, in selection, find in files) | gpui-kit's search is literal-only (Aho-Corasick). Copying the rope out costs ≈ 0.3 ms per MB; `regex-cursor` (rope search without a copy) supports ropey 1 only, the editor uses ropey 2 | A rope cursor adapter (revisit if search on very large files needs it) |

**Fallback if Phase 0 fails a gate** (§7, G1–G9): iced 0.14 with a custom virtualized editor widget on
`cosmic-text` (≈ +6–10 weeks, keeps everything below the UI layer), or — last resort — Tauri 2 +
CodeMirror 6. The crate split in §3 is what keeps that switch bounded: only `mdpp-app` is
toolkit-specific.

---

## 3. Architecture

### 3.1 Workspace

```
mdpp/
├─ crates/
│  ├─ mdpp-core/      no UI · document model, file I/O, encodings/EOL/BOM, atomic save,
│  │                  session + hot-exit store, settings & keymap loading, recent files, paths
│  ├─ mdpp-markdown/  no UI · incremental line lexer (highlight tokens), pulldown-cmark structural
│  │                  index (headings+slugs, links, refs, footnotes, images, fences, tables, front
│  │                  matter), source map for scroll sync, HTML export
│  ├─ mdpp-lang/      no UI · intellisense: completions, diagnostics, hover, go-to-definition,
│  │                  references, code actions, editing assists, table/list formatting,
│  │                  workspace index — all results as lsp-types
│  └─ mdpp-app/       bin `mdpp` · gpui-kit UI; adapters implementing InputHighlighter /
│                     CompletionProvider / HoverProvider / DefinitionProvider / CodeActionProvider
│                     over mdpp-lang; tabs, dock, palette, preview, status bar, settings UI,
│                     single instance, CLI
├─ assets/            fonts (OFL), icons, default themes, default settings.toml & keymap.toml,
│                     snippets, emoji/alert data
├─ docs/              PLAN.md, RESEARCH.md, ARCHITECTURE.md (living), user docs
└─ spikes/            bake-off & benchmark code behind the decisions (not shipped)
```

Rules: `mdpp-core`, `mdpp-markdown` and `mdpp-lang` never depend on GPUI (enforced by CI with
`cargo tree`). They are where >80 % of the logic and tests live. `mdpp-app` is glue and presentation.

### 3.2 Data flow

```
 key press ─▶ gpui-kit editor (ropey, UI thread)
                 │ edit + new rope
                 ├─▶ MdHighlighter::update  ── re-lex dirty lines (µs) ──▶ styled runs ─▶ paint
                 │
                 └─▶ revision += 1 ; snapshot = rope.clone()   (O(1))
                          │ debounce (settings: structure_debounce_ms, default 120)
                          ▼
                 background executor: pulldown-cmark ─▶ DocIndex { outline, links, refs,
                          │                                   anchors, fences, source map }
                          ▼                 (dropped if revision is stale)
                 UI thread: outline panel · diagnostics (squiggles) · preview · status bar

 completion request ─▶ context from the CURRENT line of the current snapshot (synchronous: "inside
                       `](#`?", "after `:`?") ─▶ candidates from DocIndex / WorkspaceIndex (may be one
                       revision old — they are names, not offsets) ─▶ popup
 workspace: notify watcher ─▶ incremental WorkspaceIndex (file list + headings per .md file)
 every hot_exit_interval_s (default 5): dirty docs ─▶ backup dir (atomic) ─▶ session.json
```

### 3.3 Documents and files
- `Document { id, path: Option<PathBuf>, encoding, bom, eol, revision, saved_state, backup_id }`;
  the text itself lives in the gpui-kit editor state (single owner) and is read as rope snapshots.
  `revision` only orders background results. **Dirty** = the undo-history position differs from the
  one recorded at save (falls back to a content hash), so undoing back to the saved text clears the dot.
- **Line endings:** the buffer is always LF. On open, CRLF/LF/CR is detected and stored per document;
  on save it is written back. A uniform-EOL file therefore round-trips byte-for-byte. A *mixed*-EOL file
  is flagged on open and saved with the EOL the user picks (default: the dominant one) — the one
  documented, visible exception to principle 1. (Needed because gpui-kit's Enter inserts `"\n"`.)
- **Encodings:** BOM/encoding detection (`chardetng`, `encoding_rs`) on open. Two different commands:
  *Reopen with encoding* (re-decode the bytes on disk; buffer untouched until confirmed) and *Save with
  encoding* (transcode on save; warns before saving characters the target encoding cannot represent).
- **Soft wrap** is on by default but only for documents up to `soft_wrap_max_bytes` (default
  256 KB): gpui-kit builds its wrap map by laying out every line before the first frame, measured at
  ≈ 0.55–0.6 s per MB (docs/PROGRESS.md). Lazy wrapping is an upstream improvement to propose.
- **Large-file mode** above `large_file_mb` (default 20, setting): highlighting only; structure,
  diagnostics, live preview and soft wrap off; per-line layout capped at `max_layout_line_chars`
  (default 10,000) so minified one-line files stay responsive.
- **Save:** encode with the document's encoding/BOM/EOL → temp file in the same directory → fsync →
  atomic replace (`ReplaceFileW` on Windows). Symlinks are resolved and the *target* is replaced; Unix
  permissions/ownership and Windows attributes are copied to the temp file; for hard-linked files, or
  when the temp file cannot be created (read-only directory), fall back to an in-place truncate-and-write
  of the target, with the hot-exit backup as the safety net. Record mtime/size so our own save doesn't
  trigger the "changed on disk" bar.
- **External change:** `notify` watcher on parent dirs; unmodified docs reload silently, modified docs
  show a non-modal bar (Reload — discard mine / Keep mine). A diff view is post-1.0.

### 3.4 Hot exit and sessions
- Every `hot_exit_interval_s`, each dirty document is written to `<data>/backup/<doc-id>.md` (atomic).
- `session.json` (versioned schema): windows, tabs and order, active tab, paths, cursors/selections,
  scroll offsets, dirty flags, backup ids, panel layout (gpui-kit dock layouts serialize).
- **Quitting the app** = flush and exit, no prompts (setting `confirm_quit_unsaved` for people who want
  one). **Closing a dirty tab** is different: it asks Save / Don't save / Cancel (Notepad++ behaviour);
  "Don't save" deletes that tab's backup.
- **Start** = restore the session. Untitled tabs restore from their backup unconditionally. For a file
  tab with a backup: disk unchanged since the backup → restore as dirty; **disk changed while MD++ was
  closed** (git pull, another editor) → restore as dirty *and* show a conflict bar
  (Keep my unsaved changes / Load the disk version / Save my changes as…). Never silently drop either side.
- Orphaned backups (no session entry) are kept and listed under *File → Recover unsaved…* until the
  user deletes them.

### 3.5 Theming
- Ship "MD++ Light" and "MD++ Dark" in gpui-kit's JSON theme schema, plus a high-contrast pair in v1.
- `theme = "system" | "light" | "dark" | "<theme name>"`; `system` follows GPUI's window-appearance
  events live.
- Markdown source styles are semantic tokens (`md.heading.1`, `md.marker`, `md.code`, `md.link.url`, …)
  resolved by the theme (`HighlightStyleResolver`), never colours in code. Contrast checked to WCAG AA.

---

## 4. Features and release scope

S = 1–3 days, M = 1–2 weeks, L = 3+ weeks (one developer).

### 4.1 Editor (Notepad++ core)

| Feature | Release | Size |
|---|---|---|
| Tabs (reorder, dirty dot, close others/right/unchanged, reopen closed `Ctrl+Shift+T`) | MVP | M |
| New / open / save / save as / save all / close; recent files; drag-and-drop to open | MVP | S |
| Hot exit + session restore (§3.4) | MVP | M |
| Find / replace bar: regex, case, whole word, in selection, match count (own engine, D15) | MVP | M |
| Go to line, word wrap toggle, zoom, line numbers, current-line highlight, multi-cursor | MVP | S |
| Encodings (UTF-8, UTF-8 BOM, UTF-16 LE/BE, Windows-125x/ISO-8859 via encoding_rs) and EOL — shown in the status bar; *Reopen with* / *Save with* encoding; EOL conversion | MVP | M |
| External-change detection; large-file mode | MVP | S |
| Single instance; CLI `mdpp file.md[:line[:col]]`, `mdpp <folder>` — `:line:col` parsed from the end (keeps `C:\…` intact); the second instance canonicalizes paths against *its own* working directory before forwarding | MVP | S |
| Command palette `Ctrl+Shift+P`; quick open `Ctrl+P` (files, `@` headings, `:` line) | v1.0 | M |
| Folder sidebar (simple tree) + find in files `Ctrl+Shift+F` | v1.0 | M |
| Focus/zen mode, typewriter mode | v1.0 | S |
| Settings UI for common options (font, theme, wrap, hot exit, preview) | v1.0 | M |
| Portable mode (marker file next to the executable) | v1.0 | S |
| Minimap / document map, macros | later | M |

### 4.2 Markdown

| Feature | Release | Size |
|---|---|---|
| Source highlighting: dimmed markers, scaled headings, code pills, links, front matter, fenced code (syntect, lazy) | MVP | M |
| Outline sidebar (live, click to jump, follows cursor) + breadcrumb | MVP | S |
| Split preview (toggle `Ctrl+Shift+V`), source-mapped scroll sync both ways, GFM tables, tasks, footnotes, alerts, images (local; SVG via resvg) | MVP | L |
| Export HTML (self-contained, theme CSS) | MVP | S |
| Status bar: Ln/Col (+ selection), words, characters, reading time, encoding, EOL, diagnostics count | MVP | S |
| Math in preview (`ratex-svg` + `resvg`) | v1.0 | M |
| Print / PDF export (via Typst CLI if installed; embedded engine later) | v1.x | M |
| Mermaid (external `mmdc` if installed; non-GPL engine when one exists) | later | M |
| Conceal / live-preview mode (hide markers off the cursor line via display-map layers) | later | L |

### 4.3 Intellisense — the differentiator

| Capability | Behaviour | Release | Size |
|---|---|---|---|
| **List assist** | Enter continues `-`, `*`, `1.`, `- [ ]`, `>`; Enter on an empty item ends the list; ordered lists renumber; Tab/Shift+Tab indent/outdent items | MVP | S |
| **Smart pairs & wrap** | `*` `_` `` ` `` `~` `[` `(` `"` wrap a selection; type-over closers; backspace deletes empty pairs | MVP | S |
| **Formatting commands** | Toggle bold/italic/code/strike (`Ctrl+B/I/E`), heading level up/down, insert link `Ctrl+K`, toggle task `Ctrl+Enter` (and click in preview) | MVP | S |
| **Link paste** | Pasting a URL over a selection makes `[selection](url)` | MVP | S |
| **Path completion** | In `](` / `![](` / `<img src=`: relative files and folders, images only for images, fuzzy | MVP | S |
| **Anchor completion** | `](#` → headings of this file (GitHub slug rules, duplicates `-1`, `-2`) | MVP | S |
| **Cross-file anchors** | `](other.md#` → headings of that file (needs the workspace index) | v1.0 | S |
| **Reference & footnote completion** | `[text][` → defined labels; `[^` → footnote labels | MVP | S |
| **Emoji** | `:smi` → 😄 (from `emojis` data) | MVP | S |
| **Fence & alert completion** | ```` ``` ```` → language names; `> [!` → NOTE / TIP / IMPORTANT / WARNING / CAUTION | MVP | S |
| **Diagnostics** | Missing file, missing anchor in this file, undefined reference, unused definition, duplicate footnote; severities in settings | MVP | M |
| **Workspace diagnostics** | Missing anchors in *other* files; links broken by a rename/move | v1.0 | S |
| **Go to definition** | Ctrl+click / F12 on a link: open file, jump to heading, reference definition, footnote | MVP | S |
| **Hover** | Link → target + heading preview; footnote → its text; reference → URL; image → thumbnail | MVP / v1.0 (thumbnail) | S |
| **Table assist** | Tab/Shift+Tab between cells, align on leaving a cell (one undo step), Enter in the last row adds a row, insert/delete column commands | v1.0 | M |
| **Image paste / drop** | Clipboard image (GPUI gives encoded bytes + format) or dropped file → saved under `assets_dir_template` (default `./assets/{doc_stem}/`) → `![](…)` inserted. Untitled document: asks to save first | v1.0 | S |
| **Rename heading** | Updates anchors in this file; workspace-wide with preview. Accounts for duplicate-slug shifts (`#a-1` becoming `#a`) and shows them before applying | v1.0 / v1.x | M |
| **Find references** | Who links to this heading / this file | v1.0 | S |
| **Code actions** | Create missing file; inline ↔ reference link; fix heading-level jump; generate/update TOC | v1.0 | M |
| **Snippets** | Data-defined (table, details, code block, front matter …) in completion and palette | v1.0 | S |
| **Lint rules** | Heading increments, duplicate headings, table column count, trailing spaces … (built-in subset; evaluate `rumdl` as engine) | v1.0 | M |
| **Spelling** | `spellbook` + user-installed Hunspell dictionaries (incl. Turkish with locale-aware casing); add-to-dictionary | v1.0 | M |
| **Front matter keys, LaTeX commands in `$…$`, wikilinks `[[…]]` + backlinks** | | v1.x | M |
| **Grammar (English, harper-core); external LSP client (Marksman, markdown-oxide, harper-ls)** | opt-in | later | L |

Latency budget: completion p95 ≤ 30 ms in a 1,000-file workspace; diagnostics ≤ 150 ms after the last
keystroke (debounce included). The **workspace** is a folder the user opened explicitly (`mdpp <folder>`,
*Open folder*); only then is it indexed recursively, capped by `workspace_max_files` (default 5,000).
A file opened on its own gets path completion from a *non-recursive* listing of its directory and
nothing is crawled — opening `~/notes.md` must never index the home folder.

### 4.4 Platform

| Feature | Release | Size |
|---|---|---|
| Light / dark / follow-system themes | MVP | S |
| `.md` / `.markdown` file association (installer on Windows; `.desktop` + MIME on Linux) | MVP | S |
| Native menus where the platform has them; default keymap (Notepad++/VS Code hybrid, §6.3) | MVP | S |
| Crash log (panic hook → `<data>/logs/`), "report a bug" opens a pre-filled GitHub issue (no auto-upload) | MVP | S |
| UI translations (English, Turkish) via data files | v1.0 | M |
| Accessibility pass (AccessKit roles, full keyboard use, contrast) | v1.0 | M |

**MVP** = 0.1 public alpha. **v1.0** = stable. **v1.x / later** = after 1.0, driven by issues.

---

## 5. Performance budget

Measured baseline (RESEARCH.md §2): gpui-kit first frame ~100 ms, 95 MB empty, +9 MB per MB of text.

| Metric | Budget | How it is checked |
|---|---|---|
| Cold start to first painted frame (session with 5 docs) | ≤ 300 ms Linux, ≤ 400 ms Windows | `hyperfine` + startup trace, nightly CI job |
| Open a 1 MB file to editable | ≤ 150 ms | integration bench |
| Keystroke → paint, 1 MB document | p99 ≤ 16 ms | `tracing` spans, perf test |
| Structural re-parse (index + preview blocks) | Phase 0 baseline on a dense 1 MB document: index ≤ 40 ms, preview blocks ≤ 60 ms, background only. Target after the single-pass merge (one parse feeding both): ≤ 50 ms together | `cargo test --release -- --ignored` timing tests; nightly CI |
| Completion | p95 ≤ 30 ms | `divan` bench over a fixture workspace |
| RAM, 10 typical docs (≤ 100 KB each) | ≤ 150 MB | perf test reads RSS |
| Idle CPU (window open, no input for 10 s) | 0 % | perf test |
| Binary (stripped, ship profile) | ≤ 40 MB; download ≤ 18 MB compressed (baseline: 28 MB before any MD++ code) | release job warns above budget — advisory, never a reason to switch toolkit |

Budgets are settings in `perf-budget.toml` read by the perf job, so tightening one is a data change.

---

## 6. UI / UX

### 6.1 Layout

```
┌────────────────────────────────────────────────────────────────────────────────────────┐
│ ≡  README.md ●   notes.md   + │                                      ◐   ⧉   ⋯   — ▢ ✕ │ title bar + tabs
├──────────────┬───────────────────────────────────────────┬─────────────────────────────┤
│ OUTLINE      │  1  # Project                              │  Project                    │
│ ▾ Project    │  2                                         │  ─────────                  │
│   Install    │  3  Fast, **small** editor. See [docs](#u… │  Fast, small editor. See    │
│   Usage   ◀  │  4                         ┌─────────────┐ │  docs                       │
│   License    │  5  ## Usage               │ # usage     │ │                             │
│              │  6  - [ ] write tests      │ # install   │ │  ☐ write tests              │
│ FILES        │  7  - [x] ship it          └─────────────┘ │  ☑ ship it                  │
│ (opt-in)     │                                            │                             │
├──────────────┴───────────────────────────────────────────┴─────────────────────────────┤
│ Ln 3, Col 41 · 214 words · 1 min · UTF-8 · LF · Markdown · ⚠ 1                          │ status bar
└────────────────────────────────────────────────────────────────────────────────────────┘
   sidebar (Ctrl+Shift+B)                   editor             preview (Ctrl+Shift+V)
```

- Default first run: editor only + outline collapsed, preview off — the "Notepad" moment. Everything
  else is one shortcut or the palette away.
- Empty state: New · Open · Open folder · recent files, with their shortcuts; a `Welcome.md` that
  demonstrates the intellisense features (and doubles as an end-to-end test fixture).
- Find/replace is an inline bar at the top of the editor, never a modal dialog.
- Changes on disk, errors and updates appear as non-modal bars or notifications, never modal dialogs.

### 6.1a Zed-style menu bar and settings window (requested 2026-11-17)

The owner likes the current colours, theme and layout and asked not to change much: the ≡ menu had
grown too long. Changes, smallest first:

| Part | Content | Release |
|---|---|---|
| Menu bar | ≡ opens a row of short menus *File · Edit · Selection · View · Go · Help* in place (as in Zed), built from `assets/menus.toml`; each item shows its shortcut from the keymap. Everything else stays where it is. | MVP |
| Settings window | Zed-like: search, category sidebar, one row per setting with its description and a control (switch, dropdown, number, text). **Metadata-driven**: names, defaults and descriptions come from `default-settings.toml`, categories and controls from a small UI table; a change is written to the user's `settings.toml` (format-preserving) and hot reload applies it. "Edit in settings.toml" opens the file. | MVP |
| Small Zed touches, only if wanted | back/forward jump history (Alt+←/→), breadcrumb (file › heading), icon toggles in the status bar | MVP (breadcrumb), later |
| Zed features | command palette (Ctrl+Shift+P), quick open (Ctrl+P), theme selector, folder panel, find in files, split panes | v1.0 (palette, quick open, theme selector first) |

### 6.2 Visual design
- Calm, typographic, low-chrome (references: Zed, iA Writer, Windows 11 Notepad, GNOME Text Editor).
- Editor font: one bundled OFL monospace (JetBrains Mono); optional proportional mode; UI uses the OS
  font. 4 px spacing grid; semantic colour tokens only.
- Markdown source styling: markers at reduced contrast, H1–H3 slightly larger, code in pills, links
  coloured with dimmed URLs.
- Icons from gpui-kit's set; MD++ logo is original — nothing borrowed from Notepad++.
- A short design pass with real screenshots in Phase 1 (both themes, 100 % / 150 % scale) before
  building more UI.

### 6.3 Default keys (data in `keymap.toml`, user-overridable)

| Action | Key | Action | Key |
|---|---|---|---|
| New tab | `Ctrl+N` | Open file / folder | `Ctrl+O` / `Ctrl+K Ctrl+O` |
| Save / save as / save all | `Ctrl+S` / `Ctrl+Shift+S` / `Ctrl+Alt+S` | Close tab / quit | `Ctrl+W` / `Ctrl+Q` |
| Command palette | `Ctrl+Shift+P`, `F1` | Quick open | `Ctrl+P` |
| Find / replace | `Ctrl+F` / `Ctrl+H` | Find in files | `Ctrl+Shift+F` |
| Next / prev match | `F3` / `Shift+F3` | Go to line | `Ctrl+G` |
| Toggle preview | `Ctrl+Shift+V` | Outline | `Ctrl+Shift+O` |
| Bold / italic / code | `Ctrl+B` / `Ctrl+I` / `Ctrl+E` | Insert link | `Ctrl+K` |
| Toggle task | `Ctrl+Enter` | Go to definition | `F12`, `Ctrl+click` |
| Duplicate / delete line | `Ctrl+D` / `Ctrl+Shift+K` | Move line | `Alt+↑/↓` |
| Reopen closed tab | `Ctrl+Shift+T` | Next / prev tab | `Ctrl+Tab`, `Ctrl+PgDn/PgUp` |
| Zen mode | `Shift+F11` | Zoom | `Ctrl+=`, `Ctrl+-`, `Ctrl+0` |

`Ctrl+B` is bold in the editor; the sidebar toggle therefore defaults to `Ctrl+Shift+B`.

---

## 7. Roadmap

Sizing assumes one developer working with Claude Code: roughly **4–5 months to 1.0** (15–21 weeks), alpha 0.1 after
~2.5 months. Each phase ends with its exit criteria **checked**, not with a date.

### Phase 0 — Foundations and gates (≈ 1 week)
Set up: public GitHub repo `mdpp`, MIT/Apache licence files, workspace skeleton (§3.1), CI (fmt, clippy,
test, `cargo-deny`, Linux + Windows), `CONTRIBUTING.md`, `SECURITY.md`, issue templates, pinned
toolchain, `.cargo/config.toml` with mold/lld.

Prove the risky assumptions with throw-away spikes in `spikes/`, each a pass/fail gate. **A** = automated
(CI-repeatable), **M** = manual checklist on real machines/VMs, recorded with screenshots in the PR.

| Gate | | Pass condition |
|---|---|---|
| G1 Highlighter | A | Own `InputHighlighter` (line-state lexer) on the 1 MB file: keystroke → paint p99 ≤ 16 ms |
| G2 Completion | A | Own `CompletionProvider`: popup at the caret, filtered, ≤ 50 ms |
| G3 Input methods | M | Turkish Q/F layouts, dead keys and AltGr; CJK through fcitx5 on Wayland and Windows TSF — composition inline |
| G4 Windows | M | Windows 11: crisp text at 100/125/150 %; runs over RDP (WARP); file dialog; clipboard image |
| G5 Linux | M | Wayland, X11 (XWayland) and lavapipe (software Vulkan) all start and type normally |
| G6 Preview | A | Own preview from pulldown-cmark events on gpui-kit primitives in a virtual list: GFM tables, tasks, footnotes, alerts, highlighted code, images; scroll sync lands on the right block; 1 MB document scrolls smoothly |
| G7 Footprint | A | Measure (advisory, §5): ship-profile size with trimmed features, syntect cost, idle CPU 0 % with blink idle timeout |
| G8 Upgrade drill | A | Move the spike to the next gpui-kit release; record the breakage cost (decides upgrade cadence) |
| G9 Editing API | A | Multi-cursor formatting command applied as **one** undo step, and Enter inserting the document's EOL — via an upstream gpui-kit PR (D14) or a temporary `[patch]` |

Exit: G1–G6, G8, G9 pass, or the fallback in §2 is chosen and this plan updated. G7 informs budgets only.

### Phase 1 — Editor shell (≈ 2–3 weeks)
Window, title bar with tabs, open/save/encodings/EOL, hot exit + sessions, find/replace bar, status
bar, themes (light/dark/system), settings and keymap loading, single instance + CLI, crash log.
Exit: a week of dogfooding on real files without data loss; startup and RAM inside budget; property
tests for encoding/EOL round-trips and atomic save.

### Phase 2 — Markdown core (≈ 2–3 weeks)
`mdpp-markdown`: line lexer, structural index, source map, HTML export. UI: highlighting, outline,
split preview with scroll sync. Exit: snapshot tests over a corpus (CommonMark spec examples + GFM +
real READMEs); structural re-parse within budget; sync scroll lands on the right block in every corpus
file.

### Phase 3 — Intellisense MVP (≈ 3–4 weeks)
`mdpp-lang` with every MVP row of §4.3 + workspace index + adapters. Exit: each feature has unit tests
on fixture documents and one end-to-end UI test; latency budgets met.

### Phase 4 — Alpha 0.1 (≈ 1 week)
Release pipeline: tag → build Linux/Windows → package (installer, zip, AppImage, tar.gz) → SignPath
signing → GitHub Release with checksums and generated changelog (`release-plz` / `git-cliff`). README
with screenshots, website on GitHub Pages, feedback channel (GitHub Discussions).
Exit: a clean Windows 11 VM installs and runs the signed build without SmartScreen/Defender blocks;
AppImage runs on Ubuntu LTS and Fedora.

### Phase 5 — Toward 1.0 (≈ 4–6 weeks)
Everything marked v1.0 in §4: palette, quick open, folder sidebar, find in files, table assist, image
paste/drop, rename/references/code actions, snippets, lint rules, spelling, math, focus/typewriter,
settings UI, portable mode, Turkish UI, accessibility pass, Flathub/winget/Scoop/AUR packages.
Exit: beta builds used daily; all v1.0 rows done; no open data-loss or crash bugs.

### Phase 6 — Hardening and 1.0 (≈ 2–3 weeks)
Fuzzing (`cargo-fuzz`) of lexer, parser adapters and editing assists; perf budgets enforced in CI;
startup and memory profiling pass; docs complete; 1.0 release.

### After 1.0 (by demand)
PDF export, wikilinks and backlinks, conceal/live-preview mode, external LSP client, grammar checking,
Mermaid, minimap, macOS build, plugin/scripting API.

---

## 8. Engineering practice

- **Testing pyramid.** `mdpp-core`, `mdpp-markdown`, `mdpp-lang`: unit tests + `insta` snapshots
  (tokens, outline, diagnostics, completions, HTML) + `proptest` for edit/undo, encoding and EOL
  round-trips + `cargo-fuzz` targets. `mdpp-app`: gpui-kit/GPUI UI tests for the critical flows
  (open → edit → hot exit → restore; completion accept; find/replace). Corpus of real-world Markdown in
  `tests/corpus/`.
- **Performance as a test.** `divan` benches for lexer, structural parse and completion; a nightly job
  checks §5 budgets from `perf-budget.toml` and fails on regression.
- **CI on every PR:** `cargo fmt --check`, `clippy -D warnings`, tests on Linux + Windows,
  `cargo-deny` (licences, advisories, duplicate versions), "core crates do not depend on GPUI" check.
- **Dependencies:** gpui-kit and the `gpui-pre` snapshots pinned exactly (`=`), upgraded in a dedicated
  PR on the cadence set by G8; every other new dependency needs a one-line justification in the PR
  ("what existing crate/GPUI API could not do this").
- **Commits & releases:** Conventional Commits → generated changelog; SemVer; channels: nightly
  (main), beta, stable.
- **Docs:** `ARCHITECTURE.md` kept current with each phase; user docs in `docs/`; every setting
  documented with its default in the shipped `settings.toml` comments (single source).
- **Open-source hygiene:** code of conduct, contribution guide, "good first issue" labels from
  Phase 4, a public roadmap = §4 tables as GitHub issues/milestones.

---

## 9. Risks

| Risk | Likelihood / impact | Mitigation |
|---|---|---|
| gpui-kit / GPUI API churn (weekly releases, pinned pre-release GPUI) | High / Medium | Exact pins; scheduled upgrade PRs; all logic in GPUI-free crates; G8 upgrade drill |
| No CPU renderer (GPU required) | Medium / Medium | Windows falls back to WARP; Linux to lavapipe; G4/G5 verify; clear error message if no adapter |
| Binary above budget | Medium / Low | Trim features (assets, unused components), fat LTO, opt-level tests; budget gate in release job |
| IME or HiDPI defects in the editor widget | Medium / High | G3/G4 before any feature work; upstream fixes to gpui-kit (Apache-2.0, active) |
| Toolkit API gaps (private multi-range edits, hardcoded `"\n"`, literal-only search, slow built-in Markdown view) | Certain / Medium | Known and planned: D6, D14, D15, G9 |
| Data loss in save/restore edge cases (symlinks, read-only dirs, disk changed while closed, mixed EOL) | Medium / High | §3.3–3.4 rules; property and integration tests for each rule in Phase 1 |
| Windows AV / SmartScreen flags unsigned builds | High if unsigned / High | SignPath signing from the first public build; no shell spawning |
| Scope creep toward Obsidian/Typora | High / Medium | Non-goals (§1.3); after-1.0 items need user demand |
| Dictionary licences | Low / Medium | Never bundle; download on demand with licence shown |
| Name confusion ("Markdown++" plugins) | Medium / Low | Brand as MD++ / `mdpp`; no Notepad++ marks |
| Solo-maintainer bus factor | Medium / Medium | Docs, tests, small crates, contributor-friendly issues |

---

## 10. Next steps

1. ~~Create the public GitHub repository~~ — done: <https://github.com/mcftc/mdpp>, which fixes the
   Flathub app id as `io.github.mcftc.mdpp`.
2. Phase 0: workspace skeleton + CI, then the G1–G9 spikes (start with G1–G3 and G9, the ones most likely to
   change the toolkit decision).
3. Apply to SignPath Foundation as soon as the repo is public (approval takes days).
