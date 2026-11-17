//! Key bindings as data (PLAN D9): `assets/default-keymap.toml` is embedded; a user's
//! `keymap.toml` is appended after it, so a later binding for the same keys wins. This module only
//! reads the files; turning names into UI actions is the app's job.

use serde::Deserialize;

pub const DEFAULT_KEYMAP: &str = include_str!("../../../assets/default-keymap.toml");

/// Context used when a binding does not name one: the main window.
pub const DEFAULT_CONTEXT: &str = "Workspace";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    /// Keystrokes, e.g. `secondary-shift-s` (`secondary` = Ctrl on Linux/Windows, Cmd on macOS).
    pub keys: String,
    /// Action name, e.g. `mdpp::Save`.
    pub action: String,
    /// Where the binding applies; defaults to [`DEFAULT_CONTEXT`].
    #[serde(default)]
    pub context: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeymapFile {
    #[serde(default)]
    binding: Vec<Binding>,
}

/// Default bindings followed by the user's; warnings describe a user file that could not be read.
pub fn load(user: Option<&str>) -> (Vec<Binding>, Vec<String>) {
    let mut bindings = parse(DEFAULT_KEYMAP).expect("assets/default-keymap.toml is valid (tested)");
    let mut warnings = Vec::new();
    if let Some(user) = user {
        match parse(user) {
            Ok(user) => bindings.extend(user),
            Err(error) => warnings.push(format!("keymap.toml ignored: {error}")),
        }
    }
    (bindings, warnings)
}

/// A first `keymap.toml` for the user: how to add a binding, then the defaults commented out.
pub fn user_template() -> String {
    format!(
        "# MD++ keyboard shortcuts. To change one, add a [[binding]] with the same action and new\n\
         # keys; yours come after the defaults, so they win. Restart MD++ after saving.\n\
         # `secondary` is Ctrl on Linux and Windows. Example:\n\
         #\n\
         # [[binding]]\n\
         # keys = \"secondary-shift-d\"\n\
         # action = \"mdpp::DuplicateLines\"\n\
         # context = \"MdEditor > Input\"\n\
         #\n\
         # The defaults:\n\n{}",
        crate::settings::comment_out(DEFAULT_KEYMAP)
    )
}

fn parse(text: &str) -> Result<Vec<Binding>, toml::de::Error> {
    toml::from_str::<KeymapFile>(text).map(|file| file.binding)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_parse() {
        let (bindings, warnings) = load(None);
        assert!(warnings.is_empty());
        assert!(bindings.iter().any(|b| b.action == "mdpp::Save"));
    }

    #[test]
    fn the_user_template_binds_nothing() {
        let (bindings, warnings) = load(Some(&user_template()));
        assert!(warnings.is_empty());
        assert_eq!(bindings, load(None).0);
    }

    #[test]
    fn user_bindings_come_last() {
        let user = "[[binding]]\nkeys = \"f2\"\naction = \"mdpp::Save\"\n";
        let (bindings, _) = load(Some(user));
        assert_eq!(bindings.last().unwrap().keys, "f2");
        let (_, warnings) = load(Some("[[binding]]\nkey = \"f2\"\n"));
        assert_eq!(warnings.len(), 1);
    }
}
