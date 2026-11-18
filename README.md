# MD++

A free, open-source, lightweight Markdown editor for Linux and Windows, written in Rust —
Notepad++'s speed and reliability, with Markdown intellisense built in.

**Status: Phases 2–3** (Markdown core and intellisense) — usable for daily editing, not yet
released.
See [docs/PROGRESS.md](docs/PROGRESS.md).

- Tabs (close others / to the right / saved, reopen closed), open/save/save all, recent files,
  drag-and-drop, encodings (UTF-8/16, legacy code pages, reopen/save with), line-ending conversion
- Hot exit: quit any time; tabs and unsaved text come back next start (off: asks before closing)
- A menu bar above the tabs (File · Edit · Selection · View · Go · Help) with every command and
  its shortcut — ≡ hides or shows it; go to line, zoom, word wrap, line commands
- Markdown highlighting with coloured code blocks (~200 languages, theme colours), completions (anchors — also of other files, `](other.md#` —, paths, references, emoji, fences, alerts), diagnostics
  (missing headings and files, undefined or unused references, duplicate footnotes, skipped
  heading levels, uneven tables, empty links, images without alt text; each one's severity is a
  setting), go to definition and hover for links and footnotes
- List continuation and Tab/Shift+Tab nesting, formatting toggles, heading levels, links (paste a
  URL over text), tasks, auto-closing brackets; tables: Tab/Shift+Tab between cells (aligns the
  columns), Enter adds a row
- Snippets (table, code block, details, front matter …) from Edit → Insert Snippet or the palette
- Ctrl+. code actions: create a missing linked file, insert/update a table of contents, convert
  a link to a reference link
- F2 renames a heading and the links to it; Shift+F12 finds links to a heading
- Paste or drop images: stored next to the document (`assets/<name>/`) and linked
- Outline sidebar, word count and reading time, HTML export
- Display math (`$$…$$`) drawn in the preview
- Live preview beside the text with scroll sync, open by default — the eye at the right of the tab
  row turns it on and off; find/replace with regex
- Folders: `mdpp <folder>` or File → Open Folder — a folder panel (Git-ignored files left out),
  its files in Ctrl+P, find in files (Ctrl+Shift+F, regex / case / whole word)
- Reloads files changed by other programs; single instance (`mdpp file.md:line:col`)
- Palette: Ctrl+Shift+P runs any command, Ctrl+P opens a file; in it `@` goes to a heading and
  `:` to a line (fuzzy matching)
- Focus mode (F11): full screen, the text alone; portable mode: put a file named `mdpp-portable`
  next to the program and everything stays in `mdpp-data/` beside it
- Settings window (Ctrl+,) like Zed's: search, categories, a described control per setting —
  it writes your `settings.toml`; settings and key bindings are TOML files, applied live

```sh
cargo run --release -p mdpp-app -- README.md
```

Try typing `](#`, `](`, `:roc`, ```` ``` ```` or `> [!` in a document.

| Keys | Action | Keys | Action |
|---|---|---|---|
| Ctrl+N / Ctrl+O | new / open | Ctrl+S / Ctrl+Shift+S | save / save as |
| Ctrl+W / Ctrl+Q | close tab / quit | Ctrl+Tab, Ctrl+PgDn/PgUp | switch tabs |
| Ctrl+F / Ctrl+H | find / replace | F3 / Shift+F3 | next / previous match |
| Ctrl+Shift+V / Ctrl+Shift+O | preview / outline | F12, Ctrl+click | go to link target |
| Ctrl+B / Ctrl+I / Ctrl+E | bold / italic / code | Ctrl+K / Ctrl+Enter | link / toggle task |
| Ctrl+Shift+E | export HTML | Enter / Shift+Enter | continue list / plain line break |
| Tab / Shift+Tab | nest / un-nest list item | Ctrl+Shift+] / [ | heading level +1 / −1 |
| Ctrl+D / Ctrl+Shift+K | duplicate / delete lines | Alt+↑ / Alt+↓ | move lines |
| Ctrl+G | go to line (`12` or `12:5`) | Alt+Z | word wrap |
| Ctrl+= / Ctrl+- / Ctrl+0 | zoom in / out / reset | Ctrl+Alt+S | save all |
| Ctrl+Shift+T | reopen closed tab | right-click a tab | close others / to the right / saved |
| Ctrl+Shift+P | command palette | Ctrl+P | open a file (`@` heading, `:` line) |
| Ctrl+Shift+F | find in files | | |

All bindings are in [`assets/default-keymap.toml`](assets/default-keymap.toml); settings and their
defaults in [`assets/default-settings.toml`](assets/default-settings.toml); the menus in
[`assets/menus.toml`](assets/menus.toml). *Ctrl+,* opens the settings window; *File → Settings File* and
*Keyboard Shortcuts* open your own files, created with every default commented out. *Help → Report a Bug* opens a pre-filled issue.

- [Development plan](docs/PLAN.md) · [Research summary](docs/RESEARCH.md) · [Progress](docs/PROGRESS.md)
- [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md)

Licence: MIT OR Apache-2.0, at your option.
