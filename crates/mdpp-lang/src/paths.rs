//! File-system access for path completion and link checking, behind a trait so the engine is
//! testable without touching the disk.

use std::path::Path;

/// Directory listings larger than this are truncated: path completion must stay instant even in
/// folders with huge numbers of files.
pub const MAX_DIR_ENTRIES: usize = 2_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

pub trait PathSource {
    /// Entries of `dir`, non-recursive; empty if it cannot be read.
    fn list(&self, dir: &Path) -> Vec<DirEntry>;
    fn exists(&self, path: &Path) -> bool;
}

/// The real file system.
#[derive(Clone, Copy, Debug, Default)]
pub struct FileSystem;

impl PathSource for FileSystem {
    fn list(&self, dir: &Path) -> Vec<DirEntry> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .take(MAX_DIR_ENTRIES)
            .map(|entry| DirEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                is_dir: entry.file_type().is_ok_and(|t| t.is_dir()),
            })
            .collect()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}

/// A path relative to the document: no URL scheme, not absolute, not a Windows drive path.
pub(crate) fn is_relative_path(dest: &str) -> bool {
    if dest.is_empty() || dest.starts_with(['/', '\\', '#']) {
        return false;
    }
    let bytes = dest.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return false; // C:\…
    }
    let scheme_len = dest.find(':').filter(|&i| {
        dest[..i]
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    });
    scheme_len.is_none()
}

/// A `file://` URI for an absolute path, with an optional `#fragment`.
pub fn file_uri(path: &Path, fragment: Option<&str>) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }; // C:/… → /C:/…
    let encode = |text: &str| {
        text.bytes()
            .map(|b| match b {
                b'A'..=b'Z'
                | b'a'..=b'z'
                | b'0'..=b'9'
                | b'-'
                | b'.'
                | b'_'
                | b'~'
                | b'/'
                | b':' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect::<String>()
    };
    match fragment {
        Some(fragment) => format!("file://{}#{}", encode(&path), encode(fragment)),
        None => format!("file://{}", encode(&path)),
    }
}

/// The path and fragment of a `file://` URI made by [`file_uri`].
pub fn path_from_file_uri(uri: &str) -> Option<(std::path::PathBuf, Option<String>)> {
    let rest = uri.strip_prefix("file://")?;
    let (path, fragment) = match rest.split_once('#') {
        Some((path, fragment)) => (path, Some(percent_decode(fragment))),
        None => (rest, None),
    };
    let path = percent_decode(path);
    // /C:/x → C:/x on Windows.
    let bytes = path.as_bytes();
    let path = if bytes.len() > 2 && bytes[0] == b'/' && bytes[2] == b':' {
        &path[1..]
    } else {
        &path
    };
    Some((std::path::PathBuf::from(path), fragment))
}

/// Decode `%XX` escapes in a link destination (`my%20file.md` → `my file.md`).
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2]))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Encode what a link destination cannot contain unescaped.
pub(crate) fn percent_encode_path(s: &str) -> String {
    s.replace('%', "%25")
        .replace(' ', "%20")
        .replace('(', "%28")
        .replace(')', "%29")
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod fake {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// In-memory tree: path → is_dir.
    #[derive(Default)]
    pub struct FakeFs(pub BTreeMap<PathBuf, bool>);

    impl FakeFs {
        pub fn with(paths: &[&str]) -> Self {
            let mut map = BTreeMap::new();
            for p in paths {
                let is_dir = p.ends_with('/');
                let path = PathBuf::from(p.trim_end_matches('/'));
                for ancestor in path.ancestors().skip(1) {
                    if !ancestor.as_os_str().is_empty() {
                        map.insert(ancestor.to_path_buf(), true);
                    }
                }
                map.insert(path, is_dir);
            }
            FakeFs(map)
        }
    }

    impl PathSource for FakeFs {
        fn list(&self, dir: &Path) -> Vec<DirEntry> {
            self.0
                .iter()
                .filter(|(p, _)| p.parent() == Some(dir))
                .map(|(p, &is_dir)| DirEntry {
                    name: p.file_name().unwrap().to_string_lossy().into_owned(),
                    is_dir,
                })
                .collect()
        }

        fn exists(&self, path: &Path) -> bool {
            self.0.contains_key(path)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_round_trip() {
        let path = Path::new("/home/me/my notes/ğüş.md");
        let uri = file_uri(path, Some("set up"));
        assert_eq!(
            uri,
            "file:///home/me/my%20notes/%C4%9F%C3%BC%C5%9F.md#set%20up"
        );
        assert_eq!(
            path_from_file_uri(&uri),
            Some((path.to_path_buf(), Some("set up".into())))
        );
        assert_eq!(
            file_uri(Path::new("C:\\d\\a.md"), None),
            "file:///C:/d/a.md"
        );
        assert_eq!(
            path_from_file_uri("file:///C:/d/a.md").unwrap().0,
            Path::new("C:/d/a.md")
        );
        assert_eq!(path_from_file_uri("https://x"), None);
    }

    #[test]
    fn relative_paths() {
        assert!(is_relative_path("a/b.md"));
        assert!(is_relative_path("../x.png"));
        assert!(!is_relative_path("https://x"));
        assert!(!is_relative_path("C:\\x.md"));
        assert!(!is_relative_path("/abs.md"));
    }

    #[test]
    fn percent_round_trip() {
        assert_eq!(percent_decode("my%20file%28v2%29.md"), "my file(v2).md");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(
            percent_encode_path("my file(v2).md"),
            "my%20file%28v2%29.md"
        );
    }
}
