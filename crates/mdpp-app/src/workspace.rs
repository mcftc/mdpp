//! The window's root view: tabs of documents, the active editor (and preview), the status bar.
//!
//! Editing commands live in `editing`, tab and file commands in `tabs`, measurements and
//! end-to-end checks in `checks`.

mod checks;
mod editing;
mod folder;
mod palette;
mod tabs;

use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Editor, Position, RopeExt as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::status_bar::StatusBar;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme, IconName, Sizable as _, Theme, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use mdpp_core::cli::OpenRequest;
use mdpp_core::instance::Server;
use mdpp_core::keymap::{Binding, DEFAULT_CONTEXT};
use mdpp_core::paths::Paths;
use mdpp_core::session::{SESSION_VERSION, Session, Store};
use mdpp_core::settings::Settings;

use crate::bench::{self, SharedProbe};
use crate::document::{DiskConflict, Document, DocumentChanged, OpenedDoc, Persist, Renamed};
use crate::find::{CloseRequested, FindBar};
use crate::prompt::{Ask, PromptBar};
use crate::watch::{DiskWatcher, folder_of};
use folder::Side;
use gpui_kit::component::menu::DropdownMenu as _;
use mdpp_core::text::{Eol, common_encodings, encoding_for_label};

/// Outline rows: indentation per heading level and the row's base padding, in logical pixels.
const OUTLINE_INDENT: f32 = 12.0;
const OUTLINE_PADDING: f32 = 8.0;

/// Key context around the document editor: assists bind to "MdEditor > Input" so they apply to
/// the document only, never to the find bar or dialogs.
const EDITOR_CONTEXT: &str = "MdEditor";

actions!(
    mdpp,
    [
        Save,
        SaveAs,
        TogglePreview,
        NewFile,
        OpenFile,
        CloseTab,
        NextTab,
        PreviousTab,
        Quit,
        Find,
        FindReplace,
        FindNext,
        FindPrevious,
        CloseBar,
        ToggleOutline,
        ExportHtml,
        SmartEnter,
        SmartPaste,
        SmartTab,
        SmartShiftTab,
        ToggleBold,
        ToggleItalic,
        ToggleCode,
        ToggleStrikethrough,
        InsertLink,
        ToggleTask,
        AddHeadingLevel,
        RemoveHeadingLevel,
        DuplicateLines,
        DeleteLines,
        MoveLinesUp,
        MoveLinesDown,
        GoToLine,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        ToggleWordWrap,
        ToggleMenuBar,
        CommandPalette,
        QuickOpen,
        GoToHeading,
        OpenFolder,
        CloseFolder,
        ToggleFolderPanel,
        FindInFiles,
        FormatTable,
        RenameHeading,
        FindReferences,
        SaveAll,
        ReopenClosedTab,
        ReportBug,
        OpenSettingsFile,
        CloseOtherTabs,
        CloseTabsToRight,
        CloseSavedTabs,
        OpenSettings,
        OpenKeymap
    ]
);

/// "Open Recent" menu entry.
#[derive(Action, Clone, PartialEq, serde::Deserialize)]
#[action(namespace = mdpp, no_json)]
pub struct OpenRecent {
    pub path: PathBuf,
}

/// Status-bar line-ending menu: convert the document (applied on the next save).
#[derive(Action, Clone, PartialEq, serde::Deserialize)]
#[action(namespace = mdpp, no_json)]
pub struct ConvertEol {
    pub eol: Eol,
}

/// "Reopen with <encoding>" from the status-bar menu.
#[derive(Action, Clone, PartialEq, serde::Deserialize)]
#[action(namespace = mdpp, no_json)]
pub struct ReopenWithEncoding {
    pub label: SharedString,
}

/// "Save with <encoding>" from the status-bar menu.
#[derive(Action, Clone, PartialEq, serde::Deserialize)]
#[action(namespace = mdpp, no_json)]
pub struct SaveWithEncoding {
    pub label: SharedString,
}

/// Bind the keymap (defaults + user file). Returns problems with individual bindings.
pub fn bind_keymap(bindings: &[Binding], cx: &mut App) -> Vec<String> {
    let mut warnings = Vec::new();
    let mut keys = Vec::new();
    for binding in bindings {
        let Ok(action) = cx.build_action(&binding.action, None) else {
            warnings.push(format!("keymap: unknown action `{}`", binding.action));
            continue;
        };
        let context = binding.context.as_deref().unwrap_or(DEFAULT_CONTEXT);
        let Ok(predicate) = KeyBindingContextPredicate::parse(context) else {
            warnings.push(format!("keymap: invalid context `{context}`"));
            continue;
        };
        match KeyBinding::load(
            &binding.keys,
            action,
            Some(Rc::new(predicate)),
            false,
            None,
            cx,
        ) {
            Ok(key) => keys.push(key),
            Err(_) => warnings.push(format!("keymap: invalid keys `{}`", binding.keys)),
        }
    }
    cx.bind_keys(keys);
    warnings
}

/// Disk work for one session snapshot, done off the UI thread.
struct PersistJob {
    store: Store,
    session: Session,
    writes: Vec<(String, gpui_kit::component::input::Rope)>,
    removes: Vec<String>,
    /// Generation this job was prepared in; it does nothing if a newer flush already ran.
    generation: u64,
    lock: Arc<Mutex<u64>>,
}

