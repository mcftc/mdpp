//! An opened folder (PLAN §4.1): the folder panel on the left, its files in quick open and in
//! find in files. Listed in the background (`mdpp_core::folder`), again when the window comes
//! back to the front, so files made by other programs show up without a watcher on every folder.

use std::collections::HashSet;

use gpui_kit::component::Selectable as _;
use std::sync::Arc;

use mdpp_core::folder::{self, Listing, Tree};

use super::*;
use mdpp_core::i18n::{t, tf};

/// Indentation of one tree level and the panel's padding, in logical pixels.
const TREE_INDENT: f32 = 12.0;
const TREE_PADDING: f32 = 8.0;

/// What the left panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Side {
    Folder,
    Search,
}

pub(super) struct FolderState {
    pub root: PathBuf,
    pub listing: Arc<Listing>,
    tree: Arc<Tree>,
    expanded: HashSet<PathBuf>,
    /// Rows shown, as indices into the tree.
    visible: Arc<Vec<usize>>,
    _scan: Task<()>,
}

impl FolderState {
    /// The folder's files, absolute.
    pub fn files(&self) -> impl Iterator<Item = PathBuf> + '_ {
        self.listing.files.iter().map(|f| self.root.join(f))
    }
}

impl Workspace {
    /// Open `root` as the folder (replacing the previous one) and list it.
    pub(super) fn open_folder(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        self.folder = Some(FolderState {
            root,
            listing: Arc::default(),
            tree: Arc::default(),
            expanded: HashSet::new(),
            visible: Arc::default(),
            _scan: Task::ready(()),
        });
        self.show_side = true;
        self.scan_folder(cx);
    }

    /// List the folder again in the background.
    pub(super) fn scan_folder(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.folder.as_mut() else {
            return;
        };
        let root = open.root.clone();
        let max = self.settings.workspace_max_files;
        open._scan = cx.spawn(async move |this, cx| {
            let listing = cx
                .background_spawn(async move { folder::list(&root, max) })
                .await;
            this.update(cx, |this, cx| {
                let Some(open) = this.folder.as_mut() else {
                    return;
                };
                if *open.listing == listing {
                    return;
                }
                open.tree = Arc::new(Tree::new(&listing));
                open.listing = Arc::new(listing);
                open.visible = Arc::new(open.tree.visible(&open.expanded));
                cx.notify();
            })
            .ok();
        });
    }

