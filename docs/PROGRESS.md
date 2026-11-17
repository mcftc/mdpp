# MD++ — progress

Status against [PLAN.md](PLAN.md). Numbers are measured on the dev machine (Ryzen 5 5500U, 22 GB,
AMD iGPU, CachyOS, KDE Plasma 6 Wayland) with the built-in measurements:
`MDPP_BENCH=startup | typing:300 | idle:15 target/release/mdpp <file>` and
`cargo test --release -p mdpp-lang -- --ignored --nocapture`.

## Phases 2–3 — Markdown core and intellisense MVP (in progress, started 2026-11-17)

| Item | Status | Evidence |
|---|---|---|
| Lists, tasks, quotes on Enter | Done | Continue `-`/`*`/`+`/`1.`/`1)`/tasks/`>` (also nested in quotes); renumber the rest of an ordered list in the same edit; Enter on an empty item ends the list with a blank line (so the next paragraph is not a lazy continuation); Shift+Enter is always a plain break; completion/code-action menus, selections and code blocks get the editor's own Enter. One undo step each. `mdpp_lang::assist` (tested) + keystroke check through the real keymap. |
| Formatting and links | Done | Ctrl+B/I/E, Ctrl+Shift+X toggle bold/italic/code/strikethrough (wrap, unwrap inside or around the selection; `*` inside `**` handled); Ctrl+K inserts a link (a selected URL becomes the destination); Ctrl+Enter toggles/creates a task; a URL pasted over selected text becomes `[text](url)`. Bold checked by keystroke; paste by unit test only (Wayland gives the clipboard to focused windows; test windows are unfocused) — manual check. |
| Auto-closing pairs | Done | `()`, `[]`, `{}`, ` `` ` from the `auto_close_pairs` setting through gpui-kit's language config (quotes and `*` deliberately off). Checked by simulated typing. Wrapping a selection by typing a pair character is not available in gpui-kit; the toggles above cover it. |
| Outline sidebar | Done | Ctrl+Shift+O; headings indented by level, virtualized list, current heading highlighted, click jumps; state kept in the session. Check: 300 headings, jump lands on the heading. |
| Status bar words / reading time | Done | Counted in the background analysis (markup-only tokens excluded); reading speed is a setting. |
| Go to definition, hover | Done | F12 / Ctrl+click: heading in this file, footnote definition, other file (opened in a tab, at its heading), `http(s)` in the browser, `mailto:` via the system; wiki links resolve to `Name.md`. Hover explains the target (heading text, file path + "not found", footnote text, URL, or why an anchor does not resolve). Fresh index per request on a background thread. Check: F12 on `[top](#top)` moves the caret to the heading. |
| HTML export | Done | Ctrl+Shift+E: self-contained HTML with `assets/export.css` (light/dark by the reader's preference), written atomically. Check: exported file contains the rendered heading. |
| List nesting, headings, line commands | Done | Tab / Shift+Tab nest a list item under its previous sibling (to that sibling's content column, so `1.` items nest by 3) or back to its parent's level; every selected line moves by the same amount; outside lists the editor's own indent runs, and completion menus keep Tab. Ctrl+Shift+] / [ add or remove a `#` (paragraph = level 0). Ctrl+D duplicate, Ctrl+Shift+K delete, Alt+↑/↓ move lines — one undo step each. `mdpp_lang::assist` / `lines` (tested); keystroke check through the real keymap. |
| Editor commands | Done | Ctrl+G go-to-line bar (`12`, `12:5`), zoom Ctrl+= / Ctrl+- / Ctrl+0 from the new `font_size` setting (kept in the session), Alt+Z word wrap for all tabs (also large files; kept in the session). Checked by keystrokes. |
| Tabs and files | Done | Right-click a tab: Close / Close Others / Close to the Right / Close Saved (Cancel on any unsaved tab stops the rest). Ctrl+Shift+T reopens the last closed file at its line; Ctrl+Alt+S saves every unsaved tab; recent files (setting `recent_files_max`, kept in the session); files dropped on the window open. Check: close → reopen at the same line, first in recent files. |
| ≡ menu | Done | Left of the tabs: every command with its shortcut (file, format, lines, view, settings, quit), so nothing depends on knowing the keys. Settings / Keyboard Shortcuts open the user's file, created on first use with every default commented out (check: created and opened). Visual review: manual. |
| Line-ending conversion | Done | Status-bar menu LF / CRLF / CR; the tab shows unsaved changes until saved. Check: CRLF file → LF → saved bytes have no CR. |
| Diagnostics settings | Done | New rules `unused-definition` (default hint) and `duplicate-footnote`; every rule's severity (`error` … `off`) in `[diagnostics]` of settings.toml, merged entry by entry. A test ties the rule list to the defaults. |
| Status bar | Done | `(N selected)` after the caret position; line endings menu. |
| Fixed: unsaved changes lost with `hot_exit = false` | Done | Quit and the window's close button now ask Save / Don't Save / Cancel per unsaved tab when hot exit is off (before: closed without asking). The session still keeps view state and recent files. Check: close refused and question shown with a dirty tab. |
| Fixed: files opened later were not watched | Done | The watched-folder list was only rebuilt at startup and when a tab closed, so a file opened afterwards (dialog, drop, second launch, recent) or saved under a new name in another folder missed outside changes. Now rebuilt on every open and rename. Check: external-change detection after close + reopen. |
| Code-block colours | Done | Fenced code is coloured by syntect with bat's ~200 syntaxes (pure-Rust regex, the crates gpui-component's own `syntect` feature uses). Scopes map to the app theme's highlight names through `data/code-scopes.txt`, so code follows light/dark; language aliases (`c#`, `shell`, `text`) in `data/code-aliases.txt`. Measured: 0.1–0.5 ms per line (Rust 0.33, Python 0.43, JSON 0.01) — too slow for a frame, so colours are computed in the background after the typing pause, reusing unchanged blocks, and looked up by content (whole block exactly, else line by line), so they stay put while text moves; only a freshly typed line is coloured on the UI thread (≤ 4 lines a frame). Quote markers and the fence's indentation are not code (same as the preview). Setting `code_highlight_max_lines` (default 5000) caps the background work. The preview uses the same colours. Check: `fn` in the fixture's rust block is a keyword. |
| Menu bar | Done | The ≡ list had grown too long: ≡ now opens a row of short menus — File · Edit · Selection · View · Go · Help — as in Zed, built from `assets/menus.toml` (labels + the keymap's action names, so shortcuts shown always match; check marks and recent files filled by the app). Unknown actions, checks or lists in the file are reported at startup. Check: every menu action reaches a handler from the editor (0 unhandled). Help → Report a Bug opens GitHub's issue form with version and system filled in. Look: manual. |
| Cross-file anchors in completion and diagnostics; table assist; image paste; rename | Next (v1.0 list) | |
| Settings window (Zed-like, metadata-driven), breadcrumb | Next | PLAN §6.1a |

Note on the keystroke checks: a test window can receive keys typed elsewhere on the desktop (KDE gives new windows focus despite `focus: false`); runs while nobody types pass every assist check. A second effect: GPUI routes an action through the last frame it drew, so when the compositor stops drawing a background test window, an action dispatched right after a tab switch can miss (seen once: Save, preview and find not handled, every other step fine; the next runs passed). Real use is unaffected — user input always comes from a drawn frame.

Measured after this round (release): startup 173 ms README / 194 ms 1 MB; typing frame work p99 3.0 ms on 1 MB; idle 0 frames. 94 tests.

After code colours and the menu bar: startup 181 ms README / 203 ms 1 MB (median of 5); idle 0 frames, 0.00 % CPU; typing frame work p99 4.2–5.2 ms on 1 MB — measured A/B against the previous build in alternating runs on a busy machine (load average 7–13), where the previous build gave 4.2–5.1 ms: no measurable difference. Syntax set load 2.5 ms (in the background). 100 tests.

## Phase 1 — editor shell (started 2026-11-17)

| Item | Status | Evidence |
|---|---|---|
| Settings and keymap as data | Done | `assets/default-settings.toml` is the single source of defaults (Rust parses it, repeats nothing); user `settings.toml` / `keymap.toml` in the platform config folder (Linux `~/.config/mdpp/`) are laid over them. Unknown keys, bad values, unknown actions and invalid keystrokes are reported and skipped — verified with a scratch config. Hot reload: with item 5. |
| Tabs | Done | New/Open (native dialog, multi-select)/Close/Next/Previous, opening an open file switches to it, a pristine untitled tab is replaced, closing the last tab leaves an empty one. Closing a dirty tab asks Save / Don't Save / Cancel (Save closes only after a successful save). Check: two files open, only the active one saved. |
| Hot exit + session restore | Done | Backups of dirty tabs every 5 s (setting) in the background, one write at a time; quit (Ctrl+Q) and window close flush synchronously, and a generation counter stops any older queued write from overwriting the flush. Restores tabs, active tab, preview state, cursors, unsaved text (also untitled). A file changed on disk while MD++ was closed: unsaved version restored + warning (the plan's three-button conflict bar is still to do). Checks: file unchanged on disk after quit, dirty tab restored, untitled restored, CLI file opened on top of the session. |
| Find / replace (regex) | Done | Own bar (Ctrl+F / Ctrl+H, F3 / Shift+F3, Enter / Shift+Enter, Escape) replacing gpui-kit's literal search: literal or regex (multi-line, `$1` groups in replacements), match case, whole word (letters/digits/`_` as word characters, so `c++` works), in selection; all matches highlighted, current selected and revealed; Replace and Replace all are one undo step each; 10,000-match cap. Engine `mdpp_core::search` with tests. D15 adjusted: `regex` on a text snapshot — `regex-cursor` supports ropey 1 only. Check: bar finds 300 = engine's 300; replace-all leaves 0. Known gap: the in-selection range does not follow edits made while the bar is open. |
| External change detection + settings hot reload | Done | `notify` watches the folders of open files and the config folder (non-recursive), debounced (setting). Own saves are recognized by the file stamp. Clean tab: silent reload (one undo step, cursor kept); dirty tab: banner Reload (discard mine) / Keep mine; deleted file: banner, tab becomes unsaved so hot exit keeps the text. settings.toml edits apply live (blink, debounces; wrap/line numbers/fade/preview width for tabs opened afterwards); keymap changes still need a restart (rebinding would also drop gpui-kit's own bindings). Checks: clean reload, dirty conflict, deletion; settings reload with a warning for an unknown key. |
| Legacy encodings, reopen/save with encoding | Done | Detection: BOM → valid UTF-8 → `chardetng` guess; binary files (NUL bytes) refused. Decode with `encoding_rs`; UTF-16 LE/BE encoded by hand (encoding_rs only decodes it). Saving refuses characters the encoding cannot hold and names the first one. Status-bar encoding menu: Reopen with / Save with (list in `assets/encodings.txt`); the session records each tab's encoding. Checks: a Windows-1254 Turkish file and a UTF-16LE file round-trip byte for byte through edit + save. The menu itself: manual check. |
| Single instance, `file:line:col` | Done | `mdpp a.md:12:7` (parsed from the end; `C:\…` intact; an existing file named `x:12` wins). A second launch forwards its files over a per-user local socket (`interprocess`: Linux abstract namespace / Windows named pipe — no stale files) and exits; the running window opens them at the position and comes forward. Measurements never forward (they run their own instance); `MDPP_INSTANCE` names the socket for tests. Check: second launch exits in 5 ms, the first instance opened `b.md` at line 3, column 2 (relative path resolved against the second launch's folder). |
| Crash log | Done | Panic hook writes `<data>/logs/crash-*.log` (version, OS, thread, location, message, backtrace), keeps 10, the next start mentions it once; nothing is uploaded. Check: deliberate panic → report written → announced on the next start only. |

**Phase 1 status:** all items built and checked end to end. Re-measured after Phase 1 (release build): startup 158 ms README / 195 ms 1 MB; idle 0 frames, 0.00 % CPU (also with a 1 MB document); typing frame work p99 2.6 ms on 1 MB; RSS 122 MB with 1 MB open; 67 tests. Exit criteria still open: a week of dogfooding without data loss (needs you), property tests (proptest) for encoding/EOL round-trips and atomic save, the three-button conflict bar for files changed while MD++ was closed, and the manual checks listed below.

## Phase 0 — foundations and gates (started 2026-11-17)

### Done
- Workspace (`mdpp-core`, `mdpp-markdown`, `mdpp-lang`, `mdpp-app`), MIT/Apache-2.0 licence files,
  pinned toolchain (1.97.1), workspace lints, `release` / `dist` / `profiling` build profiles.
- `cargo deny check` green (advisories, bans, licences, sources); CI workflow for Linux + Windows with
  fmt, clippy, tests, `cargo-deny` and the "core crates stay UI-free" check — **not run yet: the
  GitHub repository does not exist.**
- `mdpp-core`: UTF-8/BOM/EOL decode and encode (buffer always LF, uniform files round-trip byte for
  byte, mixed EOL flagged); settings with documented defaults.
- `mdpp-markdown`: incremental line-state lexer (fenced code, front matter, comments, math; inline
  syntax via pulldown-cmark), GitHub slugs, structure index (headings/anchors, links, references,
  footnotes, broken references). A 2,000-random-edit test checks incremental == full rebuild.
- `mdpp-lang`: completion contexts and candidates (anchors, paths, references, footnotes, emoji,
  fence languages, alerts) from data files; diagnostics (missing anchor, missing file, undefined
  reference).
- `mdpp-app`: window with the gpui-kit editor, our highlighter and completion provider plugged in,
  background analysis (index + diagnostics, debounced, revision-checked) shared with completion,
  status bar, light/dark following the OS, caret blink with idle timeout, built-in measurements.
- **Save / Save As** (Ctrl+S / Ctrl+Shift+S, native dialog): encoding, BOM and line endings kept;
  atomic replace with symlink, permission, hard-link and read-only-directory handling; dirty marker
  in the title that clears when an edit is undone back to the saved text. `MDPP_BENCH=check`
  verifies the saved bytes through the real Save action (CRLF file: exact match).
- **Preview** (Ctrl+Shift+V): see G6.
- 43 tests + 3 opt-in timing tests, all passing; clippy and rustfmt clean. (52 tests after the first Phase 1 items.)

### Gates

| Gate | Status | Evidence |
|---|---|---|
| G1 Highlighter | **Pass (automated part)** | Frame work after a keystroke (frame start → editor painted): p99 **1.2 ms** README, **2.0 ms** 200 KB, **2.0 ms** 1 MB. End-to-end keystroke → paint p99 16–32 ms is the compositor's frame pacing (one or two 60 Hz frames) for a background test window — re-check with the window focused (manual). |
| G2 Completion | **Pass (latency)**, UX manual | Anchor completion on 1 MB: p95 **4.5 ms** with the background index, **31 ms** cold (popup budget 50 ms); emoji 0.08 ms. Popup placement and feel: manual check pending. |
| G3 Input methods | Pending (manual) | Turkish Q/F, dead keys, AltGr; fcitx5 CJK on Wayland; Windows TSF. |
| G4 Windows | Pending (manual) | Needs a Windows 11 machine/VM: build, HiDPI text, RDP. |
| G5 Linux | Wayland ✓ | X11 (XWayland) and lavapipe not tried yet. |
| G6 Preview | **Pass (automated part)** | Own renderer from pulldown-cmark blocks in a GPUI virtual list: headings, paragraphs with inline styles, lists, task lists, nested lists, tables with alignment, quotes, GitHub alerts, code (language label, no syntax colours yet), images (local; remote only if enabled), footnotes, HTML, front matter. `MDPP_BENCH=check`: editor top line → preview top block exactly as expected (block 285/606 on 19 KB, 5,769/11,556 on 1 MB). Typing with the preview open: frame work p99 **3.2–3.4 ms**. RSS with a 1 MB document: 121 MB → 143 MB with the preview. Background parse on a dense 1 MB document: index 22 ms + blocks 38 ms. Visual review of both themes: manual. |
| G7 Footprint | Measured | Startup to first paint: **160 ms** README, **270 ms** 200 KB (wrapped), **185 ms** 1 MB. RSS **99 MB** README, **110 MB** 1 MB. Idle (15 s, no input): README **0 frames / 0.00 %**, 1 MB **20 frames / 0.40 %** (see finding 2). Binary: release (thin LTO) 41.8 MB; **dist 29.3 MB → 11.8 MB gzip / 7.9 MB xz** (budget ≤ 40 MB / ≤ 18 MB). |
| G8 Upgrade drill | Waiting | gpui-kit 0.10.0 is the latest release; drill on the next one. |
| G9 Editing API | Not started | Upstream PR for atomic multi-range edits + EOL-aware Enter (D14). |

### Findings
1. **Soft wrap lays out the whole document before the first frame** (profiled: time in
   `cosmic_text` shaping for every line) — ≈ 0.55–0.6 s per MB. Mitigation shipped: soft wrap only up
   to `soft_wrap_max_bytes` (256 KB); 1 MB first paint went from 705 ms to 185 ms. Upstream idea:
   lazy wrap map.
2. **Idle repaints with large documents**: about 1.3 frames/s (0.4 % CPU) with a 1 MB document and no
   input in Phase 0; no longer reproduces after Phase 1 (0 frames, 0.00 %), cause not identified.
3. Completion must not rebuild the structure index per request on large documents (24–32 ms on
   1 MB); it now reuses the index published by the background analysis.
4. GPUI's Linux path renders text with cosmic-text (not DirectWrite-class hinting); judge text
   quality during G4/G5 manual checks.
5. The index and the preview each run their own pulldown-cmark pass; one pass feeding both is
   the next optimization (PLAN §5 budget re-baselined until then).
6. GPUI's `secondary` key modifier cannot be used with `KeyBinding::new` (needs OS context); the
   defaults bind `ctrl-…` until the keymap file arrives in Phase 1.

### Next
1. Manual gates G3–G5 and the visual review of the preview (needs you at the keyboard; Windows
   machine or VM for G4).
2. G9 upstream PR (atomic multi-range edits, EOL-aware Enter); single-pass index + preview.
3. Create the GitHub repository so CI runs.