impl PersistJob {
    fn run(self) {
        let Ok(current) = self.lock.lock() else {
            return;
        };
        if *current != self.generation {
            return; // a later, final flush already wrote newer state
        }
        write_session(&self.store, &self.session, self.writes, self.removes);
    }
}

/// Backups first, then the session that refers to them (PLAN §3.4).
fn write_session(
    store: &Store,
    session: &Session,
    writes: Vec<(String, gpui_kit::component::input::Rope)>,
    removes: Vec<String>,
) {
    for (name, text) in writes {
        if let Err(error) = store.write_backup(&name, &text.to_string()) {
            eprintln!("mdpp: could not write backup {name}: {error}");
        }
    }
    for name in removes {
        store.remove_backup(&name);
    }
    if let Err(error) = store.save(session) {
        eprintln!("mdpp: could not save the session: {error}");
    }
}

/// Everything the workspace starts from.
pub struct Startup {
    pub files: Vec<OpenRequest>,
    pub settings: Rc<Settings>,
    /// Hot exit storage; `None` disables persistence.
    pub store: Option<Store>,
    pub paths: Option<Paths>,
    /// Single-instance server: files from later `mdpp` launches arrive here.
    pub server: Option<Server>,
    pub warnings: Vec<String>,
    pub bench_mode: Option<bench::Mode>,
    pub started: Instant,
}

