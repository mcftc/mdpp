//! Tabs and files (PLAN §4.1, §6.1): the tab bar with the app menu and a menu per tab, closing
//! several tabs, reopening closed ones, save all, recent files, dropped files, the settings and
//! keymap files, and asking about unsaved changes when hot exit is off.

use std::path::{Path, PathBuf};

use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu};

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

    pub(super) fn open_settings(
        &mut self,
        _: &OpenSettings,
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

    /// The ≡ menu: every command with its shortcut, so nothing depends on knowing the keys.
    fn app_menu(&self, cx: &App) -> impl IntoElement {
        let focus = self.active_document().read(cx).editor().focus_handle(cx);
        let recent = self.recent.clone();
        let (outline, preview) = (self.show_outline, self.show_preview);
        let wrap = self.active_document().read(cx).soft_wrap();
        Button::new("app-menu")
            .icon(IconName::Menu)
            .ghost()
            .small()
            .tooltip("Menu")
            .dropdown_menu(move |menu, window, cx| {
                menu.action_context(focus.clone())
                    .menu("New", Box::new(NewFile))
                    .menu("Open…", Box::new(OpenFile))
                    .submenu("Open Recent", window, cx, {
                        let recent = recent.clone();
                        move |menu, _, _| recent_menu(menu, &recent)
                    })
                    .menu("Reopen Closed Tab", Box::new(ReopenClosedTab))
                    .separator()
                    .menu("Save", Box::new(Save))
                    .menu("Save As…", Box::new(SaveAs))
                    .menu("Save All", Box::new(SaveAll))
                    .menu("Export as HTML", Box::new(ExportHtml))
                    .separator()
                    .menu("Find", Box::new(Find))
                    .menu("Replace", Box::new(FindReplace))
                    .menu("Go to Line…", Box::new(GoToLine))
                    .submenu("Format", window, cx, |menu, _, _| {
                        menu.menu("Bold", Box::new(ToggleBold))
                            .menu("Italic", Box::new(ToggleItalic))
                            .menu("Code", Box::new(ToggleCode))
                            .menu("Strikethrough", Box::new(ToggleStrikethrough))
                            .menu("Link", Box::new(InsertLink))
                            .menu("Task", Box::new(ToggleTask))
                            .separator()
                            .menu("Heading Level +1", Box::new(AddHeadingLevel))
                            .menu("Heading Level −1", Box::new(RemoveHeadingLevel))
                    })
                    .submenu("Lines", window, cx, |menu, _, _| {
                        menu.menu("Duplicate", Box::new(DuplicateLines))
                            .menu("Delete", Box::new(DeleteLines))
                            .menu("Move Up", Box::new(MoveLinesUp))
                            .menu("Move Down", Box::new(MoveLinesDown))
                    })
                    .separator()
                    .menu_with_check("Outline", outline, Box::new(ToggleOutline))
                    .menu_with_check("Preview", preview, Box::new(TogglePreview))
                    .menu_with_check("Word Wrap", wrap, Box::new(ToggleWordWrap))
                    .menu("Zoom In", Box::new(ZoomIn))
                    .menu("Zoom Out", Box::new(ZoomOut))
                    .menu("Reset Zoom", Box::new(ZoomReset))
                    .separator()
                    .menu("Settings", Box::new(OpenSettings))
                    .menu("Keyboard Shortcuts", Box::new(OpenKeymap))
                    .separator()
                    .menu("Quit", Box::new(Quit))
            })
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

/// Recent files, most recent first, labelled by name with the folder after it.
fn recent_menu(menu: PopupMenu, recent: &[PathBuf]) -> PopupMenu {
    if recent.is_empty() {
        return menu.label("No recent files");
    }
    recent.iter().fold(menu, |menu, path| {
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let folder = path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        menu.menu(
            format!("{name} — {folder}"),
            Box::new(OpenRecent { path: path.clone() }),
        )
    })
}
