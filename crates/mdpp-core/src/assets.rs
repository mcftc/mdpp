//! Where pasted and dropped images go (PLAN §4.3 "Image paste / drop"): a folder next to the
//! document named by the `assets_dir_template` setting, e.g. `assets/{doc_stem}` → `assets/notes`
//! for `notes.md`. Existing files are never overwritten.

use std::path::{Component, Path, PathBuf};

/// File name (without extension) of a pasted image: `image.png`, `image-2.png`, …
pub const PASTED_IMAGE_STEM: &str = "image";
/// Placeholder in `assets_dir_template` for the document's file name without extension.
const DOC_STEM: &str = "{doc_stem}";

/// The image folder for document `doc`: absolute, and as a link relative to the document
/// (`/`-separated, empty when it is the document's own folder). `None` when the template
/// leaves the document's folder (`..` or an absolute path) — images stay next to the text.
pub fn image_folder(doc: &Path, template: &str) -> Option<(PathBuf, String)> {
    let stem = doc.file_stem()?.to_string_lossy();
    let relative = template.replace(DOC_STEM, &stem);
    let relative = relative.trim_matches(['/', '\\']);
    let path = Path::new(relative);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return None;
    }
    let link = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/");
    Some((doc.parent()?.join(path), link))
}

/// A file name in `folder` that is not taken: `stem.ext`, else `stem-2.ext`, `stem-3.ext`, …
pub fn free_name(folder: &Path, stem: &str, extension: &str) -> String {
    let name = |n: usize| match n {
        1 => format!("{stem}.{extension}"),
        n => format!("{stem}-{n}.{extension}"),
    };
    (1..)
        .map(name)
        .find(|candidate| !folder.join(candidate).exists())
        .expect("an unbounded range finds a free name")
}

/// `path` as a link from `doc`'s folder (`/`-separated), when it is inside that folder.
pub fn link_from(doc: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(doc.parent()?).ok()?;
    Some(
        relative
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folders_from_the_template() {
        let doc = Path::new("/notes/trip plan.md");
        assert_eq!(
            image_folder(doc, "assets/{doc_stem}"),
            Some((
                PathBuf::from("/notes/assets/trip plan"),
                "assets/trip plan".into()
            ))
        );
        assert_eq!(
            image_folder(doc, "./img/"),
            Some((PathBuf::from("/notes/./img"), "img".into()))
        );
        assert_eq!(
            image_folder(doc, ""),
            Some((PathBuf::from("/notes/"), String::new()))
        );
        assert_eq!(image_folder(doc, "../shared"), None);
        assert_eq!(
            link_from(doc, Path::new("/notes/pics/a.png")),
            Some("pics/a.png".into())
        );
        assert_eq!(link_from(doc, Path::new("/elsewhere/a.png")), None);
    }

    #[test]
    fn free_names_never_overwrite() {
        let dir = std::env::temp_dir().join(format!("mdpp-assets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_name(&dir, "image", "png"), "image.png");
        std::fs::write(dir.join("image.png"), b"x").unwrap();
        std::fs::write(dir.join("image-2.png"), b"x").unwrap();
        assert_eq!(free_name(&dir, "image", "png"), "image-3.png");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
