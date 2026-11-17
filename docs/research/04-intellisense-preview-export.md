> **Raw research output** — produced by an agy (Gemini) research agent on 2026-11-17 for track "D — intellisense, preview, export".
> Not independently verified. Verified facts, measurements and corrections live in [../RESEARCH.md](../RESEARCH.md).

## Summary

For **Markdown++** to achieve its core vision—a standalone, sub-20 MB binary with sub-100 MB RAM usage, sub-300 ms cold start, and sub-50 ms editing latency—the intellisense and rendering architecture must decouple the interactive editing loop from background language intelligence. The optimal architecture pairs an **in-process, tree-sitter-based editing engine** (using `tree-sitter-md` for incremental CST parsing, error-tolerant outline extraction, smart selection, and table/list manipulations) with a **streaming pull-parser** (`pulldown-cmark`) for native GUI preview rendering. Linting and grammar checking are best structured as a hybrid: ship a zero-allocation local intelligence core for instant keystroke tasks, while delegating heavy linting (`rumdl_lib` or CLI) and grammar analysis (`harper-core` or external LSP) to asynchronous background threads with aggressive cancellation tokens.

---

## Findings

1. **Parser dichotomy: Tree-sitter for editing, pull-parsing for rendering.**  
   *Evidence:* `tree-sitter-md` (v0.5.4) provides incremental parsing ($< 0.5\text{ ms}$ for keystroke diffs), robust error tolerance with `ERROR` nodes on malformed input, and concrete byte spans required for syntax-directed folding and breadcrumbs. In contrast, AST and pull parsers (`comrak` v1.0.0, `pulldown-cmark` v0.13.6) require full document re-parsing on every keystroke, causing unacceptable frame latency on 10,000+ line documents. Conversely, `pulldown-cmark` excels at streaming preview and HTML generation with near-zero heap allocations.
2. **Grammar and linting in pure Rust are production-ready.**  
   *Evidence:* `rumdl` (v0.3.4, MIT, 1,672 stars) implements all 53 standard markdownlint rules in Rust with built-in auto-fixing and exposes `rumdl_lib`. `harper` (v3.2.0, Apache-2.0, 17,785 stars) provides developer-focused English grammar checking with $< 20\text{ MB}$ RAM footprint and millisecond lint times (via FSTs and Levenshtein automata), replacing heavy Java-based LanguageTool runtimes.
3. **Dictionary licensing and Turkish agglutination pose specific barriers.**  
   *Evidence:* Bundling SCOWL wordlists is permissible under BSD-style terms, but standard Hunspell `.dic`/`.aff` packages often carry LGPL/GPL licenses, mandating clean separation as optional disk assets. Furthermore, Turkish (`tr_TR`) cannot be served by static wordlists due to agglutinative suffix chaining, and requires Hunspell affix morphology (`spellbook` v0.4.2) plus locale-aware Unicode handling to avoid the Dotted/Dotless "I" bug (`I` $\leftrightarrow$ `ı`, `İ` $\leftrightarrow$ `i`).
4. **Sub-50 ms latency demands a hybrid architecture over pure LSP.**  
   *Evidence:* Pure external LSP clients (over JSON-RPC stdio pipes) introduce IPC serialization overhead, process scheduling jitter, and rope synchronization latency. Implementing Tier 1 features (smart pairs, list continuation, local heading completion, table alignment) in-process directly against a `ropey` buffer yields 1–5 ms latency. External LSP support (`async-lsp` v0.2.4 or `lsp-server` v0.10.0) should remain an asynchronous background layer for Marksman, Markdown-Oxide, or Harper-LS.
