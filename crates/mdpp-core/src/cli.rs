//! Command-line files: `mdpp notes.md`, `mdpp notes.md:42`, `mdpp notes.md:42:7` (PLAN §4.1).
//! `:line:col` is read from the end, so Windows paths (`C:\docs\a.md:3`) stay intact, and a file
//! whose name really ends in `:12` wins over the line suffix.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A file to open, with an optional 1-based position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenRequest {
    /// Absolute, so another instance (with another working directory) opens the same file.
    pub path: PathBuf,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

/// Parse file arguments; relative paths are resolved against `cwd`.
pub fn parse_args(args: impl IntoIterator<Item = OsString>, cwd: &Path) -> Vec<OpenRequest> {
    args.into_iter().map(|arg| parse_one(arg, cwd)).collect()
}

fn parse_one(arg: OsString, cwd: &Path) -> OpenRequest {
    let absolute = |path: &Path| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        }
    };
    let as_given = PathBuf::from(&arg);
    let plain = OpenRequest {
        path: absolute(&as_given),
        line: None,
        column: None,
    };
    let Some(text) = arg.to_str() else {
        return plain;
    };
    if absolute(&as_given).exists() {
        return plain;
    }
    let number = positive;
    let Some((rest, last)) = text
        .rsplit_once(':')
        .and_then(|(r, l)| Some((r, number(l)?)))
    else {
        return plain;
    };
    let (path, line, column) = match rest
        .rsplit_once(':')
        .and_then(|(r, l)| Some((r, number(l)?)))
    {
        Some((path, line)) => (path, line, Some(last)),
        None => (rest, last, None),
    };
    // A lone drive letter is not a file ("C:12" is not a path with a line).
    if path.is_empty() || (path.len() == 1 && path.chars().all(|c| c.is_ascii_alphabetic())) {
        return plain;
    }
    OpenRequest {
        path: absolute(Path::new(path)),
        line: Some(line),
        column,
    }
}

fn positive(s: &str) -> Option<u32> {
    s.trim().parse::<u32>().ok().filter(|&n| n > 0)
}

/// A position typed by the user: `line` or `line:column`, both counted from 1.
pub fn parse_line_column(text: &str) -> Option<(u32, Option<u32>)> {
    match text.split_once([':', ',']) {
        Some((line, column)) => Some((positive(line)?, Some(positive(column)?))),
        None => Some((positive(text)?, None)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arg: &str) -> OpenRequest {
        parse_one(OsString::from(arg), Path::new("/work"))
    }

    #[test]
    fn positions() {
        assert_eq!(
            parse("a.md"),
            OpenRequest {
                path: "/work/a.md".into(),
                line: None,
                column: None
            }
        );
        assert_eq!(
            parse("a.md:12"),
            OpenRequest {
                path: "/work/a.md".into(),
                line: Some(12),
                column: None
            }
        );
        assert_eq!(
            parse("/x/a.md:12:7"),
            OpenRequest {
                path: "/x/a.md".into(),
                line: Some(12),
                column: Some(7)
            }
        );
        assert_eq!(parse("a.md:0").line, None); // lines are 1-based
        assert_eq!(
            parse("notes:draft.md").path,
            PathBuf::from("/work/notes:draft.md")
        );
    }

    #[test]
    fn windows_drive_letters_stay_intact() {
        let request = parse(r"C:\docs\a.md:3");
        assert_eq!(
            (request.path, request.line),
            (PathBuf::from("/work/C:\\docs\\a.md"), Some(3))
        );
        assert_eq!(parse("C:12").line, None);
    }

    #[test]
    fn existing_files_win_over_suffixes() {
        let dir = std::env::temp_dir().join(format!("mdpp-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let odd = dir.join("odd.md:12");
        std::fs::write(&odd, "").unwrap();
        let request = parse_one(odd.clone().into_os_string(), &dir);
        assert_eq!((request.path, request.line), (odd, None));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typed_positions() {
        assert_eq!(parse_line_column("12"), Some((12, None)));
        assert_eq!(parse_line_column(" 12:5 "), Some((12, Some(5))));
        assert_eq!(parse_line_column("3,4"), Some((3, Some(4))));
        assert_eq!(parse_line_column("0"), None);
        assert_eq!(parse_line_column("12:"), None);
        assert_eq!(parse_line_column("x"), None);
    }
}
