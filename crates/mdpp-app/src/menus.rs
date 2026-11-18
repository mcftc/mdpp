//! The menu bar behind ≡ (PLAN §6.1a), built from `assets/menus.toml`: the menus are data, and
//! each item names its action the way the keymap does, so the shortcut shown beside it is always
//! the one that works.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::LazyLock;

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::{Action, App, Context, Window};
use serde::Deserialize;

const MENUS: &str = include_str!("../../../assets/menus.toml");

/// `check` names the app knows; an item with one shows a check mark when it is on.
pub const CHECK_FOLDER: &str = "folder";
pub const CHECK_OUTLINE: &str = "outline";
pub const CHECK_PREVIEW: &str = "preview";
pub const CHECK_WORD_WRAP: &str = "word_wrap";
pub const CHECK_MENU_BAR: &str = "menu_bar";
const CHECKS: [&str; 5] = [
    CHECK_FOLDER,
    CHECK_OUTLINE,
    CHECK_PREVIEW,
    CHECK_WORD_WRAP,
    CHECK_MENU_BAR,
];
/// `list` names the app fills.
pub const LIST_RECENT: &str = "recent";
pub const LIST_SNIPPETS: &str = "snippets";
const LISTS: [&str; 2] = [LIST_RECENT, LIST_SNIPPETS];

#[derive(Debug, Deserialize)]
struct MenuFile {
    menu: Vec<MenuSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MenuSpec {
    name: String,
    items: Vec<ItemSpec>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ItemSpec {
    Action {
        label: String,
        action: String,
        #[serde(default)]
        check: Option<String>,
    },
    Submenu {
        submenu: String,
        #[serde(default)]
        items: Vec<ItemSpec>,
        #[serde(default)]
        list: Option<String>,
    },
    Separator {
        #[allow(dead_code, reason = "its presence marks the entry as a separator")]
        separator: bool,
    },
}

static SPEC: LazyLock<MenuFile> =
    LazyLock::new(|| toml::from_str(MENUS).expect("assets/menus.toml is valid (tested)"));

/// An entry of a generated list: its label and the action it runs.
pub type ListEntry = (String, Box<dyn Action>);

/// What the menus show from the app's state when they open.
#[derive(Default)]
pub struct MenuState {
    /// `check` names that are on.
    pub checked: Vec<&'static str>,
    /// Items of each `list` submenu: label and action.
    pub lists: HashMap<&'static str, Vec<ListEntry>>,
}

/// Names of the menus, in order.
pub fn names() -> impl Iterator<Item = &'static str> {
    SPEC.menu.iter().map(|menu| menu.name.as_str())
}

/// Fill `menu` with the items of menu number `index`.
pub fn fill(
    menu: PopupMenu,
    index: usize,
    state: &Rc<MenuState>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    match SPEC.menu.get(index) {
        Some(spec) => fill_items(menu, &spec.items, state, window, cx),
        None => menu,
    }
}

fn fill_items(
    menu: PopupMenu,
    items: &'static [ItemSpec],
    state: &Rc<MenuState>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    items.iter().fold(menu, |menu, item| match item {
        ItemSpec::Separator { .. } => menu.separator(),
        ItemSpec::Action {
            label,
            action,
            check,
        } => match cx.build_action(action, None) {
            Ok(action) => match check {
                Some(check) => {
                    let on = state.checked.contains(&check.as_str());
                    menu.menu_with_check(label.clone(), on, action)
                }
                None => menu.menu(label.clone(), action),
            },
            Err(_) => menu, // reported once at startup by `problems`
        },
        ItemSpec::Submenu {
            submenu,
            items,
            list,
        } => {
            let state = state.clone();
            menu.submenu(
                submenu.clone(),
                window,
                cx,
                move |menu, window, cx| match list {
                    Some(list) => {
                        let entries = state
                            .lists
                            .get(list.as_str())
                            .map_or(&[][..], Vec::as_slice);
                        if entries.is_empty() {
                            return menu.label("None");
                        }
                        entries.iter().fold(menu, |menu, (label, action)| {
                            menu.menu(label.clone(), action.boxed_clone())
                        })
                    }
                    None => fill_items(menu, items, &state, window, cx),
                },
            )
        }
    })
}

/// Problems with menus.toml: actions that do not exist (a typo or a renamed action), and check
/// or list names MD++ does not know. Reported at startup, like keymap problems.
pub fn problems(cx: &App) -> Vec<String> {
    let mut out = name_problems();
    for item in all_items() {
        if let ItemSpec::Action { action, .. } = item
            && cx.build_action(action, None).is_err()
        {
            out.push(format!("menus: unknown action `{action}`"));
        }
    }
    out
}

/// Every action name the menus use.
pub fn actions() -> impl Iterator<Item = &'static str> {
    all_items().into_iter().filter_map(|item| match item {
        ItemSpec::Action { action, .. } => Some(action.as_str()),
        _ => None,
    })
}

/// A command as the palette lists it: its menu label, where it sits in the menus, its action.
pub struct MenuCommand {
    pub label: &'static str,
    pub place: String,
    pub action: &'static str,
}

/// Every menu command once (the first place it appears), in menu order — the palette's list.
pub fn commands() -> Vec<MenuCommand> {
    fn walk(items: &'static [ItemSpec], place: &str, out: &mut Vec<MenuCommand>) {
        for item in items {
            match item {
                ItemSpec::Action { label, action, .. } => {
                    if !out.iter().any(|c| c.action == action) {
                        out.push(MenuCommand {
                            label,
                            place: place.to_string(),
                            action,
                        });
                    }
                }
                ItemSpec::Submenu { submenu, items, .. } => {
                    walk(items, &format!("{place} › {submenu}"), out)
                }
                ItemSpec::Separator { .. } => {}
            }
        }
    }
    let mut out = Vec::new();
    for menu in &SPEC.menu {
        walk(&menu.items, &menu.name, &mut out);
    }
    out
}

fn name_problems() -> Vec<String> {
    all_items()
        .into_iter()
        .filter_map(|item| match item {
            ItemSpec::Action {
                check: Some(check), ..
            } if !CHECKS.contains(&check.as_str()) => {
                Some(format!("menus: unknown check `{check}`"))
            }
            ItemSpec::Submenu {
                list: Some(list), ..
            } if !LISTS.contains(&list.as_str()) => Some(format!("menus: unknown list `{list}`")),
            _ => None,
        })
        .collect()
}

fn all_items() -> Vec<&'static ItemSpec> {
    fn walk<'a>(items: &'a [ItemSpec], out: &mut Vec<&'a ItemSpec>) {
        for item in items {
            out.push(item);
            if let ItemSpec::Submenu { items, .. } = item {
                walk(items, out);
            }
        }
    }
    let mut out = Vec::new();
    for menu in &SPEC.menu {
        walk(&menu.items, &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_file_parses_and_names_known_state() {
        let items = all_items();
        assert!(items.len() > 30 && names().count() >= 5);
        assert_eq!(name_problems(), Vec::<String>::new());
        let commands = commands();
        let bold = commands.iter().find(|c| c.label == "Bold").unwrap();
        assert_eq!(bold.place, "Edit › Format");
        let shortcuts = commands.iter().filter(|c| c.action == "mdpp::OpenKeymap");
        assert_eq!(
            shortcuts.count(),
            1,
            "listed once although two menus have it"
        );
        for item in items {
            if let ItemSpec::Action { action, .. } = item {
                assert!(action.contains("::"), "{action}: namespace::Action");
            }
        }
    }
}