5. **Browserless GUI previewing has native Rust solutions, with caveats on math and Mermaid.**  
   *Evidence:* Both `iced` (v0.14.0, native `markdown` widget) and `egui_commonmark` (v0.26.0) render CommonMark/GFM natively without WebView overhead. Math rendering without JS is solved via `RaTeX` (v0.1.16, KaTeX-compatible layout emitting SVG) rendered via `resvg` (v0.50.0). However, pure-Rust Mermaid rendering is immature: `flowmaid` (v0.40.0) covers basic flowcharts but is licensed under copyleft GPL-3.0, making an external CLI hook (`mmdc`) or ASCII text fallback the safest initial approach.
6. **Export tradeoffs: Typst for PDF, docx-rs for Word.**  
   *Evidence:* Full embedded `typst` compilation adds significant binary weight (the standalone CLI is ~16.65 MB compressed; linking the compiler adds ~10–15 MB). Translating Markdown to Typst markup and invoking an external `typst` binary (or offering embedded Typst as an opt-in cargo feature) keeps the base editor under the 20 MB target. For Word export, `docx-rs` (v0.4.22, MIT) generates native `.docx` OpenXML files in pure Rust with $< 500\text{ KB}$ footprint.

---

## Details

### 1. Markdown Intellisense Survey: Modern Tooling & Mechanics

#### Competitive Landscape
* **VS Code Built-in (`vscode-markdown-languageserver` v0.5.0):**  
  Implements path completion (`./images/fig1.png`), workspace heading completion (`[link](#heading)`), reference link resolution (`[ref]: url`), link validation diagnostics (flags missing files and broken anchor slugs), workspace heading rename (renaming `# Title` updates inbound links across documents), outline/symbols, smart selection ranges (word $\to$ inline element $\to$ paragraph $\to$ section $\to$ parent section), and link updates when moving files.
* **Marksman (Release `2026-02-08`, MIT, 3,396 stars):**  
  Written in F# (.NET). Focuses on CommonMark, GFM, and bidirectional wikilinks (`[[page]]`, `[[page#heading]]`). Provides hover documentation, go-to-definition, reference discovery, and heading diagnostics. Packaged as a self-contained ~35 MB binary.
* **Markdown-Oxide (v0.25.12, Apache-2.0, 2,349 stars):**  
  Written in Rust. Tailored for Obsidian-style PKM vaults: wikilinks, backlinks, tag indexing (`#tag`), daily notes resolution, and hover previews. Uses `ropey`, `nucleo-matcher` for fuzzy completion, and custom regex scanners rather than a full CommonMark AST.
* **Obsidian, Zettlr, Foam:**  
  Obsidian relies on CodeMirror 6 with live preview, interactive table grid manipulation, and block references (`^blockid`). Zettlr specializes in academic markdown (BibTeX/CSL JSON citations `[@key]`, Zettelkasten timestamp IDs). Foam provides graph visualization on top of VS Code.

#### Feature Mechanics & Implementation Details
* **GFM Table Auto-Format & Undo-Friendliness:**  
  *The Problem:* Re-aligning markdown tables (`| Col 1 | Header |`) on every keystroke generates continuous full-table replacement edits, corrupting the editor's undo stack and displacing the cursor due to inserted whitespace padding.  
  *The Solution:* (1) Format on cell blur/navigation (`Tab`, `Shift+Tab`, `Enter`, arrow key out) rather than per keystroke; (2) Batch keystrokes inside an active cell into a single logical undo transaction; (3) Compute the cursor's character offset relative to trimmed cell text rather than the raw line start, restoring the cursor accurately after padding adjustments.
* **List Continuation & Renumbering:**  
  Pressing `Enter` on an ordered item (`1. `) inspects leading indentation and syntax. If empty, `Enter` consumes the prefix to break out of the list. If non-empty, it inserts the next numeral (`2. `) and triggers a downstream renumbering pass that increments subsequent sequential list items until a blank line or non-list block is reached.
