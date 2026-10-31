//! XDG base-directory resolution and private file and directory creation.
//!
//! Resolving a path never touches the filesystem. Directories and files are created only by
//! [`ensure_private_dir`], [`open_private_append`] and [`ensure_private_file`], and only when a
//! caller explicitly asks for it. Checks run on open descriptors, never "check then open by name".

use std::env;
use std::ffi::OsString;
use std::fs::{DirBuilder, File, OpenOptions, Permissions};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use rustix::fs::OFlags;
use rustix::io::Errno;

use crate::app::NAME;
use crate::error::{KelluError, Result};

/// Mode of every directory Kellu creates for itself.
pub const PRIVATE_DIR_MODE: u32 = 0o700;
/// Mode of every file Kellu creates for itself.
pub const PRIVATE_FILE_MODE: u32 = 0o600;

const PERMISSION_BITS: u32 = 0o7777;
const NOFOLLOW_FLAGS: i32 = (OFlags::NOFOLLOW.bits() | OFlags::CLOEXEC.bits()) as i32;
const NOFOLLOW_DIR_FLAGS: i32 = NOFOLLOW_FLAGS | OFlags::DIRECTORY.bits() as i32;

// ------------------------------------------------------------------------- XDG ----

/// Providers, routing, scopes, sandbox policy.
pub fn config_dir() -> Result<PathBuf> {
    resolve(
        env::var_os("XDG_CONFIG_HOME"),
        env::var_os("HOME"),
        ".config",
    )
}

/// Session logs and history.
pub fn state_dir() -> Result<PathBuf> {
    resolve(
        env::var_os("XDG_STATE_HOME"),
        env::var_os("HOME"),
        ".local/state",
    )
}

/// Disposable caches.
pub fn cache_dir() -> Result<PathBuf> {
    resolve(env::var_os("XDG_CACHE_HOME"), env::var_os("HOME"), ".cache")
}

/// Pure resolution of an XDG directory (testable without touching the environment).
///
/// The XDG specification says relative values must be ignored, so a relative or empty `xdg`
/// falls back to `home/default_relative`.
fn resolve(
    xdg: Option<OsString>,
    home: Option<OsString>,
    default_relative: &str,
) -> Result<PathBuf> {
    let absolute = |value: OsString| Some(PathBuf::from(value)).filter(|p| p.is_absolute());
    if let Some(base) = xdg.and_then(absolute) {
        return Ok(base.join(NAME));
    }
    let home = home.and_then(absolute).ok_or_else(|| {
        KelluError::config("cannot determine the home directory (HOME is unset or not absolute)")
    })?;
    Ok(home.join(default_relative).join(NAME))
}

// ------------------------------------------------------------ private creation ----

fn is_loop(error: &io::Error) -> bool {
    error.raw_os_error() == Some(Errno::LOOP.raw_os_error())
}

fn is_not_dir(error: &io::Error) -> bool {
    error.raw_os_error() == Some(Errno::NOTDIR.raw_os_error())
}

fn ensure_owner(path: &Path, uid: u32) -> Result<()> {
    if uid == rustix::process::getuid().as_raw() {
        Ok(())
    } else {
        Err(KelluError::security(format!(
            "{} is owned by another user",
            path.display()
        )))
    }
}

/// Create `path` with mode 0700 if needed and verify it is safe to use.
///
/// Only the final component is checked: a symlinked parent (for example `~/.config`) is allowed.
///
/// # Errors
///
/// [`KelluError::Security`] when `path` is a symlink, not a directory, or owned by another user;
/// [`KelluError::Io`] when it cannot be created or inspected.
pub fn ensure_private_dir(path: &Path) -> Result<()> {
    DirBuilder::new()
        .recursive(true)
        .mode(PRIVATE_DIR_MODE)
        .create(path)
        .map_err(|e| KelluError::io(path, e))?;
    let not_real_dir = || {
        KelluError::security(format!(
            "{} must be a real directory, not a symlink or a file",
            path.display()
        ))
    };
    let dir = OpenOptions::new()
        .read(true)
        .custom_flags(NOFOLLOW_DIR_FLAGS)
        .open(path)
        .map_err(|e| {
            if is_loop(&e) || is_not_dir(&e) {
                not_real_dir()
            } else {
                KelluError::io(path, e)
            }
        })?;
    let meta = dir.metadata().map_err(|e| KelluError::io(path, e))?;
    if !meta.is_dir() {
        return Err(not_real_dir());
    }
    ensure_owner(path, meta.uid())?;
    if meta.mode() & PERMISSION_BITS != PRIVATE_DIR_MODE {
        dir.set_permissions(Permissions::from_mode(PRIVATE_DIR_MODE))
            .map_err(|e| KelluError::io(path, e))?;
    }
    Ok(())
}

