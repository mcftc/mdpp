//! Hot exit and session restore (PLAN §3.4).
//!
//! The session (tabs, cursors, which tabs hold unsaved text) is a small TOML file; the unsaved
//! text of each dirty document is a separate backup file. Both are written atomically, so a crash
//! at any moment leaves the previous complete state. Nothing here asks the user anything — the
//! app decides what to show; this module only stores and reads.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::text::Eol;

/// Bumped when the format changes incompatibly; older sessions are then ignored, not misread.
pub const SESSION_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub version: u32,
    /// Index of the active tab.
    pub active: usize,
    pub preview: bool,
    /// Outline sidebar open (absent in older sessions: closed).
    #[serde(default)]
    pub outline: bool,
    /// Editor text size relative to the `font_size` setting (Ctrl+= / Ctrl+-), in pixels.
    #[serde(default)]
    pub zoom: i32,
    /// Word wrap switched on or off with Alt+Z; `None` follows the settings.
    #[serde(default)]
    pub wrap: Option<bool>,
    /// Recently opened files, most recent first.
    #[serde(default)]
    pub recent: Vec<PathBuf>,
    /// The opened folder, and whether its panel was showing.
    #[serde(default)]
    pub folder: Option<PathBuf>,
    #[serde(default)]
    pub folder_panel: bool,
    #[serde(default, rename = "document")]
    pub documents: Vec<SessionDocument>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionDocument {
    /// `None` for an untitled document.
    pub path: Option<PathBuf>,
    /// Backup file (in the backup folder) holding unsaved text; `None` when the tab was clean.
    pub backup: Option<String>,
    pub cursor_line: u32,
    pub cursor_column: u32,
    pub eol: Eol,
    pub bom: bool,
    /// Encoding label (`UTF-8`, `windows-1254`, …); older sessions without it mean UTF-8.
    #[serde(default = "utf8_label")]
    pub encoding: String,
    /// The file on disk when the backup was taken — tells whether it changed while MD++ was closed.
    pub disk: Option<FileStamp>,
}

fn utf8_label() -> String {
    crate::text::UTF_8.name().to_string()
}

/// Size and modification time of a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileStamp {
    pub len: u64,
    pub modified_secs: u64,
    pub modified_nanos: u32,
}

impl FileStamp {
    pub fn of(path: &Path) -> Option<Self> {
        let meta = fs::metadata(path).ok()?;
        let since_epoch = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
        Some(FileStamp {
            len: meta.len(),
            modified_secs: since_epoch.as_secs(),
            modified_nanos: since_epoch.subsec_nanos(),
        })
    }
}

/// Where sessions and backups are kept: `<data>/session.toml` and `<data>/backup/`.
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Store {
            root: data_dir.into(),
        }
    }

    fn session_file(&self) -> PathBuf {
        self.root.join("session.toml")
    }

    fn backup_dir(&self) -> PathBuf {
        self.root.join("backup")
    }

    /// The last session, if there is a readable one of the current version.
    pub fn load(&self) -> Option<Session> {
        let text = fs::read_to_string(self.session_file()).ok()?;
        let session: Session = toml::from_str(&text).ok()?;
        (session.version == SESSION_VERSION).then_some(session)
    }

    pub fn save(&self, session: &Session) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        let text = toml::to_string(session).map_err(io::Error::other)?;
        crate::file::save(&self.session_file(), text.as_bytes()).map(drop)
    }

    pub fn write_backup(&self, name: &str, text: &str) -> io::Result<()> {
        let dir = self.backup_dir();
        fs::create_dir_all(&dir)?;
        crate::file::save(&dir.join(name), text.as_bytes()).map(drop)
    }

    pub fn read_backup(&self, name: &str) -> io::Result<String> {
        fs::read_to_string(self.backup_dir().join(name))
    }

    /// Delete a backup; a missing one is fine.
    pub fn remove_backup(&self, name: &str) {
        let _ = fs::remove_file(self.backup_dir().join(name));
    }

    /// A backup name no other document uses.
    pub fn new_backup_name() -> String {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        format!(
            "{nanos:x}-{:x}-{}.md",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store(name: &str) -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("mdpp-session-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        (Store::new(&dir), dir)
    }

    #[test]
    fn session_round_trip() {
        let (store, dir) = temp_store("roundtrip");
        assert_eq!(store.load(), None);
        let session = Session {
            version: SESSION_VERSION,
            active: 1,
            preview: true,
            outline: true,
            zoom: 2,
            wrap: Some(false),
            recent: vec![PathBuf::from("/n/a.md")],
            folder: Some(PathBuf::from("/notes")),
            folder_panel: true,
            documents: vec![
                SessionDocument {
                    path: Some(PathBuf::from("/notes/a.md")),
                    backup: None,
                    cursor_line: 3,
                    cursor_column: 7,
                    eol: Eol::CrLf,
                    bom: true,
                    encoding: "windows-1254".into(),
                    disk: Some(FileStamp {
                        len: 10,
                        modified_secs: 1,
                        modified_nanos: 2,
                    }),
                },
                SessionDocument {
                    path: None,
                    backup: Some(Store::new_backup_name()),
                    cursor_line: 0,
                    cursor_column: 0,
                    eol: Eol::Lf,
                    bom: false,
                    encoding: "UTF-8".into(),
                    disk: None,
                },
            ],
        };
        store.save(&session).unwrap();
        assert_eq!(store.load(), Some(session));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn other_versions_are_ignored() {
        let (store, dir) = temp_store("version");
        store
            .save(&Session {
                version: SESSION_VERSION + 1,
                ..Session::default()
            })
            .unwrap();
        assert_eq!(store.load(), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn backups() {
        let (store, dir) = temp_store("backup");
        let name = Store::new_backup_name();
        assert_ne!(name, Store::new_backup_name());
        store.write_backup(&name, "unsaved ğüş").unwrap();
        assert_eq!(store.read_backup(&name).unwrap(), "unsaved ğüş");
        store.remove_backup(&name);
        assert!(store.read_backup(&name).is_err());
        store.remove_backup(&name); // twice is fine
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn stamps_change_with_the_file() {
        let (_, dir) = temp_store("stamp");
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("f.md");
        fs::write(&file, "one").unwrap();
        let before = FileStamp::of(&file).unwrap();
        fs::write(&file, "three").unwrap();
        assert_ne!(FileStamp::of(&file).unwrap(), before);
        assert_eq!(FileStamp::of(&dir.join("missing.md")), None);
        let _ = fs::remove_dir_all(dir);
    }
}
