//! Tabs and files (PLAN §4.1, §6.1): the tab bar with the menu bar (≡) and a menu per tab, closing
//! several tabs, reopening closed ones, save all, recent files, dropped files, the settings and
//! keymap files, and asking about unsaved changes when hot exit is off.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_kit::component::Selectable as _;
use gpui_kit::component::menu::ContextMenuExt as _;

use crate::menus;
use crate::settings_window::{MainWindowCall, SettingsWindow};

/// The settings window's size: room for the category list and a readable row of descriptions.
const SETTINGS_WINDOW_SIZE: (f32, f32) = (900.0, 640.0);

use super::*;

impl Workspace {
    /// Ask Save / Don't Save / Cancel when `document` has unsaved changes. Resolves to whether it
    /// may go: saved, or the changes discarded.
    pub(super) fn ask_to_save(
        &mut self,
        document: Entity<Document>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if !document.read(cx).is_dirty() {
            return Task::ready(true);
        }
        if let Some(index) = self.documents.iter().position(|d| *d == document) {
            self.activate(index, window, cx); // show what the question is about
        }
        let name = document.read(cx).name();
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!("Save changes to {name}?"),
            Some("Your changes will be lost if you close without saving."),
            &["Save", "Don't Save", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |_, cx| match answer.await {
            Ok(0) => match document.update_in(cx, |doc, window, cx| doc.save(window, cx)) {
                Ok(saving) => saving.await,
                Err(_) => false,
            },
            Ok(1) => true,
            _ => false,
        })
    }

    /// Close a tab, asking first when it has unsaved changes. Resolves to whether it closed.
    pub(super) fn close(
        &mut self,
        document: Entity<Document>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let asked = self.ask_to_save(document.clone(), window, cx);
        cx.spawn_in(window, async move |this, cx| {
            asked.await
                && this
                    .update_in(cx, |this, window, cx| {
                        this.remove_document(&document, window, cx)
                    })
                    .is_ok()
        })
    }