    pub(super) fn open_folder_dialog(
        &mut self,
        _: &OpenFolder,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = chosen.await
                && let Some(path) = paths.pop()
            {
                this.update(cx, |this, cx| this.open_folder(path, cx)).ok();
            }
        })
        .detach();
    }

    pub(super) fn close_folder(&mut self, _: &CloseFolder, _: &mut Window, cx: &mut Context<Self>) {
        self.folder = None;
        cx.notify();
    }

    /// The left panel with the folder; again, it closes. With no folder open: choose one.
    pub(super) fn toggle_folder_panel(
        &mut self,
        _: &ToggleFolderPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.folder.is_none() {
            return self.open_folder_dialog(&OpenFolder, window, cx);
        }
        self.show_side = !(self.show_side && self.side == Side::Folder);
        self.side = Side::Folder;
        cx.notify();
    }

    /// The left panel with find in files, starting from the selection when it is in one line.
    pub(super) fn find_in_files(
        &mut self,
        _: &FindInFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_side = true;
        self.side = Side::Search;
        let editor = self.active_document().read(cx).editor().clone();
        let selected = crate::find::selected_line(&editor, cx);
        self.search_panel
            .update(cx, |panel, cx| panel.focus(selected, window, cx));
        cx.notify();
    }

    /// What find in files searches: the folder's files, or the open tabs' files without one.
    pub(super) fn searched_files(&self, cx: &App) -> Vec<PathBuf> {
        match &self.folder {
            Some(folder) => folder.files().collect(),
            None => self
                .documents
                .iter()
                .filter_map(|d| d.read(cx).path().map(std::path::Path::to_path_buf))
                .collect(),
        }
    }

    /// The left panel: Folder | Search, then the chosen view.
    pub(super) fn render_side(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.show_side {
            return None;
        }
        let content = match self.side {
            Side::Folder => self.render_folder(cx)?,
            Side::Search => self.search_panel.clone().into_any_element(),
        };
        let tab = |id: &'static str, label: &'static str, side: Side| {
            Button::new(id)
                .label(t(label))
                .ghost()
                .xsmall()
                .selected(self.side == side)
        };
        let tabs = h_flex()
            .gap_1()
            .px_1()
            .py_0p5()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                tab("side-folder", "Folder", Side::Folder).on_click(cx.listener(
                    |this, _, window, cx| {
                        if this.side != Side::Folder || this.folder.is_none() {
                            this.toggle_folder_panel(&ToggleFolderPanel, window, cx)
                        }
                    },
                )),
            )
            .child(tab("side-search", "Search", Side::Search).on_click(
                cx.listener(|this, _, window, cx| this.find_in_files(&FindInFiles, window, cx)),
            ))
            .child(div().flex_1())
            .child(
                Button::new("side-close")
                    .icon(IconName::Close)
                    .ghost()
                    .xsmall()
                    .tooltip(t("Close the panel"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_side = false;
                        cx.notify();
                    })),
            );
        Some(
            v_flex()
                .size_full()
                .bg(cx.theme().sidebar)
                .child(tabs)
                .child(div().flex_1().min_h_0().child(content))
                .into_any_element(),
        )
    }

    fn toggle_tree_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        if let Some(open) = self.folder.as_mut() {
            if !open.expanded.remove(&path) {
                open.expanded.insert(path);
            }
            open.visible = Arc::new(open.tree.visible(&open.expanded));
            cx.notify();
        }
    }

    /// The folder panel: the folder's name, then its tree (virtualized; folders open on click).
    pub(super) fn render_folder(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let open = self.folder.as_ref()?;
        let theme = cx.theme();
        let name = open.root.file_name().map_or_else(
            || open.root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        let active = self
            .active_document()
            .read(cx)
            .path()
            .map(std::path::Path::to_path_buf);
        let (tree, visible, root) = (open.tree.clone(), open.visible.clone(), open.root.clone());
        let expanded = open.expanded.clone();
        let header = h_flex()
            .px_2()
            .py_1()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name.to_uppercase()),
            )
            .when(open.listing.truncated, |row| {
                row.child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(tf("first {count}", &[("count", &open.listing.files.len())])),
                )
            })
            .child(
                Button::new("refresh-folder")
                    .icon(IconName::RefreshCw)
                    .ghost()
                    .xsmall()
                    .tooltip(t("List the folder again"))
                    .on_click(cx.listener(|this, _, _, cx| this.scan_folder(cx))),
            );
        let list = uniform_list(
            "folder-tree",
            visible.len(),
            cx.processor(move |_, range: std::ops::Range<usize>, _window, cx| {
                let theme = cx.theme();
                let (active_bg, hover_bg, muted) =
                    (theme.accent, theme.secondary, theme.muted_foreground);
                range
                    .map(|row| {
                        let node = &tree.nodes[visible[row]];
                        let path = node.path.clone();
                        let is_folder = node.is_folder;
                        let absolute = root.join(&node.path);
                        let current = !is_folder && active.as_ref() == Some(&absolute);
                        let chevron = is_folder.then(|| {
                            if expanded.contains(&node.path) {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            }
                        });
                        h_flex()
                            .id(row)
                            .w_full()
                            .gap_1()
                            .pl(px(TREE_INDENT * node.depth as f32 + TREE_PADDING))
                            .pr_2()
                            .py_0p5()
                            .text_sm()
                            .truncate()
                            .cursor_pointer()
                            .when(current, |row| row.bg(active_bg))
                            .when(is_folder, |row| row.text_color(muted))
                            .hover(move |row| row.bg(hover_bg))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if is_folder {
                                    this.toggle_tree_folder(path.clone(), cx)
                                } else {
                                    this.open_paths(vec![absolute.clone()], window, cx)
                                }
                            }))
                            .child(div().flex_shrink_0().size(px(TREE_INDENT)).children(
                                chevron.map(|icon| gpui_kit::component::Icon::new(icon).xsmall()),
                            ))
                            .child(div().truncate().child(node.name.clone()))
                    })
                    .collect()
            }),
        )
        .flex_1()
        .py_1();
        Some(
            v_flex()
                .size_full()
                .bg(theme.sidebar)
                .child(header)
                .child(list)
                .into_any_element(),
        )
    }
}