pub struct Workspace {
    documents: Vec<Entity<Document>>,
    /// Subscriptions to each document, same order as `documents`.
    document_subscriptions: Vec<[Subscription; 2]>,
    active: usize,
    show_preview: bool,
    find_bar: Entity<FindBar>,
    show_find: bool,
    prompt_bar: Entity<PromptBar>,
    show_prompt: bool,
    show_outline: bool,
    /// The folder opened with `mdpp <folder>` or File → Open Folder, and whether its panel shows.
    folder: Option<folder::FolderState>,
    /// The left panel shows, and which view: the folder or find in files.
    show_side: bool,
    side: Side,
    search_panel: Entity<crate::search_panel::SearchPanel>,
    /// The palette, while it is open.
    palette: Option<(Entity<crate::palette::Palette>, Subscription)>,
    /// The settings window, while it is open.
    settings_window: Option<(
        AnyWindowHandle,
        Entity<crate::settings_window::SettingsWindow>,
    )>,
    /// Zoom steps from the `font_size` setting.
    zoom: i32,
    /// Word wrap chosen with Alt+Z; `None` follows the settings.
    wrap: Option<bool>,
    /// Files of closed tabs, most recent last (Ctrl+Shift+T).
    closed: Vec<OpenRequest>,
    /// Recently opened files, most recent first.
    recent: Vec<PathBuf>,
    settings: Rc<Settings>,
    /// Hot exit (PLAN §3.4); `None` disables persistence.
    store: Option<Store>,
    last_session: Option<Session>,
    /// Serializes session writes; bumped by the final flush so stale background writes skip.
    persist_lock: Arc<Mutex<u64>>,
    paths: Option<Paths>,
    watcher: Option<DiskWatcher>,
    probe: SharedProbe,
    _watch: Task<()>,
    _forwarded: Task<()>,
    _autosave: Task<()>,
    _bench: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(startup: Startup, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let Startup {
            files,
            settings,
            store,
            paths,
            server,
            warnings,
            bench_mode,
            started,
        } = startup;
        let mut problems = warnings;
        problems.extend(crate::menus::problems(cx));
        let (changes, changed) = smol::channel::unbounded();
        let watcher = DiskWatcher::new(changes)
            .map_err(|error| problems.push(format!("File changes will not be noticed: {error}")))
            .ok();
        let find_bar = cx.new(|cx| FindBar::new(window, cx));
        let prompt_bar = cx.new(|cx| PromptBar::new(window, cx));
        let search_panel = {
            let workspace = cx.entity().downgrade();
            let files: crate::search_panel::FileSource = Rc::new(move |cx: &App| {
                workspace
                    .upgrade()
                    .map(|w| w.read(cx).searched_files(cx))
                    .unwrap_or_default()
            });
            let (debounce, max_bytes) =
                (settings.structure_debounce, settings.search_max_file_bytes);
            cx.new(|cx| {
                crate::search_panel::SearchPanel::new(files, debounce, max_bytes, window, cx)
            })
        };
        let subscriptions = vec![
            cx.subscribe_in(
                &search_panel,
                window,
                |this, _, found: &crate::search_panel::OpenMatch, window, cx| {
                    this.open_and_report(vec![found.0.clone()], window, cx)
                },
            ),
            cx.subscribe_in(
                &find_bar,
                window,
                |this, _, _: &CloseRequested, window, cx| this.hide_bars(window, cx),
            ),
            cx.subscribe_in(
                &prompt_bar,
                window,
                |this, _, _: &CloseRequested, window, cx| this.hide_bars(window, cx),
            ),
            cx.observe_window_appearance(window, |_, window, cx| {
                Theme::sync_system_appearance(Some(window), cx);
            }),
            // Hot exit: quitting and closing the window flush everything, without asking.
            cx.on_app_quit(|this, cx| {
                this.persist_now(cx);
                async {}
            }),
            cx.on_release(|this, cx| this.persist_now(cx)),
        ];
        let mut this = Self {
            documents: Vec::new(),
            document_subscriptions: Vec::new(),
            active: 0,
            show_preview: settings.preview_on_start,
            find_bar,
            show_find: false,
            prompt_bar,
            show_prompt: false,
            show_outline: settings.outline_on_start,
            folder: None,
            show_side: false,
            side: Side::Folder,
            search_panel,
            palette: None,
            settings_window: None,
            zoom: 0,
            wrap: None,
            closed: Vec::new(),
            recent: Vec::new(),
            settings,
            store,
            last_session: None,
            persist_lock: Arc::default(),
            paths,
            watcher,
            probe: SharedProbe::default(),
            _watch: Task::ready(()),
            _forwarded: Task::ready(()),
            _autosave: Task::ready(()),
            _bench: Task::ready(()),
            _subscriptions: subscriptions,
        };
        problems.extend(this.restore_session(window, cx));
        problems.extend(this.open_requests(files, window, cx));
        if this.documents.is_empty() {
            this.add_document(OpenedDoc::untitled(None), window, cx);
        }
        // Notifications need the window's root, which exists once this constructor returns.
        window.defer(cx, move |window, cx| {
            for problem in problems {
                eprintln!("mdpp: {problem}");
                window.push_notification(Notification::warning(problem), cx);
            }
        });
        // The close button asks about unsaved changes when hot exit is off, like Quit.
        let workspace = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            workspace
                .update(cx, |this, cx| this.should_close(window, cx))
                .unwrap_or(true)
        });
        this.refresh_watches(cx);
        // Coming back to the window lists the opened folder again (files made elsewhere).
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.scan_folder(cx);
            }
        });
        this._subscriptions.push(activation);
        this._watch = this.start_watching(changed, window, cx);
        this._forwarded = this.serve_instance(server, window, cx);
        this._autosave = this.start_autosave(cx);
        this._bench = this.start_bench(bench_mode, started, window, cx);
        this
    }

    /// Reopen the tabs of the last session (PLAN §3.4). Returns messages to show.
    fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<String> {
        let Some(session) = self.store.as_ref().and_then(Store::load) else {
            return Vec::new();
        };
        let store = self.store.clone().expect("checked above");
        self.show_preview = session.preview;
        self.show_outline = session.outline;
        self.zoom = session.zoom;
        self.wrap = session.wrap;
        self.recent = session.recent.clone();
        if let Some(root) = session.folder.clone().filter(|root| root.is_dir()) {
            self.open_folder(root, cx);
            self.show_side = session.folder_panel;
        }
        let mut messages = Vec::new();
        // Without hot exit nothing of the tabs is kept (unsaved changes were asked about).
        let documents = if self.settings.hot_exit {
            &session.documents[..]
        } else {
            &[]
        };
        for entry in documents {
            match OpenedDoc::restore(entry, &store) {
                Ok((opened, warning)) => {
                    messages.extend(warning);
                    self.add_document(opened, window, cx);
                }
                Err(error) => messages.push(format!("Could not restore a tab: {error:#}")),
            }
        }
        if !self.documents.is_empty() {
            self.activate(session.active.min(self.documents.len() - 1), window, cx);
        }
        messages
    }

    /// Snapshot what must be stored; `None` when nothing changed since the last snapshot.
    fn prepare_persist(&mut self, cx: &mut App, force: bool) -> Option<PersistJob> {
        let store = self.store.clone()?;
        let (mut entries, mut writes, mut removes) = (Vec::new(), Vec::new(), Vec::new());
        let documents = if self.settings.hot_exit {
            &self.documents[..]
        } else {
            &[]
        };
        for document in documents {
            let Persist {
                entry,
                write,
                remove,
            } = document.update(cx, |doc, cx| doc.persist(cx));
            entries.push(entry);
            writes.extend(write);
            removes.extend(remove);
        }
        let session = Session {
            version: SESSION_VERSION,
            active: self.active,
            preview: self.show_preview,
            outline: self.show_outline,
            zoom: self.zoom,
            wrap: self.wrap,
            recent: self.recent.clone(),
            folder: self.folder.as_ref().map(|f| f.root.clone()),
            folder_panel: self.show_side,
            documents: entries,
        };
        let unchanged =
            writes.is_empty() && removes.is_empty() && self.last_session.as_ref() == Some(&session);
        if unchanged && !force {
            return None;
        }
        self.last_session = Some(session.clone());
        let generation = self.persist_lock.lock().map(|g| *g).unwrap_or_default();
        Some(PersistJob {
            store,
            session,
            writes,
            removes,
            generation,
            lock: self.persist_lock.clone(),
        })
    }

    /// Back up unsaved changes every `hot_exit_interval`, one write at a time, off the UI thread.
    fn start_autosave(&mut self, cx: &mut Context<Self>) -> Task<()> {
        if self.store.is_none() {
            return Task::ready(());
        }
        let interval = self.settings.hot_exit_interval;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(interval).await;
                let Ok(job) = this.update(cx, |this, cx| this.prepare_persist(cx, false)) else {
                    break; // the workspace is gone
                };
                if let Some(job) = job {
                    cx.background_spawn(async move { job.run() }).await;
                }
            }
        })
    }

    /// Final flush on quit or window close: synchronous, and it wins over any write still queued.
    fn persist_now(&mut self, cx: &mut App) {
        let Some(job) = self.prepare_persist(cx, true) else {
            return;
        };
        let Ok(mut generation) = self.persist_lock.lock() else {
            return;
        };
        *generation += 1;
        write_session(&job.store, &job.session, job.writes, job.removes);
    }

    /// Files forwarded by later `mdpp` launches: open them and bring the window forward.
    fn serve_instance(
        &mut self,
        server: Option<Server>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let Some(server) = server else {
            return Task::ready(());
        };
        let (sender, received) = smol::channel::unbounded();
        server.serve(move |requests| {
            let _ = sender.try_send(requests);
        });
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(requests) = received.recv().await {
                let opened = this.update_in(cx, |this, window, cx| {
                    this.open_and_report(requests, window, cx);
                    window.activate_window();
                });
                if opened.is_err() {
                    break;
                }
            }
        })
    }

    /// Watch the folders of open files and the config folder (PLAN §3.3).
    pub(super) fn refresh_watches(&mut self, cx: &App) {
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        let mut folders: HashSet<PathBuf> = self
            .documents
            .iter()
            .filter_map(|doc| doc.read(cx).path().and_then(folder_of))
            .collect();
        folders.extend(
            self.paths
                .as_ref()
                .map(|p| p.config.clone())
                .filter(|dir| dir.exists()),
        );
        watcher.watch_folders(folders);
    }

    /// Drain change notifications; a burst within the debounce counts once.
    fn start_watching(
        &mut self,
        changed: smol::channel::Receiver<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let debounce = self.settings.file_watch_debounce;
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(first) = changed.recv().await {
                cx.background_executor().timer(debounce).await;
                let mut paths = HashSet::from([first]);
                while let Ok(more) = changed.try_recv() {
                    paths.insert(more);
                }
                if this
                    .update_in(cx, |this, window, cx| {
                        this.on_disk_changes(&paths, window, cx)
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    fn on_disk_changes(
        &mut self,
        paths: &HashSet<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for document in self.documents.clone() {
            let touched = document.read(cx).path().is_some_and(|p| paths.contains(p));
            if touched {
                document.update(cx, |doc, cx| doc.check_disk(window, cx));
            }
        }
        let settings_file = self.paths.as_ref().map(Paths::settings_file);
        if settings_file.is_some_and(|file| paths.contains(&file)) {
            self.reload_settings(window, cx);
        }
    }

    /// settings.toml changed: apply it. Most values take effect at once; a few (soft wrap, line
    /// numbers, markup fade, preview width) apply to tabs opened afterwards.
    pub(super) fn reload_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let file = self.paths.as_ref().map(Paths::settings_file);
        let text = file
            .and_then(|f| std::fs::read_to_string(f).ok())
            .unwrap_or_default();
        let loaded = Settings::load(&text);
        let changed = loaded.settings != *self.settings;
        let settings = Rc::new(loaded.settings);
        crate::highlight::configure_language(&settings.auto_close_pairs, cx);
        for doc in &self.documents {
            doc.update(cx, |doc, _| doc.set_settings(settings.clone()));
        }
        Theme::update(cx, |theme| {
            theme.cursor_blink =
                gpui_kit::component::input::CursorBlink::blinking(settings.cursor_blink_interval)
                    .with_idle_timeout(settings.cursor_blink_idle_timeout);
        });
        self.search_panel.update(cx, |panel, _| {
            panel.set_limits(settings.structure_debounce, settings.search_max_file_bytes)
        });
        self.settings = settings;
        eprintln!("mdpp: settings reloaded");
        // The settings window shows its own changes; a hand edit that changed something gets a
        // notice (not the watcher's echo of a change made in MD++ itself).
        match &self.settings_window {
            Some((_, view)) => view.update(cx, |view, cx| {
                view.reload();
                cx.notify();
            }),
            None if changed => {
                window.push_notification(Notification::info("Settings reloaded"), cx)
            }
            None => {}
        }
        cx.notify();
        for warning in loaded.warnings {
            eprintln!("mdpp: {warning}");
            window.push_notification(Notification::warning(warning), cx);
        }
    }

    fn render_conflict(&self, conflict: DiskConflict, cx: &mut Context<Self>) -> impl IntoElement {
        let name = self.active_document().read(cx).name();
        let (message, reload) = match conflict {
            DiskConflict::Changed => (
                format!("{name} changed on disk, and you have unsaved changes."),
                true,
            ),
            DiskConflict::Deleted => (
                format!(
                    "{name} was deleted on disk. Its text stays here — save to write it again."
                ),
                false,
            ),
        };
        h_flex()
            .w_full()
            .gap_2()
            .px_3()
            .py_1()
            .bg(cx.theme().warning.opacity(0.15))
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().flex_1().text_sm().child(message))
            .when(reload, |bar| {
                bar.child(
                    Button::new("reload-from-disk")
                        .label("Reload (discard mine)")
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.active_document()
                                .update(cx, |doc, cx| doc.reload(window, cx));
                        })),
                )
            })
            .child(
                Button::new("keep-mine")
                    .label(if reload { "Keep mine" } else { "OK" })
                    .small()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.active_document()
                            .update(cx, |doc, cx| doc.keep_mine(cx));
                    })),
            )
    }

    fn reopen_with_encoding(
        &mut self,
        action: &ReopenWithEncoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(encoding) = encoding_for_label(&action.label) {
            self.active_document()
                .update(cx, |doc, cx| doc.reopen_with(encoding, window, cx));
        }
    }

    fn save_with_encoding(
        &mut self,
        action: &SaveWithEncoding,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(encoding) = encoding_for_label(&action.label) {
            self.active_document()
                .update(cx, |doc, cx| doc.save_with(encoding, window, cx));
        }
    }

    /// Open `path` (or switch to it) and put the caret on the heading with `anchor`.
    fn open_at_anchor(
        &mut self,
        path: PathBuf,
        anchor: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_paths(vec![path], window, cx);
        let Some(anchor) = anchor else {
            return;
        };
        let source = self
            .active_document()
            .read(cx)
            .editor()
            .read(cx)
            .text()
            .to_string();
        match mdpp_markdown::DocIndex::build(&source).heading_with_anchor(&anchor) {
            Some(heading) => self.jump_to(heading.range.start, window, cx),
            None => window.push_notification(
                Notification::warning(format!("No heading #{anchor} there.")),
                cx,
            ),
        }
    }

    fn export_html(&mut self, _: &ExportHtml, window: &mut Window, cx: &mut Context<Self>) {
        self.active_document()
            .update(cx, |doc, cx| doc.export_html(window, cx));
    }

    fn toggle_outline(&mut self, _: &ToggleOutline, _window: &mut Window, cx: &mut Context<Self>) {
        self.show_outline = !self.show_outline;
        cx.notify();
    }

    /// Move the caret to `offset` in the active editor and focus it (outline clicks).
    fn jump_to(&mut self, offset: usize, window: &mut Window, cx: &mut Context<Self>) {
        let editor = self.active_document().read(cx).editor().clone();
        editor.update(cx, |state, cx| {
            let position = state.text().offset_to_position(offset);
            state.set_cursor_position(position, window, cx);
        });
        window.focus(&editor.focus_handle(cx), cx);
    }

    /// Headings of the active document, indented by level; the one containing the caret is
    /// highlighted. Virtualized: only visible rows are built (PLAN §4.2).
    fn render_outline(&self, cx: &mut Context<Self>) -> AnyElement {
        let doc = self.active_document().read(cx);
        let Some(index) = doc.index() else {
            return div().into_any_element();
        };
        let cursor = doc.editor().read(cx).cursor();
        let current = index
            .headings
            .partition_point(|h| h.range.start <= cursor)
            .checked_sub(1);
        let count = index.headings.len();
        if count == 0 {
            return div()
                .p_3()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("No headings")
                .into_any_element();
        }
        uniform_list(
            "outline",
            count,
            cx.processor(move |_, range: std::ops::Range<usize>, _window, cx| {
                let theme = cx.theme();
                let (active_bg, hover_bg) = (theme.accent, theme.secondary);
                range
                    .map(|ix| {
                        let heading = &index.headings[ix];
                        let start = heading.range.start;
                        let indent = px(OUTLINE_INDENT
                            * f32::from(heading.level.saturating_sub(1))
                            + OUTLINE_PADDING);
                        div()
                            .id(ix)
                            .w_full()
                            .pl(indent)
                            .pr_2()
                            .py_0p5()
                            .text_sm()
                            .truncate()
                            .cursor_pointer()
                            .when(Some(ix) == current, |row| row.bg(active_bg))
                            .hover(move |row| row.bg(hover_bg))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.jump_to(start, window, cx)
                            }))
                            .child(heading.text.clone())
                    })
                    .collect()
            }),
        )
        .size_full()
        .py_1()
        .into_any_element()
    }

    fn open_find(&mut self, replace: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find = true;
        let editor = self.active_document().read(cx).editor().clone();
        self.find_bar
            .update(cx, |bar, cx| bar.open(editor, replace, window, cx));
        cx.notify();
    }

    /// Close the find and go-to-line bars and give the editor the keyboard back.
    fn hide_bars(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_find = false;
        self.show_prompt = false;
        self.find_bar.update(cx, |bar, cx| bar.close(cx));
        let editor = self.active_document().read(cx).editor().clone();
        window.focus(&editor.focus_handle(cx), cx);
        cx.notify();
    }

    fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(false, window, cx);
    }

    fn find_replace(&mut self, _: &FindReplace, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find(true, window, cx);
    }

    fn find_next(&mut self, _: &FindNext, window: &mut Window, cx: &mut Context<Self>) {
        if !self.show_find {
            return self.open_find(false, window, cx);
        }
        self.find_bar
            .update(cx, |bar, cx| bar.step(true, window, cx));
    }

    fn find_previous(&mut self, _: &FindPrevious, window: &mut Window, cx: &mut Context<Self>) {
        if !self.show_find {
            return self.open_find(false, window, cx);
        }
        self.find_bar
            .update(cx, |bar, cx| bar.step(false, window, cx));
    }

    fn close_bar(&mut self, _: &CloseBar, window: &mut Window, cx: &mut Context<Self>) {
        self.hide_bars(window, cx);
    }

    /// Open files (at a line and column when given), or switch to them when already open.
    /// Returns errors to show.
    fn open_requests(
        &mut self,
        requests: Vec<OpenRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<String> {
        let mut errors = Vec::new();
        for request in requests {
            let path = request.path.clone();
            if path.is_dir() {
                self.open_folder(path, cx);
                continue;
            }
            // The CLI counts from 1, the editor from 0.
            let cursor = request
                .line
                .map(|line| (line - 1, request.column.map_or(0, |c| c - 1)));
            let open = self
                .documents
                .iter()
                .position(|doc| doc.read(cx).path().is_some_and(|p| same_file(p, &path)));
            if let Some(index) = open {
                self.activate(index, window, cx);
                if let Some((line, column)) = cursor {
                    let editor = self.documents[index].read(cx).editor().clone();
                    editor.update(cx, |state, cx| {
                        state.set_cursor_position(Position::new(line, column), window, cx)
                    });
                }
                continue;
            }
            match OpenedDoc::load(&path).map(|doc| OpenedDoc {
                cursor: cursor.or(doc.cursor),
                ..doc
            }) {
                Ok(doc) => {
                    if doc.had_errors {
                        errors.push(format!(
                            "{} has bytes that are not valid {}; if the text looks wrong, use the encoding menu in the status bar to reopen it with another encoding.",
                            path.display(),
                            doc.encoding.name()
                        ));
                    }
                    // Replace a pristine untitled tab instead of piling up empty ones.
                    if self.documents.len() == 1 && self.is_pristine_untitled(0, cx) {
                        self.remove_at(0);
                    }
                    self.note_recent(&path);
                    self.add_document(doc, window, cx);
                }
                Err(error) => errors.push(format!("{error:#}")),
            }
        }
        errors
    }

    fn is_pristine_untitled(&self, index: usize, cx: &App) -> bool {
        let doc = self.documents[index].read(cx);
        doc.path().is_none() && !doc.is_dirty() && doc.editor().read(cx).text().len() == 0
    }

    fn add_document(&mut self, opened: OpenedDoc, window: &mut Window, cx: &mut Context<Self>) {
        let settings = self.settings.clone();
        let show_preview = self.show_preview;
        let wrap = self.wrap;
        let document = cx.new(|cx| {
            let mut doc = Document::new(opened, settings, window, cx);
            doc.set_preview_visible(show_preview, cx);
            if let Some(wrap) = wrap {
                doc.set_soft_wrap(wrap, window, cx);
            }
            doc
        });
        let subscriptions = [
            cx.subscribe_in(
                &document,
                window,
                |this, _, _: &DocumentChanged, window, cx| {
                    this.refresh_title(window, cx);
                    cx.notify();
                },
            ),
            cx.subscribe(&document, |this, _, Renamed(path), cx| {
                this.note_recent(path);
                this.refresh_watches(cx);
            }),
        ];
        // Go to definition into another file opens it here, at the heading (PLAN §4.3).
        let workspace = cx.entity().downgrade();
        let show_document: gpui_kit::component::input::ShowDocumentHandler =
            Rc::new(move |params, window, cx| {
                let uri = params.uri.as_str();
                if uri.starts_with("mailto:") {
                    cx.open_url(uri);
                    return true;
                }
                let Some((path, anchor)) = mdpp_lang::path_from_file_uri(uri) else {
                    return false; // this document: the editor jumps within itself
                };
                let workspace = workspace.clone();
                window.defer(cx, move |window, cx| {
                    workspace
                        .update(cx, |this, cx| this.open_at_anchor(path, anchor, window, cx))
                        .ok();
                });
                true
            });
        document.update(cx, |doc, cx| {
            doc.editor().update(cx, |state, _| {
                state.lsp_mut().show_document = Some(show_document)
            });
        });
        self.documents.push(document);
        self.document_subscriptions.push(subscriptions);
        self.refresh_watches(cx); // its folder may not be watched yet
        self.activate(self.documents.len() - 1, window, cx);
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(doc) = self.documents.get(index) else {
            return;
        };
        self.active = index;
        let editor = doc.read(cx).editor().clone();
        window.focus(&editor.focus_handle(cx), cx);
        if self.show_find {
            self.find_bar
                .update(cx, |bar, cx| bar.retarget(editor, window, cx));
        }
        self.refresh_title(window, cx);
        cx.notify();
    }

    fn active_document(&self) -> &Entity<Document> {
        &self.documents[self.active]
    }

    fn refresh_title(&self, window: &mut Window, cx: &App) {
        let doc = self.active_document().read(cx);
        let marker = if doc.is_dirty() { "● " } else { "" };
        window.set_window_title(&format!("{marker}{} — MD++", doc.name()));
    }

    fn remove_at(&mut self, index: usize) {
        self.documents.remove(index);
        drop(self.document_subscriptions.remove(index)); // unsubscribes
        if self.active >= self.documents.len() {
            self.active = self.documents.len().saturating_sub(1);
        }
    }

    fn remove_document(
        &mut self,
        document: &Entity<Document>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.documents.iter().position(|d| d == document) else {
            return;
        };
        // Ctrl+Shift+T brings the file back (its unsaved changes were saved or discarded).
        let doc = document.read(cx);
        if let Some(path) = doc.path() {
            let cursor = doc.editor().read(cx).cursor_position();
            self.closed.push(OpenRequest {
                path: path.to_path_buf(),
                line: Some(cursor.line + 1),
                column: Some(cursor.character + 1),
            });
        }
        // Closed tabs do not come back with unsaved text: their backup goes with them.
        let backup = document.update(cx, |doc, _| doc.discard_backup());
        if let (Some(store), Some(name)) = (&self.store, backup) {
            store.remove_backup(&name);
        }
        self.remove_at(index);
        self.refresh_watches(cx);
        if self.documents.is_empty() {
            // Like Notepad++: closing the last tab leaves an empty one, not an empty window.
            self.add_document(OpenedDoc::untitled(None), window, cx);
        } else {
            let active = self.active;
            self.activate(active, window, cx);
        }
    }

    fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        self.active_document()
            .update(cx, |doc, cx| doc.save(window, cx))
            .detach();
    }

    fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        self.active_document()
            .update(cx, |doc, cx| doc.save_as(window, cx))
            .detach();
    }

    fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        self.add_document(OpenedDoc::untitled(None), window, cx);
    }

    fn open_file(&mut self, _: &OpenFile, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await {
                this.update_in(cx, |this, window, cx| this.open_paths(paths, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        let next = (self.active + 1) % self.documents.len();
        self.activate(next, window, cx);
    }

    fn previous_tab(&mut self, _: &PreviousTab, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.documents.len();
        self.activate((self.active + count - 1) % count, window, cx);
    }

    fn toggle_preview(&mut self, _: &TogglePreview, _window: &mut Window, cx: &mut Context<Self>) {
        self.show_preview = !self.show_preview;
        let visible = self.show_preview;
        for doc in &self.documents {
            doc.update(cx, |doc, cx| doc.set_preview_visible(visible, cx));
        }
        cx.notify();
    }

    fn convert_eol(&mut self, action: &ConvertEol, _window: &mut Window, cx: &mut Context<Self>) {
        self.active_document()
            .update(cx, |doc, cx| doc.set_eol(action.eol, cx));
    }

    fn render_status(&self, cx: &App) -> impl IntoElement {
        let doc = self.active_document().read(cx);
        let state = doc.editor().read(cx);
        let cursor = state.cursor_position();
        let selection = state.selected_range();
        let selected = (!selection.is_empty())
            .then(|| state.text().slice(selection).chars().count())
            .map(|chars| format!(" ({chars} selected)"))
            .unwrap_or_default();
        let lines = state.text().len_lines(ropey::LineType::LF);
        let muted = cx.theme().muted_foreground;
        let item = move |text: String| div().text_xs().text_color(muted).child(text);
        let (encoding, eol) = doc.format_labels();
        let diagnostics = doc.diagnostic_count();
        let words = doc.word_count();
        let minutes = words
            .div_ceil(self.settings.reading_words_per_minute.max(1) as usize)
            .max(1);
        let reading = format!("{words} words · {minutes} min read");
        StatusBar::new()
            .left(
                h_flex()
                    .gap_3()
                    .child(item(format!(
                        "Ln {}, Col {}{selected}",
                        cursor.line + 1,
                        cursor.character + 1
                    )))
                    .child(item(format!("{lines} lines")))
                    .child(item(reading)),
            )
            .right(
                h_flex()
                    .gap_3()
                    .when(diagnostics > 0, |row| {
                        row.child(item(format!("⚠ {diagnostics}")))
                    })
                    .child(
                        Button::new("encoding")
                            .label(encoding)
                            .ghost()
                            .xsmall()
                            .dropdown_menu(|menu, window, cx| {
                                menu.submenu("Reopen with", window, cx, |menu, _, _| {
                                    common_encodings().fold(menu, |menu, label| {
                                        menu.menu(
                                            label,
                                            Box::new(ReopenWithEncoding {
                                                label: label.into(),
                                            }),
                                        )
                                    })
                                })
                                .submenu(
                                    "Save with",
                                    window,
                                    cx,
                                    |menu, _, _| {
                                        common_encodings().fold(menu, |menu, label| {
                                            menu.menu(
                                                label,
                                                Box::new(SaveWithEncoding {
                                                    label: label.into(),
                                                }),
                                            )
                                        })
                                    },
                                )
                            }),
                    )
                    .child(
                        Button::new("line-endings")
                            .label(eol)
                            .ghost()
                            .xsmall()
                            .tooltip("Line endings")
                            .dropdown_menu(|menu, _, _| {
                                Eol::ALL.into_iter().fold(
                                    menu.label("Convert to (on the next save)"),
                                    |menu, eol| {
                                        menu.menu(eol.to_string(), Box::new(ConvertEol { eol }))
                                    },
                                )
                            }),
                    )
                    .child(item("Markdown".to_string())),
            )
    }
}

/// Same file, comparing canonical paths when they can be resolved.
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let probe = self.probe.clone();
        let conflict = self.active_document().read(cx).conflict();
        let doc = self.active_document().read(cx);
        let editor = div().key_context(EDITOR_CONTEXT).size_full().child(
            Editor::new(doc.editor())
                .bordered(false)
                .h_full()
                .font_family(cx.theme().mono_font_family.clone())
                .text_size(px(self.font_size())),
        );
        let preview = doc.preview().clone();
        let folder = self.render_side(cx);
        let body = if self.show_preview || self.show_outline || folder.is_some() {
            let mut panels = h_resizable("workspace-panels");
            if let Some(folder) = folder {
                panels = panels.child(
                    resizable_panel()
                        .size(px(self.settings.outline_width))
                        .child(folder),
                );
            }
            if self.show_outline {
                let outline = self.render_outline(cx);
                panels = panels.child(
                    resizable_panel()
                        .size(px(self.settings.outline_width))
                        .child(div().size_full().bg(cx.theme().sidebar).child(outline)),
                );
            }
            panels = panels.child(resizable_panel().child(editor));
            if self.show_preview {
                panels = panels.child(resizable_panel().child(preview));
            }
            panels.into_any_element()
        } else {
            editor.into_any_element()
        };
        v_flex()
            .relative()
            .size_full()
            .key_context(DEFAULT_CONTEXT)
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::toggle_preview))
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::open_file))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::smart_enter))
            .on_action(cx.listener(Self::smart_paste))
            .on_action(cx.listener(Self::smart_tab))
            .on_action(cx.listener(Self::smart_shift_tab))
            .on_action(cx.listener(Self::add_heading_level))
            .on_action(cx.listener(Self::remove_heading_level))
            .on_action(cx.listener(Self::duplicate_lines))
            .on_action(cx.listener(Self::delete_lines))
            .on_action(cx.listener(Self::move_lines_up))
            .on_action(cx.listener(Self::move_lines_down))
            .on_action(cx.listener(Self::go_to_line))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .on_action(cx.listener(Self::toggle_word_wrap))
            .on_action(cx.listener(Self::toggle_menu_bar))
            .on_action(cx.listener(Self::command_palette))
            .on_action(cx.listener(Self::quick_open))
            .on_action(cx.listener(Self::go_to_heading))
            .on_action(cx.listener(Self::open_folder_dialog))
            .on_action(cx.listener(Self::close_folder))
            .on_action(cx.listener(Self::toggle_folder_panel))
            .on_action(cx.listener(Self::find_in_files))
            .on_action(cx.listener(Self::format_table))
            .on_action(cx.listener(Self::rename_heading))
            .on_action(cx.listener(Self::find_references))
            .on_action(cx.listener(Self::save_all))
            .on_action(cx.listener(Self::reopen_closed_tab))
            .on_action(cx.listener(Self::close_other_tabs))
            .on_action(cx.listener(Self::close_tabs_to_right))
            .on_action(cx.listener(Self::close_saved_tabs))
            .on_action(cx.listener(Self::open_recent))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::open_keymap))
            .on_action(cx.listener(Self::open_settings_file))
            .on_action(cx.listener(Self::report_bug))
            .on_action(cx.listener(Self::convert_eol))
            .on_action(cx.listener(Self::toggle_outline))
            .on_action(cx.listener(Self::export_html))
            .on_action(cx.listener(Self::toggle_bold))
            .on_action(cx.listener(Self::toggle_italic))
            .on_action(cx.listener(Self::toggle_code))
            .on_action(cx.listener(Self::toggle_strikethrough))
            .on_action(cx.listener(Self::insert_link))
            .on_action(cx.listener(Self::toggle_task))
            .on_action(cx.listener(Self::reopen_with_encoding))
            .on_action(cx.listener(Self::save_with_encoding))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::find_replace))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::close_bar))
            // Dropped images are linked into the document; other files open as tabs.
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                let (images, others): (Vec<PathBuf>, Vec<PathBuf>) =
                    paths.paths().iter().cloned().partition(|path| {
                        path.is_file()
                            && mdpp_lang::is_image(
                                &path.file_name().unwrap_or_default().to_string_lossy(),
                            )
                    });
                if !images.is_empty() {
                    let images = images.into_iter().map(editing::NewImage::File).collect();
                    this.insert_images(images, window, cx);
                }
                this.open_paths(others, window, cx)
            }))
            .bg(cx.theme().background)
            .when(self.settings.menu_bar, |root| {
                root.child(self.render_menu_bar(cx))
            })
            .child(self.render_tabs(cx))
            .when_some(conflict, |root, conflict| {
                root.child(self.render_conflict(conflict, cx))
            })
            .when(self.show_find, |root| root.child(self.find_bar.clone()))
            .when(self.show_prompt, |root| root.child(self.prompt_bar.clone()))
            .child(div().flex_1().min_h_0().child(body))
            .child(self.render_status(cx))
            // Painted after the editor: marks "the frame is on its way" for the measurements.
            .children(self.render_palette(cx))
            .child(canvas(|_, _, _| {}, move |_, _, _, _| probe.borrow_mut().painted()).size_0())
    }
}
