//! The app's own storage: the `data/` directory and nothing else.
//!
//! The app runs chrooted into its own install directory, and `data/` inside it is the only place
//! it may write. **Paths are relative**, and that is not a style point: `data/state.json` is the
//! file the app can reach, `/data/state.json` is a path on a filesystem the app cannot see.
//!
//! ```no_run
//! use nest_app_sdk::Store;
//! use serde_json::json;
//!
//! let store = Store::new();
//! let state = store.load("state.json").unwrap_or_else(|| json!({"mode": 0}));
//! let _ = store.save("state.json", &state);
//! ```
//!
//! `data/` belongs to the app's UID and survives upgrades: installing a new version replaces the
//! binary and the manifest and leaves `data/` alone. A file is at most 1 MiB
//! (`RLIMIT_FSIZE`), and writes are small enough for jffs2 — save when something
//! changed, not on every frame.

use std::fs;
use std::io::{self, Error, ErrorKind, Write as _};
use std::path::PathBuf;

use serde_json::Value;

/// The app's writable directory, relative to its install root.
pub const DIR: &str = "data";

/// A directory of JSON files, `data/` by default.
#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Default for Store {
    fn default() -> Self {
        Self::new()
    }
}

impl Store {
    /// The app's own `data/` directory.
    pub fn new() -> Self {
        Self {
            root: PathBuf::from(DIR),
        }
    }

    /// A store somewhere else — for tests, where writing into the checkout would be rude.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Where `name` lives.
    pub fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// The contents of `name`, or `None`.
    ///
    /// A missing file, an unreadable one and one that is not JSON all read as `None`: callers get
    /// their default and carry on. That is what makes `data/` safe to be empty on a fresh install.
    pub fn load(&self, name: &str) -> Option<Value> {
        if check(name).is_err() {
            return None;
        }
        let bytes = fs::read(self.path(name)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// Write `value` to `name`, atomically.
    ///
    /// The bytes go to `name.tmp`, are synced, and are then renamed over `name`, so a power cut
    /// during a save leaves either the old state or the new one, never half of a JSON document.
    /// The name may not contain a path separator: an app cannot escape `data/` by accident.
    pub fn save(&self, name: &str, value: &Value) -> io::Result<()> {
        check(name)?;
        let bytes = serde_json::to_vec(value).map_err(|e| Error::new(ErrorKind::InvalidData, e))?;
        fs::create_dir_all(&self.root)?;
        let tmp = self.path(&format!("{name}.tmp"));
        let mut file = fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, self.path(name))
    }
}

/// A file name that stays inside the store's directory.
fn check(name: &str) -> io::Result<()> {
    let allowed = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if name.is_empty() || name.len() > 64 || name == "." || name == ".." || !allowed {
        return Err(Error::new(ErrorKind::InvalidInput, format!("bad file name {name:?}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A store in a private temporary directory, and that directory.
    fn temp(name: &str) -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!("nest-app-sdk-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        (Store::at(&dir), dir)
    }

    #[test]
    fn the_default_root_is_the_relative_data_directory() {
        assert_eq!(Store::new().path("state.json"), PathBuf::from("data/state.json"));
        assert_eq!(Store::default().path("state.json"), PathBuf::from("data/state.json"));
    }

    #[test]
    fn round_trips_a_value() {
        let (store, dir) = temp("round-trip");
        assert_eq!(store.load("state.json"), None, "nothing is there yet");
        let state = json!({"mode": 2, "value": 7.5, "names": ["a", "b"]});
        store.save("state.json", &state).expect("save");
        assert!(dir.join("state.json").is_file());
        assert_eq!(store.load("state.json"), Some(state));
        // The temporary file is gone: only the saved one is left.
        assert_eq!(fs::read_dir(&dir).expect("read_dir").count(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn overwrites_in_place() {
        let (store, dir) = temp("overwrite");
        store.save("s.json", &json!({"n": 1})).expect("save");
        store.save("s.json", &json!({"n": 2})).expect("save");
        assert_eq!(store.load("s.json"), Some(json!({"n": 2})));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_corrupt_and_unwritable_all_come_back_as_none() {
        let (store, dir) = temp("corrupt");
        assert_eq!(store.load("never.json"), None);
        fs::create_dir_all(&dir).expect("mkdir");
        fs::write(dir.join("half.json"), b"{\"mode\": ").expect("write");
        assert_eq!(store.load("half.json"), None, "half a document is not a document");
        fs::write(dir.join("empty.json"), b"").expect("write");
        assert_eq!(store.load("empty.json"), None);
        // A directory is not a document either.
        fs::create_dir_all(dir.join("dir.json")).expect("mkdir");
        assert_eq!(store.load("dir.json"), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn names_cannot_escape_the_data_directory() {
        let (store, dir) = temp("escape");
        for name in ["../x", "a/b", "/etc/passwd", ".", "..", "", "x\0y", &"n".repeat(65)] {
            assert!(store.save(name, &json!(1)).is_err(), "{name:?} was accepted");
            assert_eq!(store.load(name), None);
        }
        assert!(!dir.exists(), "nothing was created for a bad name");
        for name in ["state.json", "a-b_c.2.json", "X"] {
            store.save(name, &json!(1)).expect("a plain name is fine");
        }
        let _ = fs::remove_dir_all(dir);
    }
}