/// Open `path` for appending, creating it with mode 0600.
///
/// The file is opened without following symlinks and verified on the open descriptor (regular
/// file, owned by the current user), which avoids check-then-open races. Loose permissions on an
/// existing file are tightened.
///
/// With `new_only`, an existing file is an error whose [`KelluError::io_kind`] is
/// `AlreadyExists`.
///
/// # Errors
///
/// [`KelluError::Security`] when `path` is a symlink, not a regular file, or owned by another
/// user; [`KelluError::Io`] for anything else.
pub fn open_private_append(path: &Path, new_only: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options
        .append(true)
        .mode(PRIVATE_FILE_MODE)
        .custom_flags(NOFOLLOW_FLAGS);
    if new_only {
        options.create_new(true);
    } else {
        options.create(true);
    }
    let file = options.open(path).map_err(|e| {
        if is_loop(&e) {
            KelluError::security(format!(
                "{} is a symlink; refusing to follow it",
                path.display()
            ))
        } else {
            KelluError::io(path, e)
        }
    })?;
    let meta = file.metadata().map_err(|e| KelluError::io(path, e))?;
    if !meta.is_file() {
        return Err(KelluError::security(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    ensure_owner(path, meta.uid())?;
    if meta.mode() & PERMISSION_BITS != PRIVATE_FILE_MODE {
        file.set_permissions(Permissions::from_mode(PRIVATE_FILE_MODE))
            .map_err(|e| KelluError::io(path, e))?;
    }
    Ok(file)
}

/// Create `path` with mode 0600 if needed (tightening permissions) and verify it.
///
/// # Errors
///
/// Same as [`open_private_append`].
pub fn ensure_private_file(path: &Path) -> Result<()> {
    open_private_append(path, false).map(drop)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    fn os(value: &str) -> Option<OsString> {
        Some(OsString::from(value))
    }

    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).unwrap().mode() & PERMISSION_BITS
    }

    #[test]
    fn defaults_follow_xdg_layout() {
        assert_eq!(
            resolve(None, os("/home/u"), ".config").unwrap(),
            Path::new("/home/u/.config/kellu")
        );
        assert_eq!(
            resolve(None, os("/home/u"), ".local/state").unwrap(),
            Path::new("/home/u/.local/state/kellu")
        );
        assert_eq!(
            resolve(None, os("/home/u"), ".cache").unwrap(),
            Path::new("/home/u/.cache/kellu")
        );
    }

    #[test]
    fn absolute_xdg_variable_is_honored() {
        assert_eq!(
            resolve(os("/custom"), os("/home/u"), ".config").unwrap(),
            Path::new("/custom/kellu")
        );
    }

    #[test]
    fn relative_or_empty_xdg_variable_is_ignored() {
        for bad in ["relative/dir", ""] {
            assert_eq!(
                resolve(os(bad), os("/home/u"), ".config").unwrap(),
                Path::new("/home/u/.config/kellu")
            );
        }
    }

    #[test]
    fn missing_or_relative_home_is_an_error_not_a_panic() {
        assert!(matches!(
            resolve(None, None, ".config"),
            Err(KelluError::Config(_))
        ));
        assert!(matches!(
            resolve(None, os("home"), ".config"),
            Err(KelluError::Config(_))
        ));
    }

    #[test]
    fn resolving_paths_does_not_create_anything() {
        let home = TempDir::new().unwrap();
        let home_str = home.path().to_str().unwrap();
        let resolved = resolve(None, os(home_str), ".config").unwrap();
        assert!(!resolved.exists());
        assert_eq!(fs::read_dir(home.path()).unwrap().count(), 0);
    }

    #[test]
    fn ensure_private_dir_creates_mode_0700() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("a/b/state");
        ensure_private_dir(&dir).unwrap();
        assert_eq!(mode_of(&dir), PRIVATE_DIR_MODE);
    }

    #[test]
    fn ensure_private_dir_tightens_loose_permissions() {
        let root = TempDir::new().unwrap();
        let dir = root.path().join("loose");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, Permissions::from_mode(0o755)).unwrap();
        ensure_private_dir(&dir).unwrap();
        assert_eq!(mode_of(&dir), PRIVATE_DIR_MODE);
    }

    #[test]
    fn ensure_private_dir_rejects_symlink() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        fs::create_dir(&target).unwrap();
        fs::set_permissions(&target, Permissions::from_mode(0o755)).unwrap();
        let link = root.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(matches!(
            ensure_private_dir(&link),
            Err(KelluError::Security(_))
        ));
        assert_eq!(
            mode_of(&target),
            0o755,
            "the symlink target must not be touched"
        );
    }

    #[test]
    fn ensure_private_dir_rejects_a_regular_file() {
        let root = TempDir::new().unwrap();
        let file = root.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(ensure_private_dir(&file).is_err());
    }

    #[test]
    fn ensure_private_dir_rejects_a_directory_owned_by_another_user() {
        if !rustix::process::getuid().is_root() {
            return; // chown needs root; the check itself is exercised when tests run as root
        }
        let root = TempDir::new().unwrap();
        let dir = root.path().join("foreign");
        fs::create_dir(&dir).unwrap();
        std::os::unix::fs::chown(&dir, Some(65534), None).unwrap();
        assert!(matches!(
            ensure_private_dir(&dir),
            Err(KelluError::Security(_))
        ));
    }

    #[test]
    fn open_private_append_creates_mode_0600() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("log");
        open_private_append(&path, false).unwrap();
        assert_eq!(mode_of(&path), PRIVATE_FILE_MODE);
    }

    #[test]
    fn open_private_append_appends() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("log");
        open_private_append(&path, false)
            .unwrap()
            .write_all(b"one\n")
            .unwrap();
        open_private_append(&path, false)
            .unwrap()
            .write_all(b"two\n")
            .unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "one\ntwo\n");
    }

    #[test]
    fn open_private_append_new_only_rejects_existing() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("log");
        open_private_append(&path, true).unwrap();
        let err = open_private_append(&path, true).unwrap_err();
        assert_eq!(err.io_kind(), Some(io::ErrorKind::AlreadyExists));
    }

    #[test]
    fn open_private_append_refuses_symlink() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("target");
        fs::write(&target, "secret").unwrap();
        let link = root.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(matches!(
            open_private_append(&link, false),
            Err(KelluError::Security(_))
        ));
        assert_eq!(fs::read_to_string(&target).unwrap(), "secret");
    }

    #[test]
    fn open_private_append_refuses_a_dangling_symlink_and_never_creates_its_target() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("not-yet");
        let link = root.path().join("link");
        symlink(&target, &link).unwrap();
        assert!(open_private_append(&link, false).is_err());
        assert!(!target.exists());
    }

    #[test]
    fn open_private_append_refuses_non_regular_files() {
        let root = TempDir::new().unwrap();
        assert!(open_private_append(root.path(), false).is_err());
    }

    #[test]
    fn open_private_append_tightens_existing_permissions() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("log");
        fs::write(&path, "x").unwrap();
        fs::set_permissions(&path, Permissions::from_mode(0o644)).unwrap();
        open_private_append(&path, false).unwrap();
        assert_eq!(mode_of(&path), PRIVATE_FILE_MODE);
    }

    #[test]
    fn ensure_private_file_creates_empty_private_file() {
        let root = TempDir::new().unwrap();
        let path = root.path().join("history");
        ensure_private_file(&path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        assert_eq!(mode_of(&path), PRIVATE_FILE_MODE);
    }
}