    /// Close tabs one after another; Cancel on any of them keeps it and the rest.
    fn close_many(
        &mut self,
        documents: Vec<Entity<Document>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |this, cx| {
            for document in documents {
                let Ok(closing) =
                    this.update_in(cx, |this, window, cx| this.close(document, window, cx))
                else {
                    return;
                };
                if !closing.await {
                    return;
                }
            }
        })
        .detach();
    }

    pub(super) fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        let document = self.active_document().clone();
        self.close(document, window, cx).detach();
    }

    pub(super) fn close_other_tabs(
        &mut self,
        _: &CloseOtherTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active = self.active_document().clone();
        let others = self
            .documents
            .iter()
            .filter(|d| **d != active)
            .cloned()
            .collect();
        self.close_many(others, window, cx);
    }

    pub(super) fn close_tabs_to_right(
        &mut self,
        _: &CloseTabsToRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let right = self.documents[self.active + 1..].to_vec();
        self.close_many(right, window, cx);
    }

    pub(super) fn close_saved_tabs(
        &mut self,
        _: &CloseSavedTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let saved = self
            .documents
            .iter()
            .filter(|d| !d.read(cx).is_dirty())
            .cloned()
            .collect();
        self.close_many(saved, window, cx);
    }

    /// Ctrl+Shift+T: the most recently closed file that is not open and still exists.
    pub(super) fn reopen_closed_tab(
        &mut self,
        _: &ReopenClosedTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        while let Some(request) = self.closed.pop() {
            let open = self.documents.iter().any(|d| {
                d.read(cx)
                    .path()
                    .is_some_and(|p| same_file(p, &request.path))
            });
            if !open && request.path.exists() {
                self.open_and_report(vec![request], window, cx);
                return;
            }
        }
    }

    /// Save every tab with unsaved changes, one after another (untitled ones ask for a name).
    pub(super) fn save_all(&mut self, _: &SaveAll, window: &mut Window, cx: &mut Context<Self>) {
        let dirty: Vec<_> = self
            .documents
            .iter()
            .filter(|d| d.read(cx).is_dirty())
            .cloned()
            .collect();
        cx.spawn_in(window, async move |_, cx| {
            for document in dirty {
                if let Ok(saving) = document.update_in(cx, |doc, window, cx| doc.save(window, cx)) {
                    saving.await;
                }
            }
        })
        .detach();
    }

    /// With hot exit off, quitting asks about each tab with unsaved changes; Cancel stops it.
    pub(super) fn confirm_unsaved(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if self.settings.hot_exit {
            return Task::ready(true);
        }
        let dirty: Vec<_> = self
            .documents
            .iter()
            .filter(|d| d.read(cx).is_dirty())
            .cloned()
            .collect();
        cx.spawn_in(window, async move |this, cx| {
            for document in dirty {
                let Ok(asked) = this.update_in(cx, |this, window, cx| {
                    this.ask_to_save(document, window, cx)
                }) else {
                    return false;
                };
                if !asked.await {
                    return false;
                }
            }
            true
        })
    }

    pub(super) fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        let confirmed = self.confirm_unsaved(window, cx);
        cx.spawn(async move |_, cx| {
            if confirmed.await {
                cx.update(|cx| cx.quit());
            }
        })
        .detach();
    }

    /// The window's close button: like Quit. `false` keeps the window while the questions run.
    pub(super) fn should_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let unsaved = self.documents.iter().any(|d| d.read(cx).is_dirty());
        if self.settings.hot_exit || !unsaved {
            return true;
        }
        self.quit(&Quit, window, cx);
        false
    }

    /// Remember `path` at the top of the recent files.
    pub(super) fn note_recent(&mut self, path: &Path) {
        self.recent.retain(|p| !same_file(p, path));
        self.recent.insert(0, path.to_path_buf());
        self.recent.truncate(self.settings.recent_files_max);
    }

    /// Open files and show what went wrong as notifications.
    pub(super) fn open_and_report(
        &mut self,
        requests: Vec<OpenRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for error in self.open_requests(requests, window, cx) {
            window.push_notification(Notification::error(error), cx);
        }
    }

    pub(super) fn open_paths(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let requests = paths
            .into_iter()
            .map(|path| OpenRequest {
                path,
                line: None,
                column: None,
            })
            .collect();
        self.open_and_report(requests, window, cx);
    }

    pub(super) fn open_recent(
        &mut self,
        action: &OpenRecent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_paths(vec![action.path.clone()], window, cx);
    }

    /// Open the user's settings or keymap file, creating it from `template` the first time.
    fn open_config_file(
        &mut self,
        path: Option<PathBuf>,
        template: fn() -> String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = path else {
            let message = "This system has no configuration folder for MD++.";
            return window.push_notification(Notification::error(message), cx);
        };
        if !path.exists() {
            let created = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, template()));
            if let Err(error) = created {
                let message = format!("Could not create {}: {error}", path.display());
                return window.push_notification(Notification::error(message), cx);
            }
        }
        self.open_paths(vec![path], window, cx);
    }

    /// Settings (Ctrl+,): the settings window, or the one already open brought forward.
    pub(super) fn open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((handle, _)) = &self.settings_window
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let path = self.paths.as_ref().map(Paths::settings_file);
        // "Edit in settings.toml" opens the file here, in the main window.
        let main = window.window_handle();
        let edit_file: MainWindowCall = Rc::new(move |cx: &mut App| {
            main.update(cx, |_, window, cx| {
                window.activate_window();
                window.dispatch_action(Box::new(OpenSettingsFile), cx);
            })
            .ok();
        });
        // A change applies at once; the watcher would miss a config folder that did not exist.
        let workspace = cx.entity().downgrade();
        let on_change: MainWindowCall = Rc::new(move |cx: &mut App| {
            let workspace = workspace.clone();
            main.update(cx, |_, window, cx| {
                workspace
                    .update(cx, |this, cx| {
                        this.refresh_watches(cx);
                        this.reload_settings(window, cx);
                    })
                    .ok();
            })
            .ok();
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(
                size(px(SETTINGS_WINDOW_SIZE.0), px(SETTINGS_WINDOW_SIZE.1)),
                cx,
            )),
            ..Default::default()
        };
        let opened = gpui_kit::open_window(options, cx, |window, cx| {
            window.set_window_title("MD++ Settings");
            cx.new(|_| SettingsWindow::new(path, edit_file, on_change))
        });
        match opened {
            Ok((handle, view)) => {
                // Forget the window once it closes, so the next Ctrl+, opens a new one.
                cx.observe_release(&view, |this, _, _| this.settings_window = None)
                    .detach();
                self.settings_window = Some((handle, view));
            }
            Err(error) => window.push_notification(
                Notification::error(format!("Could not open the settings window: {error:#}")),
                cx,
            ),
        }
    }

    /// The user's settings.toml in a tab, created with every default commented out.
    pub(super) fn open_settings_file(
        &mut self,
        _: &OpenSettingsFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = self.paths.as_ref().map(Paths::settings_file);
        self.open_config_file(path, mdpp_core::settings::user_template, window, cx);
    }

    pub(super) fn open_keymap(
        &mut self,
        _: &OpenKeymap,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = self.paths.as_ref().map(Paths::keymap_file);
        self.open_config_file(path, mdpp_core::keymap::user_template, window, cx);
    }

    /// ≡ and, once clicked, the menu bar beside it (File · Edit · …) as in Zed: short menus
    /// instead of one long list. Clicking ≡ again folds it away.
    fn app_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.active_document().read(cx).editor().focus_handle(cx);
        let state = Rc::new(self.menu_state(cx));
        h_flex()
            .gap_0p5()
            .child(
                Button::new("app-menu")
                    .icon(IconName::Menu)
                    .ghost()
                    .small()
                    .tooltip("Menu")
                    .selected(self.show_menu_bar)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_menu_bar = !this.show_menu_bar;
                        cx.notify();
                    })),
            )
            .when(self.show_menu_bar, |row| {
                row.children(menus::names().enumerate().map(|(index, name)| {
                    let (state, focus) = (state.clone(), focus.clone());
                    Button::new(("menu-bar", index))
                        .label(name)
                        .ghost()
                        .small()
                        .dropdown_menu(move |menu, window, cx| {
                            let menu = menu.action_context(focus.clone());
                            menus::fill(menu, index, &state, window, cx)
                        })
                }))
            })
    }

    /// Check marks and the recent files, as the menus show them.
    fn menu_state(&self, cx: &App) -> menus::MenuState {
        let wrap = self.active_document().read(cx).soft_wrap();
        let checked = [
            (menus::CHECK_OUTLINE, self.show_outline),
            (menus::CHECK_PREVIEW, self.show_preview),
            (menus::CHECK_WORD_WRAP, wrap),
        ]
        .into_iter()
        .filter_map(|(name, on)| on.then_some(name))
        .collect();
        let recent = self
            .recent
            .iter()
            .map(|path| {
                let action: Box<dyn Action> = Box::new(OpenRecent { path: path.clone() });
                (recent_label(path), action)
            })
            .collect();
        menus::MenuState {
            checked,
            lists: HashMap::from([(menus::LIST_RECENT, recent)]),
        }
    }

    /// Help ▸ Report a Bug: GitHub's issue form with the version and system filled in. Nothing is
    /// sent until the user submits the form.
    pub(super) fn report_bug(&mut self, _: &ReportBug, _: &mut Window, cx: &mut Context<Self>) {
        let session = std::env::var("XDG_SESSION_TYPE")
            .map(|kind| format!(" ({kind})"))
            .unwrap_or_default();
        let system = format!(
            "{}, {} {}{session}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        cx.open_url(&format!(
            "{}/issues/new?template=bug.yml&version={}",
            env!("CARGO_PKG_REPOSITORY"),
            percent_encode(&system)
        ));
    }

    pub(super) fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.active_document().read(cx).editor().focus_handle(cx);
        let tabs: Vec<Tab> = self
            .documents
            .iter()
            .enumerate()
            .map(|(index, document)| {
                let doc = document.read(cx);
                let label = if doc.is_dirty() {
                    format!("● {}", doc.name())
                } else {
                    doc.name()
                };
                let tooltip = doc
                    .path()
                    .map_or_else(|| "Not saved yet".to_string(), |p| p.display().to_string());
                let document = document.clone();
                let focus = focus.clone();
                Tab::new()
                    // A right click selects the tab first: the menu then acts on it.
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, _, window, cx| this.activate(index, window, cx)),
                    )
                    .child(
                        div()
                            .id(("tab-label", index))
                            .child(label)
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(tooltip.clone())
                                    .build(window, cx)
                            })
                            .context_menu(move |menu, _, _| {
                                menu.action_context(focus.clone())
                                    .menu("Close", Box::new(CloseTab))
                                    .menu("Close Others", Box::new(CloseOtherTabs))
                                    .menu("Close to the Right", Box::new(CloseTabsToRight))
                                    .menu("Close Saved", Box::new(CloseSavedTabs))
                            }),
                    )
                    .suffix(
                        Button::new(("close-tab", index))
                            .ghost()
                            .xsmall()
                            .icon(IconName::Close)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.close(document.clone(), window, cx).detach()
                            })),
                    )
            })
            .collect();
        TabBar::new("documents")
            .prefix(div().px_1().child(self.app_menu(cx)))
            .selected_index(self.active)
            .on_click(
                cx.listener(|this, index: &usize, window, cx| this.activate(*index, window, cx)),
            )
            .children(tabs)
    }
}

/// A recent file as the menu shows it: its name, then its folder.
fn recent_label(path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let folder = path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    format!("{name} — {folder}")
}

/// Encode `text` for a URL query value (RFC 3986: unreserved characters stay as they are).
fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}
