//! An opened folder (PLAN §4.3 "workspace"): `mdpp <folder>` or File → Open Folder. Only a folder
//! the user opened is listed, recursively — never the folder of a lone file — skipping what Git
//! ignores and hidden entries, up to `workspace_max_files` files.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The files under a folder, relative to it, sorted.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub files: Vec<PathBuf>,
    /// More files than the cap: the rest were left out.
    pub truncated: bool,
}

/// List the files under `root` (relative paths), honouring `.gitignore`/`.ignore` files and
/// skipping hidden entries. Unreadable entries are skipped.
pub fn list(root: &Path, max_files: usize) -> Listing {
    let mut files = Vec::new();
    let mut truncated = false;
    let walk = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .require_git(false)
        .follow_links(false)
        .build();
    for entry in walk.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        if files.len() == max_files {
            truncated = true;
            break;
        }
        if let Ok(relative) = entry.path().strip_prefix(root) {
            files.push(relative.to_path_buf());
        }
    }
    files.sort_by(|a, b| compare_paths(a, b));
    Listing { files, truncated }
}

/// Folders before files at every level, then by name ignoring case — the order of the tree.
fn compare_paths(a: &Path, b: &Path) -> Ordering {
    let (mut a, mut b) = (a.components().peekable(), b.components().peekable());
    loop {
        match (a.next(), b.next()) {
            (Some(x), Some(y)) if x == y => continue,
            (Some(x), Some(y)) => {
                // A component with more after it is a folder.
                let (x_dir, y_dir) = (a.peek().is_some(), b.peek().is_some());
                return y_dir.cmp(&x_dir).then_with(|| {
                    let (x, y) = (
                        x.as_os_str().to_string_lossy(),
                        y.as_os_str().to_string_lossy(),
                    );
                    x.to_lowercase().cmp(&y.to_lowercase()).then(x.cmp(&y))
                });
            }
            (x, y) => return x.is_some().cmp(&y.is_some()),
        }
    }
}

/// One row of the folder tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// Relative to the folder.
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_folder: bool,
}

/// The folder tree in display order (each folder followed by its contents).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    pub nodes: Vec<Node>,
}

impl Tree {
    /// Build from a sorted listing.
    pub fn new(listing: &Listing) -> Self {
        let mut nodes = Vec::new();
        let mut open: Vec<&std::ffi::OsStr> = Vec::new(); // folders of the previous file
        for file in &listing.files {
            let parts: Vec<_> = file.iter().collect();
            let (folders, name) = parts.split_at(parts.len() - 1);
            let shared = open.iter().zip(folders).take_while(|(a, b)| a == b).count();
            for (depth, folder) in folders.iter().enumerate().skip(shared) {
                nodes.push(Node {
                    path: folders[..=depth].iter().collect(),
                    name: folder.to_string_lossy().into_owned(),
                    depth,
                    is_folder: true,
                });
            }
            nodes.push(Node {
                path: file.clone(),
                name: name[0].to_string_lossy().into_owned(),
                depth: folders.len(),
                is_folder: false,
            });
            open = folders.to_vec();
        }
        Self { nodes }
    }

    /// Indices of the rows shown when only the folders in `expanded` are open.
    pub fn visible(&self, expanded: &HashSet<PathBuf>) -> Vec<usize> {
        let mut out = Vec::new();
        let mut hidden_below: Option<usize> = None; // depth of a closed folder
        for (index, node) in self.nodes.iter().enumerate() {
            if hidden_below.is_some_and(|depth| node.depth > depth) {
                continue;
            }
            hidden_below = (node.is_folder && !expanded.contains(&node.path)).then_some(node.depth);
            out.push(index);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mdpp-folder-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn lists_files_folders_first_without_ignored_or_hidden() {
        let root = scratch("list");
        for file in [
            "b.md",
            "A.md",
            "docs/z.md",
            "docs/img/p.png",
            "target/out.md",
            ".hidden/x.md",
        ] {
            let path = root.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        let listing = list(&root, 100);
        let names: Vec<_> = listing
            .files
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        assert_eq!(names, ["docs/img/p.png", "docs/z.md", "A.md", "b.md"]);
        assert!(!listing.truncated);
        let capped = list(&root, 2);
        assert!(capped.truncated && capped.files.len() == 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn tree_rows_follow_open_folders() {
        let listing = Listing {
            files: ["docs/img/p.png", "docs/z.md", "notes/n.md", "A.md"]
                .map(PathBuf::from)
                .to_vec(),
            truncated: false,
        };
        let tree = Tree::new(&listing);
        let shown = |open: &[&str]| -> Vec<String> {
            let open = open.iter().map(PathBuf::from).collect();
            tree.visible(&open)
                .into_iter()
                .map(|i| format!("{}{}", "  ".repeat(tree.nodes[i].depth), tree.nodes[i].name))
                .collect()
        };
        assert_eq!(shown(&[]), ["docs", "notes", "A.md"]);
        assert_eq!(
            shown(&["docs"]),
            ["docs", "  img", "  z.md", "notes", "A.md"]
        );
        assert_eq!(
            shown(&["docs", "docs/img", "notes"]),
            [
                "docs",
                "  img",
                "    p.png",
                "  z.md",
                "notes",
                "  n.md",
                "A.md"
            ]
        );
        // A closed parent hides an open child.
        assert_eq!(shown(&["docs/img"]), ["docs", "notes", "A.md"]);
    }
}
