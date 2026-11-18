//! One open document (one tab): its editor, preview, background analysis and file state.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use anyhow::Context as _;
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::highlighter::{Diagnostic, DiagnosticSeverity};
use gpui_kit::component::input::{EditorState, InputEvent, Position, Rope, RopeExt};
use gpui_kit::component::notification::Notification;
use gpui_kit::*;
use mdpp_core::session::{FileStamp, SessionDocument, Store};
use mdpp_core::settings::Level;
use mdpp_core::settings::Settings;
use mdpp_core::text::{self, Encoding, Eol, Fingerprint, UTF_8};
use mdpp_lang::FileSystem;
use mdpp_markdown::code::CodeColors;
use mdpp_markdown::{DocIndex, blocks};

use crate::completion::{MdCompletions, SharedIndex};
use crate::highlight::{self, SharedCodeColors, SharedLexer};
use crate::navigation::MdNavigation;
use crate::preview::Preview;

/// Name suggested when saving an untitled document.
const UNTITLED_FILE_NAME: &str = "Untitled.md";

/// The document's path, shared with the completion and navigation providers (Save As changes it).
pub type SharedPath = Rc<RefCell<Option<PathBuf>>>;

/// File contents decoded for the editor.
pub struct OpenedDoc {
    pub path: Option<PathBuf>,
    pub text: String,
    pub eol: Eol,
    pub bom: bool,
    pub mixed_eol: bool,
    pub encoding: &'static Encoding,
    /// Some bytes were invalid in `encoding` and were replaced (suggest "Reopen with").
    pub had_errors: bool,
    /// The saved state when it differs from `text` (a restored backup of unsaved changes).
    pub saved: Option<Fingerprint>,
    /// Backup this document's unsaved text lives in (restored sessions).
    pub backup: Option<String>,
    /// Cursor to restore (line, column).
    pub cursor: Option<(u32, u32)>,
}

impl OpenedDoc {
    /// Load `path`; a path that does not exist yet opens as an empty document with that name.
    /// Paths are made absolute, so they match file-change events and session entries.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let absolute = std::path::absolute(path);
        let path = absolute.as_deref().unwrap_or(path);
        if !path.exists() {
            return Ok(Self::untitled(Some(path.to_path_buf())));
        }
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        let loaded = text::decode(&bytes, Eol::native())
            .with_context(|| format!("opening {}", path.display()))?;
        Ok(Self {
            path: Some(path.to_path_buf()),
            text: loaded.text,
            eol: loaded.eol,
            bom: loaded.bom,
            mixed_eol: loaded.mixed_eol,
            encoding: loaded.encoding,
            had_errors: loaded.had_errors,
            saved: None,
            backup: None,
            cursor: None,
        })
    }

    pub fn untitled(path: Option<PathBuf>) -> Self {
        Self {
            path,
            text: String::new(),
            eol: Eol::native(),
            bom: false,
            mixed_eol: false,
            encoding: UTF_8,
            had_errors: false,
            saved: None,
            backup: None,
            cursor: None,
        }
    }

    /// Rebuild a document from its session entry: unsaved text from the backup when there is one,
    /// else the file. Returns a warning when the file changed on disk while MD++ was closed.
    pub fn restore(
        entry: &SessionDocument,
        store: &Store,
    ) -> anyhow::Result<(Self, Option<String>)> {
        let cursor = Some((entry.cursor_line, entry.cursor_column));
        let Some(text) = entry
            .backup
            .as_deref()
            .and_then(|name| store.read_backup(name).ok())
        else {
            let path = entry
                .path
                .as_deref()
                .context("untitled tab without a backup")?;
            anyhow::ensure!(path.exists(), "{} no longer exists", path.display());
            return Ok((
                Self {
                    cursor,
                    ..Self::load(path)?
                },
                None,
            ));
        };
        // What is on disk now is the "saved" state the restored text is compared against.
        let encoding = text::encoding_for_label(&entry.encoding).unwrap_or(UTF_8);
        let disk = entry.path.as_deref().and_then(|p| std::fs::read(p).ok());
        let disk_text = disk.map(|bytes| text::decode_as(&bytes, encoding, entry.eol).text);
        let saved = Fingerprint::of_chunks([disk_text.as_deref().unwrap_or("")]);
        let changed = entry
            .path
            .as_deref()
            .is_some_and(|p| FileStamp::of(p) != entry.disk);
        let warning = changed.then(|| {
            format!(
                "{} changed on disk while MD++ was closed. Your unsaved version was restored: save to keep it, or close without saving to use the version on disk.",
                file_label(entry.path.as_deref())
            )
        });
        let doc = Self {
            path: entry.path.clone(),
            text,
            eol: entry.eol,
            bom: entry.bom,
            mixed_eol: false,
            encoding,
            had_errors: false,
            saved: Some(saved),
            backup: entry.backup.clone(),
            cursor,
        };
        Ok((doc, warning))
    }
}