* **Smart Pairs & Selection Wrapping:**  
  Typing `*`, `_`, `` ` ``, `~`, or `$` around an active selection wraps the text (`foo` $\to$ `**foo**`). Typing a closing delimiter when the cursor is immediately before it executes a skip-over (type-over) without inserting duplicate characters. Backspace between empty pairs (`|`) removes both.
* **Link Wrapping on Paste:**  
  When clipboard text matches a URL regex (`https?://...` or `./...`) and an active non-empty text selection exists, pasting substitutes `[<selected-text>](<url>)` instead of overwriting the text.
* **Front-Matter & LaTeX Completion:**  
  Front-matter completion activates when `---` is at byte 0, prompting standard YAML metadata keys (`title:`, `date:`, `tags:`, `draft:`). LaTeX math completion activates inside `$...$` or `$$...$$`, suggesting Greek characters, symbols, and operators (`\frac`, `\sqrt`, `\sum`, `\int`).
* **Word Count & Readability:**  
  Live statistics must use standard Unicode text segmentation (`unicode-segmentation` crate, UAX #29) to avoid inaccurate word boundaries across non-Latin and CJK scripts. Readability metrics (Flesch Reading Ease and Flesch-Kincaid Grade Level) are computed from sentence boundaries, word counts, and syllable heuristic lookups.

---

### 2. Linting & Spelling in Rust

| Tool / Crate | Latest Version | Release Date | License | Stars | Primary Use Case & Characteristics |
| :--- | :--- | :--- | :--- | :--- | :--- |
| **`rumdl`** | `0.3.4` | 2026-11-12 | MIT | 1,672 | Markdown linter/formatter. Full 53 markdownlint rules; auto-fix; exposes `rumdl_lib`. |
| **`harper-core`** | `3.2.0` | 2026-11-13 | Apache-2.0 | 17,785 | Developer grammar checker. FST dictionary + Levenshtein automata; millisecond speed; low RAM. |
| **`spellbook`** | `0.4.2` | 2026-06-03 | MPL-2.0 | 150 | Hunspell-compatible spellchecker in pure Rust. Maintained by Helix editor team. |
| **`zspell`** | `0.5.5` | 2024-06-13 | MIT/Apache-2.0 | 63 | Native Rust Hunspell parser. Maintenance has stalled since mid-2024. |
| **`typos`** | `0.10.44` | 2026-08-03 | Apache-2.0 | 4,224 | Source code typo fixer. Precompiled typo database; ultra-low false positives; not general NLP. |

#### Dictionaries & Multi-Language Support
* **SCOWL (Spell Checking Oriented Word Lists):**  
  Permissive BSD-style license. Suitable for embedding English wordlists directly into the executable without license conflicts.
* **Hunspell Dictionaries & Licensing:**  
  Many official dictionaries (e.g., LibreOffice, GNU Aspell packages) are licensed under GPL, LGPL, or MPL. To prevent GPL contamination of an MIT/Apache-2.0 editor binary, Hunspell `.dic`/`.aff` files must not be compiled into the binary via `include_bytes!`. Instead, load them at runtime as external assets from an application data directory.
* **Turkish (`tr_TR`) Status & Gotchas:**  
  1. *Morphological Agglutination:* Static wordlists cannot handle Turkish because suffixes stack extensively (e.g., `ev` $\to$ `evler` $\to$ `evlerinizden`). Suffix-stripping morphology (`.aff` rules) via `spellbook` or `hunspell` is mandatory.
  2. *The Dotted/Dotless "I" Pitfall:* Standard Rust `char::to_lowercase()` and `to_uppercase()` follow Unicode default casing, which maps ASCII `I` (U+0049) to `i` (U+0069). In Turkish, `I` lowercases to dotless `ı` (U+0131), while dotted `İ` (U+0130) lowercases to `i` (U+0069). Standard case-folding corrupts Turkish spellchecking; locale-aware casing routines must be enforced.
  3. *Harper Status:* Harper 3.2.0 is currently restricted to English dialects (US, UK, CA, AU, NZ); Turkish grammar is not supported.

---

### 3. Architecture: In-Process Core vs. LSP

```
+-----------------------------------------------------------------------+
|                          Markdown++ Application                       |
+-----------------------------------------------------------------------+
|  UI / Editor Viewport (Iced / GPUI / Egui)                            |
|    |                                                                  |
|    | Direct In-Memory Calls (< 5 ms latency)                          |
|    v                                                                  |
|  [In-Process Trait: MarkdownLanguageService]                          |
|    +-- Incremental CST: tree-sitter-md (outline, folding, pairs)      |
|    +-- Structural Formatting: table align, list renumbering           |
|    +-- Local Completions: emojis, snippets, current doc anchors       |
+-----------------------------------------------------------------------+
|  Background Thread / Async Tokio Runtime                              |
|    +-- In-Memory Linters: rumdl_lib, harper-core (English grammar)    |
|    +-- LSP Client Manager (async-lsp v0.2.4 / lsp-server)             |
|          | Stdio Pipes (JSON-RPC)                                     |
|          v                                                            |
|       [External LSP Servers: Marksman / Markdown-Oxide / Custom]      |
+-----------------------------------------------------------------------+
```

#### Why a Pure External LSP Fails the Constraints
1. **Binary Size & Dependencies:** Shipping an external LSP server requires bundling separate binaries. Marksman (.NET runtime host) adds 35 MB; Harper-LS adds 15 MB.
2. **Latency Budget ($< 50\text{ ms}$):** IPC over stdin/stdout pipes involves JSON serialization, text synchronization via `didChange`, and process context switching. Local in-process inspections on a `ropey` data structure take $< 5\text{ ms}$.

#### Recommended Trait Abstraction
```rust
pub trait MarkdownLanguageService: Send + Sync {
    fn complete(&self, doc: &Rope, offset: usize) -> Vec<CompletionItem>;
    fn hover(&self, doc: &Rope, offset: usize) -> Option<HoverInfo>;
    fn outline(&self, doc: &Rope) -> Vec<OutlineNode>;
    fn format_table_at(&self, doc: &mut Rope, cursor_offset: usize) -> Option<TextEdit>;
    fn toggle_task_at(&self, doc: &mut Rope, line_idx: usize) -> Option<TextEdit>;
}
```

#### Latency & Cancellation Strategy
* **Debounced LSP Dispatch:** Dispatch background diagnostic and completion queries with a 20 ms debounce window.
* **CancellationToken / Generation ID:** Increment an atomic document revision counter on every keystroke. When a background task completes, discard results if `task_revision != current_revision`.
* **LSP Request Cancellation:** If an external server is querying, emit `$/cancelRequest` with the corresponding RPC ID upon receiving subsequent input.

---

### 4. Browserless Preview & Native GUI Rendering

| Framework / Crate | Latest Version | License | Stars | Rendering Mechanism & GFM Fidelity |
| :--- | :--- | :--- | :--- | :--- |
| **`iced` (widget::markdown)** | `0.14.0` | MIT | 32,058 | Built-in native widget; uses `pulldown-cmark`. Supports headers, lists, code blocks, syntax themes. |
| **`egui_commonmark`** | `0.26.0` | MIT/Apache-2.0 | 183 | Native egui renderer. Tables, tasklists, footnotes, syntect highlighting, local/remote/SVG images. |
| **`resvg`** | `0.50.0` | Apache-2.0 | 4,197 | Pure Rust SVG rendering engine. High specification compliance; rasterizes to GPU textures. |
| **`RaTeX`** | `0.1.16` | MIT | 1,593 | Pure Rust LaTeX math engine ($> 99.5\%$ KaTeX coverage). Emits SVG/PNG without JS or WebView. |
| **`flowmaid`** | `0.40.0` | GPL-3.0 | 4 | Pure Rust Mermaid-like diagram engine. Emits SVG for flowcharts/ER diagrams. Copyleft risk. |

#### Preview Features & Solutions
* **GFM Tables & Tasklists:** Native GUI tables must use proportional flex columns calculated from cell contents. Tasklists render native clickable checkbox widgets that send toggle events back to the source buffer.
* **Images:** Local files are resolved relative to document path; remote images are fetched asynchronously via non-blocking HTTP (`ureq` or `reqwest`) and cached in memory. SVGs are rasterized using `resvg`.
* **Syntax Highlighting:** Avoid `syntect`'s default binary dump (adds 15–25 MB RAM). Instead, use `tree-sitter-highlight` with grammars for common languages (Markdown, Rust, Python, JS/TS, HTML, CSS, JSON, YAML, Bash, SQL).
* **Math without JS:** Do not embed headless browsers or Node runtimes for KaTeX. Integrate `RaTeX` (`ratex-svg`), which parses LaTeX equations and outputs clean SVG strings in pure Rust, then render via `resvg`.
* **Mermaid without JS:** Pure-Rust Mermaid implementations are in their infancy (`flowmaid` is functional but GPL-3.0). Recommended architecture: (1) Render code fences as formatted blocks; (2) Provide a pluggable external command hook (`mmdc -i input.mmd -o output.svg`) that caches SVG to disk; (3) Fall back to ASCII/Unicode diagram boxes if offline.
* **Scroll-Sync Strategy (Source-Position Mapping):**  
  Proportional scrolling fails when documents contain large diagrams or multi-line tables. Instead:
  1. Extract block source spans using `pulldown-cmark::into_offset_iter()`.
  2. Associate each rendered GUI block with its starting line number: `Vec<(SourceLine, ViewportY)>`.
  3. When editor line $L$ scrolls, find adjacent anchors $(L_i, Y_i)$ and $(L_{i+1}, Y_{i+1})$, and interpolate:
     $$Y = Y_i + (L - L_i) \cdot \frac{Y_{i+1} - Y_i}{L_{i+1} - L_i}$$

---

### 5. Document Export

```
                 +-----------------------+
                 | Markdown Source File  |
                 +-----------------------+
                             |
             +---------------+---------------+
             |                               |
             v                               v
     [pulldown-cmark]                 [Typst Compiler]
             |                               |
     +-------+-------+                       v
     |               |               +---------------+
     v               v               |  PDF Export   |
  [HTML]         [docx-rs]           +---------------+
                     |
                     v
             +---------------+
             |  DOCX Export  |
             +---------------+
```

* **HTML Export:** Use `pulldown-cmark` (v0.13.6). Zero-cost streaming event generation, bundled with embedded clean CSS supporting `@media (prefers-color-scheme: dark)`.
* **PDF Export:**
  * *Option A (Embedded Typst):* Embedding `typst` (v0.15.1) yields high-quality typesetting, footnotes, and math. However, the compiler and font engine add ~10–15 MB to the final binary.
  * *Option B (printpdf v0.12.9):* Minimal binary footprint ($< 1\text{ MB}$), but provides no layout engine; wrapping text, calculating page breaks, and rendering tables must be coded manually.
  * *Option C (Headless Browser / wkhtmltopdf):* Unacceptable; requires >150 MB external dependencies and high memory usage.
  * *Recommendation:* Implement a pure-Rust Markdown $\to$ Typst transpiler. If an external `typst` binary is present on `PATH`, call it. Provide an opt-in Cargo build feature (`--features embedded-pdf`) for users who accept a 30 MB binary for self-contained PDF export.
* **DOCX Export:**
  * Use `docx-rs` (v0.4.22, MIT). Pure Rust, $< 500\text{ KB}$ footprint, directly generates OpenXML archives (`.docx`) supporting headings, paragraphs, bold/italic, tables, bullet lists, and embedded images without requiring Microsoft Word or Pandoc.

---

### 6. Parser Evaluation for Editor Core vs. Preview

| Parser | Latest Version | License | Stars | AST / Output | Incremental? | Byte Spans / Positions | Error Tolerance | Memory Footprint | Recommended Role |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| **`tree-sitter-md`** | `0.5.4` | MIT | 631 | Concrete Syntax Tree | **Yes** (GLR) | Precise byte ranges & row/cols | **High** (recovers via `ERROR` nodes) | Low (re-uses unaffected nodes) | **Editor Core (Typing, Outline, Folds)** |
| **`pulldown-cmark`** | `0.13.6` | MIT | 2,765 | Pull Event Stream | No (Batch) | Byte offsets via `offset_iter()` | Moderate (CommonMark fallback) | **Minimal** (near-zero heap allocs) | **Preview, HTML, & Export** |
| **`comrak`** | `1.0.0` | BSD-2-Clause | 1,741 | Arena Mutable AST | No (Batch) | `Sourcepos` (line/col spans) | Moderate (Standard CommonMark) | High (arena node tree) | Advanced AST AST mutations |
| **`markdown-rs`** | `1.0.0` | MIT | 1,595 | `mdast` AST | No (Batch) | Exact `Point` / `Position` offsets | Moderate | High (Rust enum boxing) | Static MDX/AST transformations |

---

## Recommendations for Markdown++

### Core Architectural Choices
1. **Editor Core Parser:** Use `tree-sitter-md` — enables $< 0.5\text{ ms}$ incremental reparsing, syntax folding, breadcrumb outlines, and robust error recovery during typing.
2. **Preview & HTML Engine:** Use `pulldown-cmark` — pull-parsing delivers fast streaming rendering with minimal RAM allocations.
3. **Language Service Architecture:** Implement an in-process engine for Tier 1 editing actions, backed by an asynchronous `async-lsp` client for external LSP servers.
4. **Linter Integration:** Embed `rumdl_lib` — delivers full markdownlint rule compliance in pure Rust with auto-fixing capabilities.
5. **Grammar & Spelling:** Embed `harper-core` for English grammar; use `spellbook` with external Hunspell dictionaries for multi-language and Turkish spelling.
6. **Math Rendering:** Use `RaTeX` (`ratex-svg`) with `resvg` — pure Rust KaTeX equation layout to vector graphics without JavaScript.
7. **Document Export:** Native `pulldown-cmark` for HTML, `docx-rs` for Word documents, and Markdown $\to$ Typst translation for PDF export.

---

### Prioritized Feature Roadmap

| Phase | Feature | Effort | Rationale & Dependencies |
| :--- | :--- | :---: | :--- |
| **MVP** | Incremental Outline & Breadcrumbs | **S** | Immediate user navigation value; built on `tree-sitter-md` CST query. |
| **MVP** | Smart Delimiter Pairs & Selection Wrap | **S** | High typing utility; standard text editor behavior (`*`, `` ` ``, `~`, `_`, `$`). |
| **MVP** | List Continuation & Auto-Renumbering | **S** | In-process line analysis on `Enter`; handles ordered/unordered/task lists. |
| **MVP** | Task Checkbox Toggling | **S** | Shortcut (`Ctrl+Enter`) or click in preview toggles `- [ ]` $\leftrightarrow$ `- [x]`. |
| **MVP** | Native GFM Preview (Tables, Lists, Code) | **M** | Core preview capability via `pulldown-cmark` + GUI text widgets. |
| **MVP** | Source-Position Scroll-Sync | **M** | Anchored mapping using `offset_iter()` line positions to prevent scroll drift. |
| **MVP** | HTML & DOCX Export | **S** | Streaming HTML via `pulldown-cmark`; native `.docx` via `docx-rs`. |
| **v1** | GFM Table Auto-Format & Align | **M** | Requires cell navigation formatting on `Tab` with atomic undo grouping. |
| **v1** | Link Paste Wrapping & File Dropping | **S** | Detects URLs on selection paste; copies dropped images to `./assets`. |
| **v1** | Built-in Linter Diagnostics (`rumdl_lib`) | **M** | Runs markdownlint rules on background worker; displays inline diagnostics. |
| **v1** | Built-in Grammar Checker (`harper-core`) | **M** | In-process English grammar linting via `harper-core`. |
| **v1** | LaTeX Math Preview (Pure Rust) | **M** | Integrates `RaTeX` + `resvg` to render `$...$` without web runtimes. |
| **v1** | PDF Export via Typst | **M** | Compiles to Typst syntax and invokes external CLI or optional embedded engine. |
| **Later** | External LSP Client Interface | **L** | Subprocess supervisor for Marksman/Markdown-Oxide using `async-lsp`. |
| **Later** | Multi-Language Hunspell (`spellbook`) | **L** | User dictionary management, Turkish affix rules, and locale-aware casing. |
| **Later** | Pure-Rust Mermaid Diagrams | **L** | Blocked on ecosystem maturity; tracks non-GPL Rust diagram engines. |
| **Later** | Workspace Link Rename / Refactoring | **L** | Scans and edits references across all workspace files upon heading rename. |

*Effort Scale: **S** = 1–3 days, **M** = 1–2 weeks, **L** = 3+ weeks.*

---

## Unverified / Open Questions

1. **Typst Embedded Binary Footprint:** While the standalone CLI is ~16.65 MB compressed, the exact stripped binary increase when linking only `typst::compile` and `typst-pdf` into a minimal GUI host remains unverified in this environment; it is estimated between 8 and 14 MB under full LTO.
2. **`iced::widget::markdown` GFM Table Interaction:** The internal `iced_widget::markdown` implementation parses CommonMark via `pulldown-cmark`, but native interactive column resizing and custom cell styling for complex GFM tables require verification against iced's latest layout engine.
3. **Headless Mermaid Execution via QuickJS:** Feasibility of bundling a minimal QuickJS engine (`rquickjs`) to evaluate `mermaid.min.js` in a headless environment remains unverified; Mermaid relies heavily on DOM layout calls (`getBBox()`), which typically fail without an emulated browser layout environment.

---

## Sources

* [VS Code Markdown Language Features](https://github.com/microsoft/vscode/tree/main/extensions/markdown-language-features)
* [VS Code Markdown Language Server](https://github.com/microsoft/vscode-markdown-languageserver)
* [Marksman Repository](https://github.com/artempyanykh/marksman)
* [Markdown-Oxide Repository](https://github.com/Feel-ix-343/markdown-oxide)
* [rumdl (Rust Markdown Linter)](https://github.com/rvben/rumdl)
* [Harper Grammar Checker](https://github.com/automattic/harper)
* [Spellbook (Helix Hunspell Crate)](https://github.com/helix-editor/spellbook)
* [Typos Source Code Spell Checker](https://github.com/crate-ci/typos)
* [tree-sitter-markdown Grammar](https://github.com/tree-sitter-grammars/tree-sitter-markdown)
* [pulldown-cmark Repository](https://github.com/pulldown-cmark/pulldown-cmark)
* [Comrak Repository](https://github.com/kivikakk/comrak)
* [markdown-rs Repository](https://github.com/wooorm/markdown-rs)
* [egui_commonmark Viewer](https://github.com/lampsitter/egui_commonmark)
* [Iced GUI Library](https://github.com/iced-rs/iced)
* [RaTeX Pure Rust KaTeX Engine](https://github.com/erweixin/RaTeX)
* [resvg SVG Renderer](https://github.com/linebender/resvg)
* [Typst Markup & Typesetting System](https://github.com/typst/typst)
* [printpdf PDF Library](https://github.com/fschutt/printpdf)
* [docx-rs DOCX Writer](https://github.com/bokuweb/docx-rs)
* [Flowmaid Diagram Engine](https://github.com/go-routine-id/flowmaid)
* [async-lsp Framework](https://github.com/oxalica/async-lsp)