fn file_label(path: Option<&Path>) -> String {
    path.and_then(Path::file_name).map_or_else(
        || "Untitled".to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Disk work the session store has to do for one document.
pub struct Persist {
    pub entry: SessionDocument,
    /// Unsaved text changed since the last backup: write it to this backup.
    pub write: Option<(String, Rope)>,
    /// The document is clean again: this backup is obsolete.
    pub remove: Option<String>,
}

/// Something the workspace shows (tab label, window title, status bar) changed.
pub struct DocumentChanged;

/// Saved under a new path (Save As, or a first save): its folder needs watching.
pub struct Renamed(pub PathBuf);

/// The file changed on disk in a way that needs the user (PLAN §3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskConflict {
    /// Changed on disk while this tab has unsaved changes.
    Changed,
    /// Deleted (or moved away) on disk; the tab keeps its text.
    Deleted,
}

pub struct Document {
    editor: Entity<EditorState>,
    /// The highlighter's per-line block state, also used by editing assists and completion.
    lexer: SharedLexer,
    /// Colours of the fenced code blocks, shared with the highlighter and the preview.
    code_colors: SharedCodeColors,
    preview: Entity<Preview>,
    preview_visible: bool,
    /// Editor line the preview was last synced to.
    synced_line: Option<usize>,
    path: Option<PathBuf>,
    /// Mirror of `path` for the providers; updated together with it in `saved_as`.
    shared_path: SharedPath,
    eol: Eol,
    bom: bool,
    mixed_eol: bool,
    encoding: &'static Encoding,
    /// The text as last loaded or saved; dirty = the editor no longer matches it.
    saved: Fingerprint,
    /// Line endings were converted: the file differs from the disk even with the same text.
    format_changed: bool,
    dirty: bool,
    soft_wrap: bool,
    /// The file on disk when it was loaded or saved (recorded in the session).
    disk_stamp: Option<FileStamp>,
    conflict: Option<DiskConflict>,
    /// Backup holding the unsaved text, and the revision it holds.
    backup: Option<String>,
    backed_up_revision: Option<u64>,
    settings: Rc<Settings>,
    /// Bumped on every edit; background results computed for an older revision are dropped.
    revision: u64,
    diagnostic_count: usize,
    word_count: usize,
    index: SharedIndex,
    _analysis: Task<()>,
    _coloring: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DocumentChanged> for Document {}
impl EventEmitter<Renamed> for Document {}

impl Document {
    pub fn new(
        doc: OpenedDoc,
        settings: Rc<Settings>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let shared_path: SharedPath = Rc::new(RefCell::new(doc.path.clone()));
        let saved = doc
            .saved
            .unwrap_or_else(|| Fingerprint::of_chunks([doc.text.as_str()]));
        let dirty = !saved.matches(doc.text.len(), [doc.text.as_str()]);
        let disk_stamp = doc.path.as_deref().and_then(FileStamp::of);
        let lexer = SharedLexer::default();
        let code_colors = SharedCodeColors::default();
        let index = SharedIndex::default();
        let soft_wrap = settings.soft_wrap && doc.text.len() <= settings.soft_wrap_max_bytes;

        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(highlight::LANGUAGE)
                .line_number(settings.line_numbers)
                .soft_wrap(soft_wrap)
                .default_value(doc.text)
        });
        editor.update(cx, |state, cx| {
            if let Some((line, column)) = doc.cursor {
                state.set_cursor_position(Position::new(line, column), window, cx);
            }
            state.set_highlighter_factory(
                highlight::factory(lexer.clone(), code_colors.clone(), settings.markup_fade),
                cx,
            );
            state.lsp_mut().completion_provider = Some(Rc::new(MdCompletions::new(
                lexer.clone(),
                index.clone(),
                shared_path.clone(),
            )));
            state.lsp_mut().code_action_providers.push(Rc::new(
                crate::code_actions::MdCodeActions::new(shared_path.clone()),
            ));
            let navigation = Rc::new(MdNavigation::new(shared_path.clone()));
            state.lsp_mut().definition_provider = Some(navigation.clone());
            state.lsp_mut().hover_provider = Some(navigation);
        });
        let preview = cx.new(|_| {
            Preview::new(
                settings.preview_max_width,
                settings.preview_remote_images,
                code_colors.clone(),
            )
        });

        let subscriptions = vec![
            cx.subscribe_in(
                &editor,
                window,
                |this, _, event: &InputEvent, _window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.revision += 1;
                        this.refresh_dirty(cx);
                        this.schedule_analysis(cx);
                    }
                },
            ),
            // Cursor moves and scrolling emit no `Change`: follow every editor update.
            cx.observe(&editor, |this, _, cx| {
                this.sync_preview(cx);
                cx.emit(DocumentChanged);
            }),
        ];

        let mut this = Self {
            editor,
            lexer,
            code_colors,
            preview,
            preview_visible: false,
            synced_line: None,
            path: doc.path,
            shared_path,
            eol: doc.eol,
            bom: doc.bom,
            mixed_eol: doc.mixed_eol,
            encoding: doc.encoding,
            saved,
            format_changed: false,
            dirty,
            soft_wrap,
            disk_stamp,
            conflict: None,
            backup: doc.backup,
            backed_up_revision: None,
            settings,
            revision: 0,
            diagnostic_count: 0,
            word_count: 0,
            index,
            _analysis: Task::ready(()),
            _coloring: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.schedule_analysis(cx);
        this
    }

    pub fn editor(&self) -> &Entity<EditorState> {
        &self.editor
    }

    pub fn preview(&self) -> &Entity<Preview> {
        &self.preview
    }

    /// The latest colours of the fenced code blocks.
    pub fn code_colors(&self) -> Arc<CodeColors> {
        self.code_colors.borrow().clone()
    }

    /// Whether the cursor is in ordinary Markdown (not a code block, front matter, comment or math).
    pub fn cursor_in_prose(&self, cx: &App) -> bool {
        let state = self.editor.read(cx);
        let offset = state.cursor().min(state.text().len());
        let row = state.text().byte_to_line_idx(offset, ropey::LineType::LF);
        self.lexer.borrow().state_at(row) == mdpp_markdown::BlockState::Normal
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// The document's folder; `None` while it is unsaved.
    fn doc_dir(&self) -> Option<PathBuf> {
        self.path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn diagnostic_count(&self) -> usize {
        self.diagnostic_count
    }

    pub fn word_count(&self) -> usize {
        self.word_count
    }

    /// The latest structure index (headings for the outline); `None` before the first analysis.
    pub fn index(&self) -> Option<Arc<DocIndex>> {
        self.index.borrow().clone()
    }

    /// File name for tabs and the title bar.
    pub fn name(&self) -> String {
        self.path.as_deref().and_then(Path::file_name).map_or_else(
            || "Untitled".to_string(),
            |n| n.to_string_lossy().into_owned(),
        )
    }

    pub fn conflict(&self) -> Option<DiskConflict> {
        self.conflict
    }

    pub fn set_settings(&mut self, settings: Rc<Settings>) {
        self.settings = settings;
    }

    /// The file may have changed on disk. Our own saves and touches that change nothing are
    /// recognized by the file stamp; a clean tab reloads silently, a dirty one gets a conflict.
    pub fn check_disk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let stamp = FileStamp::of(&path);
        if stamp == self.disk_stamp {
            return;
        }
        match stamp {
            None => {
                self.conflict = Some(DiskConflict::Deleted);
                // Unsaved now: the text exists only here (hot exit keeps it).
                self.saved = Fingerprint::of_chunks([""]);
                self.disk_stamp = None;
                self.refresh_dirty(cx);
            }
            Some(_) if self.dirty => self.conflict = Some(DiskConflict::Changed),
            Some(_) => self.reload(window, cx),
        }
        cx.emit(DocumentChanged);
    }

    /// Replace the text with the file on disk (one undo step; the cursor stays where it was).
    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.load_from_disk(None, window, cx);
    }

    /// Ask where to save, then export the document as self-contained HTML (PLAN §4.2).
    pub fn export_html(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self
            .doc_dir()
            .or_else(std::env::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let stem = self.path.as_deref().and_then(Path::file_stem).map_or_else(
            || "Untitled".to_string(),
            |s| s.to_string_lossy().into_owned(),
        );
        let chosen = cx.prompt_for_new_path(&dir, Some(&format!("{stem}.html")));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(path))) = chosen.await
                && let Ok(task) =
                    this.update_in(cx, |this, window, cx| this.export_html_to(path, window, cx))
            {
                task.await;
            }
        })
        .detach();
    }

    /// Export to `path` on a background thread; resolves to whether it worked.
    pub fn export_html_to(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let text = self.editor.read(cx).text().clone();
        let title = self.name();
        let job = cx.background_spawn(async move {
            let html = mdpp_markdown::export::to_html(&text.to_string(), &title);
            mdpp_core::file::save(&path, html.as_bytes()).map(|_| path)
        });
        cx.spawn_in(window, async move |_, cx| {
            let result = job.await;
            cx.update(|window, cx| match result {
                Ok(path) => {
                    window.push_notification(
                        Notification::success(format!("Exported to {}", path.display())),
                        cx,
                    );
                    true
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(format!("Could not export: {error}")),
                        cx,
                    );
                    false
                }
            })
            .unwrap_or(false)
        })
    }

    /// "Reopen with": decode the file on disk with `encoding` instead of the detected one.
    pub fn reopen_with(
        &mut self,
        encoding: &'static Encoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.load_from_disk(Some(encoding), window, cx);
    }

    /// "Save with": write the file in `encoding` from now on. Restored if the save fails (for
    /// example because a character cannot be represented).
    pub fn save_with(
        &mut self,
        encoding: &'static Encoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous = (self.encoding, self.bom);
        self.encoding = encoding;
        self.bom = text::wants_bom(encoding) || (encoding == UTF_8 && self.bom);
        let saving = self.save(window, cx);
        cx.spawn(async move |this, cx| {
            if !saving.await {
                this.update(cx, |this, cx| {
                    (this.encoding, this.bom) = previous;
                    cx.emit(DocumentChanged);
                })
                .ok();
            }
        })
        .detach();
        cx.emit(DocumentChanged);
    }

    fn load_from_disk(
        &mut self,
        encoding: Option<&'static Encoding>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.path.clone() else {
            return;
        };
        let opened = match encoding {
            None => OpenedDoc::load(&path),
            Some(encoding) => std::fs::read(&path)
                .map(|bytes| text::decode_as(&bytes, encoding, self.eol))
                .map(|loaded| OpenedDoc {
                    text: loaded.text,
                    eol: loaded.eol,
                    bom: loaded.bom,
                    mixed_eol: loaded.mixed_eol,
                    encoding: loaded.encoding,
                    had_errors: loaded.had_errors,
                    ..OpenedDoc::untitled(Some(path.clone()))
                })
                .map_err(anyhow::Error::from),
        };
        match opened {
            Ok(opened) => {
                self.encoding = opened.encoding;
                self.eol = opened.eol;
                self.bom = opened.bom;
                self.mixed_eol = opened.mixed_eol;
                self.saved = Fingerprint::of_chunks([opened.text.as_str()]);
                self.format_changed = false;
                self.disk_stamp = FileStamp::of(&path);
                self.conflict = None;
                self.editor.update(cx, |state, cx| {
                    let cursor = state.cursor_position();
                    state.replace_all(opened.text, window, cx);
                    state.set_cursor_position(cursor, window, cx);
                });
                self.refresh_dirty(cx);
            }
            Err(error) => window.push_notification(
                Notification::error(format!("Could not reload {}: {error:#}", self.name())),
                cx,
            ),
        }
        cx.emit(DocumentChanged);
    }

    /// Keep the text in the editor; the change on disk is acknowledged.
    pub fn keep_mine(&mut self, cx: &mut Context<Self>) {
        self.disk_stamp = self.path.as_deref().and_then(FileStamp::of);
        self.conflict = None;
        cx.emit(DocumentChanged);
    }

    fn refresh_dirty(&mut self, cx: &mut Context<Self>) {
        let text = self.editor.read(cx).text();
        let dirty = self.format_changed || !self.saved.matches(text.len(), text.chunks());
        if dirty != self.dirty {
            self.dirty = dirty;
            cx.emit(DocumentChanged);
        }
    }

    /// Session entry for this document, plus the backup work it needs (PLAN §3.4).
    pub fn persist(&mut self, cx: &App) -> Persist {
        let state = self.editor.read(cx);
        let cursor = state.cursor_position();
        let (mut write, mut remove) = (None, None);
        if self.dirty {
            let name = self
                .backup
                .get_or_insert_with(Store::new_backup_name)
                .clone();
            if self.backed_up_revision != Some(self.revision) {
                self.backed_up_revision = Some(self.revision);
                write = Some((name, state.text().clone()));
            }
        } else {
            remove = self.backup.take();
            self.backed_up_revision = None;
        }
        let entry = SessionDocument {
            path: self.path.clone(),
            backup: self.backup.clone(),
            cursor_line: cursor.line,
            cursor_column: cursor.character,
            eol: self.eol,
            bom: self.bom,
            encoding: self.encoding.name().to_string(),
            disk: self.disk_stamp,
        };
        Persist {
            entry,
            write,
            remove,
        }
    }

    /// The tab is being closed without saving: its backup must not come back.
    pub fn discard_backup(&mut self) -> Option<String> {
        self.backed_up_revision = None;
        self.backup.take()
    }

    pub fn eol(&self) -> Eol {
        self.eol
    }

    /// Encoding and line-ending labels for the status bar.
    pub fn format_labels(&self) -> (String, String) {
        let bom = if self.bom { " BOM" } else { "" };
        let encoding = format!("{}{bom}", self.encoding.name());
        let eol = if self.mixed_eol {
            format!("{} (mixed)", self.eol)
        } else {
            self.eol.to_string()
        };
        (encoding, eol)
    }

    /// Convert the line endings; written on the next save (the tab shows unsaved changes).
    pub fn set_eol(&mut self, eol: Eol, cx: &mut Context<Self>) {
        if eol == self.eol && !self.mixed_eol {
            return;
        }
        self.eol = eol;
        self.mixed_eol = false;
        self.format_changed = true;
        self.refresh_dirty(cx);
        cx.emit(DocumentChanged);
    }

    pub fn soft_wrap(&self) -> bool {
        self.soft_wrap
    }

    pub fn set_soft_wrap(&mut self, wrap: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.soft_wrap = wrap;
        self.editor
            .update(cx, |state, cx| state.set_soft_wrap(wrap, window, cx));
    }

    pub fn set_preview_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        self.preview_visible = visible;
        self.synced_line = None;
        if visible {
            self.schedule_analysis(cx);
        }
    }

    /// Save to the document's path (asks for one if it has none). Resolves to whether it saved.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        match self.path.clone() {
            Some(path) => self.write(path, window, cx),
            None => self.save_as(window, cx),
        }
    }

    /// Ask for a path and save there. Resolves to whether it saved.
    pub fn save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let dir = self
            .doc_dir()
            .or_else(std::env::home_dir)
            .unwrap_or_else(|| PathBuf::from("."));
        let name = self.path.as_deref().and_then(Path::file_name).map_or_else(
            || UNTITLED_FILE_NAME.to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let chosen = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = chosen.await else {
                return false;
            };
            match this.update_in(cx, |this, window, cx| this.write(path, window, cx)) {
                Ok(written) => written.await,
                Err(_) => false,
            }
        })
    }

    /// Encode and write on a background thread (PLAN §3.3); the editor stays responsive.
    fn write(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Task<bool> {
        let text = self.editor.read(cx).text().clone();
        let (eol, bom, encoding) = (self.eol, self.bom, self.encoding);
        let job = cx.background_spawn(async move {
            // Refuse to write characters the encoding cannot hold rather than lose them.
            let bytes = text::encode(&text.to_string(), eol, bom, encoding)
                .map_err(std::io::Error::other)?;
            mdpp_core::file::save(&path, &bytes).map(|_| (path, text))
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok((path, text)) => {
                    this.saved_as(path, &text, cx);
                    true
                }
                Err(error) => {
                    window.push_notification(
                        Notification::error(format!("Could not save: {error}")),
                        cx,
                    );
                    false
                }
            })
            .unwrap_or(false)
        })
    }

    fn saved_as(&mut self, path: PathBuf, text: &Rope, cx: &mut Context<Self>) {
        if self.path.as_ref() != Some(&path) {
            cx.emit(Renamed(path.clone()));
        }
        *self.shared_path.borrow_mut() = Some(path.clone());
        self.path = Some(path);
        self.saved = Fingerprint::of_chunks(text.chunks());
        self.disk_stamp = self.path.as_deref().and_then(FileStamp::of);
        self.conflict = None;
        self.mixed_eol = false; // written with one line-ending style
        self.format_changed = false;
        let current = self.editor.read(cx).text();
        self.dirty = !self.saved.matches(current.len(), current.chunks());
        cx.emit(DocumentChanged);
        self.schedule_analysis(cx); // relative links resolve against the (new) folder
    }

    /// Colour the fenced code blocks on a snapshot after the debounce, reusing the blocks that did
    /// not change. Separate from the analysis so a document full of code does not hold up the
    /// outline and diagnostics.
    fn schedule_coloring(&mut self, cx: &mut Context<Self>) {
        let delay = self.settings.structure_debounce;
        let max_lines = self.settings.code_highlight_max_lines;
        let text = self.editor.read(cx).text().clone();
        let previous = self.code_colors.borrow().clone();
        self._coloring = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let colors = cx
                .background_spawn(async move { CodeColors::build(&text, &previous, max_lines) })
                .await;
            this.update(cx, |this, cx| {
                *this.code_colors.borrow_mut() = Arc::new(colors);
                this.editor.update(cx, |_, cx| cx.notify());
                this.preview.update(cx, |_, cx| cx.notify());
            })
            .ok();
        });
    }

    /// Rebuild the index, diagnostics and (when visible) the preview on a snapshot after the
    /// debounce (PLAN §3.2).
    fn schedule_analysis(&mut self, cx: &mut Context<Self>) {
        self.schedule_coloring(cx);
        let revision = self.revision;
        let delay = self.settings.structure_debounce;
        let doc_dir = self.doc_dir();
        let text = self.editor.read(cx).text().clone();
        let with_preview = self.preview_visible;
        // Replacing the task cancels the previous one: only the last edit of a burst does work.
        self._analysis = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let (text, index, found, blocks, doc_dir, words) = cx
                .background_spawn(async move {
                    let source = text.to_string();
                    let words = mdpp_markdown::word_count(&source);
                    let index = DocIndex::build(&source);
                    let found =
                        mdpp_lang::diagnose(&source, &index, doc_dir.as_deref(), &FileSystem);
                    let blocks = with_preview.then(|| Arc::new(blocks::parse(&source)));
                    (text, Arc::new(index), found, blocks, doc_dir, words)
                })
                .await;
            this.update(cx, |this, cx| {
                // Completion takes only names from the index and the preview may trail by one
                // edit; diagnostics carry positions and must match the current text.
                *this.index.borrow_mut() = Some(index);
                this.word_count = words;
                cx.emit(DocumentChanged);
                if let Some(blocks) = blocks {
                    this.preview
                        .update(cx, |preview, cx| preview.set_document(blocks, doc_dir, cx));
                    this.synced_line = None;
                    this.sync_preview(cx);
                }
                if this.revision == revision {
                    this.apply_diagnostics(text, found, cx);
                }
            })
            .ok();
        });
    }

    fn apply_diagnostics(
        &mut self,
        text: Rope,
        found: Vec<mdpp_lang::Diagnostic>,
        cx: &mut Context<Self>,
    ) {
        // How each rule is shown comes from the settings; "off" rules are dropped.
        let levels = &self.settings.diagnostics;
        let shown: Vec<_> = found
            .into_iter()
            .filter_map(|d| {
                let severity = match levels.get(d.code).copied().unwrap_or(Level::Warning) {
                    Level::Off => return None,
                    Level::Hint => DiagnosticSeverity::Hint,
                    Level::Info => DiagnosticSeverity::Info,
                    Level::Warning => DiagnosticSeverity::Warning,
                    Level::Error => DiagnosticSeverity::Error,
                };
                Some((d, severity))
            })
            .collect();
        self.diagnostic_count = shown.len();
        self.editor.update(cx, |state, cx| {
            if let Some(set) = state.diagnostics_mut() {
                set.reset(&text);
                set.extend(shown.into_iter().map(|(d, severity)| {
                    let range = text.offset_to_position(d.range.start)
                        ..text.offset_to_position(d.range.end);
                    Diagnostic::new(range, d.message).with_severity(severity)
                }));
            }
            cx.notify();
        });
        cx.emit(DocumentChanged);
    }

    /// Scroll the preview to the block shown at the top of the editor (PLAN §4.2).
    fn sync_preview(&mut self, cx: &mut Context<Self>) {
        if !self.preview_visible {
            return;
        }
        let state = self.editor.read(cx);
        let Some(top) = state.visible_row_range().map(|rows| rows.start) else {
            return;
        };
        if self.synced_line == Some(top) {
            return;
        }
        let text = state.text();
        let line = top.min(text.len_lines(ropey::LineType::LF).saturating_sub(1));
        let byte = text.line_to_byte_idx(line, ropey::LineType::LF);
        self.synced_line = Some(top);
        self.preview
            .update(cx, |preview, cx| preview.reveal_source(byte, cx));
    }
}

#[cfg(test)]
mod tests {
    use mdpp_core::settings::Settings;

    #[test]
    fn every_diagnostic_rule_has_a_default_level() {
        let levels = Settings::default().diagnostics;
        for rule in mdpp_lang::RULES {
            assert!(
                levels.contains_key(*rule),
                "{rule} missing in [diagnostics]"
            );
        }
        assert_eq!(levels.len(), mdpp_lang::RULES.len());
    }
}
