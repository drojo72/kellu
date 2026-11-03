//! Project scope: which directory the assistant may read, and how files inside it are opened.
//!
//! * The root must not be `/`, the home directory or any ancestor of it. Kate falls back to the
//!   home directory as the LSP root when it finds no project, so this check is mandatory.
//! * A project marker (`.git`, `Cargo.toml`, ...) is required unless the user confirmed the
//!   directory explicitly.
//! * Files are opened **beneath** the root through a directory descriptor with `openat2` and
//!   `RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS`: the kernel refuses `..`, absolute paths and any
//!   symlink on the way, so there is no check-then-open race. Where `openat2` is missing
//!   (Linux before 5.6) reading fails closed.
//! * Deny globs apply before anything is opened.

use std::fs::{self, File};
use std::io::Read;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use kellu_common::{KelluError, Result};
use rustix::fs::{Mode, OFlags, ResolveFlags, openat2};
use rustix::io::Errno;

use crate::deny::DenyGlobs;

/// Files whose presence marks a directory as a project root.
const MARKERS: &[&str] = &[
    ".git",
    ".kateproject",
    "Cargo.toml",
    "pyproject.toml",
    "package.json",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "CMakeLists.txt",
    "meson.build",
    "composer.json",
    "Gemfile",
    "Makefile",
];

/// A project directory the assistant may read, opened once and held by descriptor.
#[derive(Debug)]
pub struct Scope {
    root: PathBuf,
    root_fd: OwnedFd,
    deny: DenyGlobs,
}

/// The home directory from `$HOME`, which must be absolute.
///
/// # Errors
/// `HOME` is unset or not an absolute path.
pub fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| KelluError::config("HOME is not set to an absolute path"))
}

impl Scope {
    /// Open `root` as a project scope.
    ///
    /// `confirmed` is the user's explicit confirmation for a directory without a project marker.
    ///
    /// # Errors
    /// The root is not a directory, is `/`, the home directory or an ancestor of it, has no
    /// marker and was not confirmed, or cannot be opened.
    pub fn open(root: &Path, home: &Path, deny: DenyGlobs, confirmed: bool) -> Result<Self> {
        let canonical = fs::canonicalize(root).map_err(|e| KelluError::io(root, e))?;
        if !fs::metadata(&canonical).is_ok_and(|meta| meta.is_dir()) {
            return Err(KelluError::security(format!(
                "{} is not a directory",
                canonical.display()
            )));
        }
        let home = fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
        if canonical.parent().is_none() || home.starts_with(&canonical) {
            return Err(KelluError::security(format!(
                "{} is the filesystem root, your home directory or one of its ancestors; \
                 open a project directory instead",
                canonical.display()
            )));
        }
        let has_marker = MARKERS
            .iter()
            .any(|marker| fs::symlink_metadata(canonical.join(marker)).is_ok());
        if !has_marker && !confirmed {
            return Err(KelluError::security(format!(
                "{} has no project marker (.git, Cargo.toml, ...); confirm it explicitly to use it",
                canonical.display()
            )));
        }
        let root_fd = rustix::fs::open(
            &canonical,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|errno| KelluError::io(&canonical, errno.into()))?;
        Ok(Self {
            root: canonical,
            root_fd,
            deny,
        })
    }

    /// The canonical root path.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the deny globs forbid `relative`.
    #[must_use]
    pub fn is_denied(&self, relative: &Path) -> bool {
        self.deny.is_denied(relative)
    }

    /// Open the regular file `relative` for reading.
    ///
    /// # Errors
    /// The path is denied, escapes the root, crosses a symlink, is not a regular file, or the
    /// kernel lacks `openat2`.
    pub fn open_file(&self, relative: &Path) -> Result<File> {
        if self.deny.is_denied(relative) {
            return Err(KelluError::security(format!(
                "{} is denied by policy",
                relative.display()
            )));
        }
        let fd = openat2(
            &self.root_fd,
            relative,
            // NONBLOCK keeps a FIFO placed in the tree from hanging the open.
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
            Mode::empty(),
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS,
        )
        .map_err(|errno| open_error(relative, errno))?;
        let file = File::from(fd);
        let meta = file.metadata().map_err(|e| KelluError::io(relative, e))?;
        if !meta.is_file() {
            return Err(KelluError::security(format!(
                "{} is not a regular file",
                relative.display()
            )));
        }
        Ok(file)
    }

    /// Read at most `max_bytes` of the regular file `relative`.
    ///
    /// # Errors
    /// As [`Scope::open_file`], or the file is larger than `max_bytes`.
    pub fn read_file(&self, relative: &Path, max_bytes: u64) -> Result<Vec<u8>> {
        let file = self.open_file(relative)?;
        let mut data = Vec::new();
        file.take(max_bytes + 1)
            .read_to_end(&mut data)
            .map_err(|e| KelluError::io(relative, e))?;
        if data.len() as u64 > max_bytes {
            return Err(KelluError::security(format!(
                "{} is larger than {max_bytes} bytes",
                relative.display()
            )));
        }
        Ok(data)
    }
}

fn open_error(relative: &Path, errno: Errno) -> KelluError {
    match errno {
        Errno::XDEV | Errno::LOOP | Errno::MLINK => KelluError::security(format!(
            "{} leaves the project or crosses a symlink",
            relative.display()
        )),
        Errno::NOSYS => KelluError::security(
            "this kernel lacks openat2 (Linux 5.6 or newer is required); refusing to read files",
        ),
        other => KelluError::io(relative, other.into()),
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    struct Fixture {
        _tmp: TempDir,
        home: PathBuf,
        project: PathBuf,
        outside: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let base = fs::canonicalize(tmp.path()).unwrap();
        let home = base.join("home");
        let project = home.join("project");
        let outside = base.join("outside");
        for dir in [&project.join(".git"), &project.join("src"), &outside] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(project.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(project.join(".env"), "TOKEN=abc").unwrap();
        fs::write(outside.join("secret.txt"), "outside").unwrap();
        Fixture {
            _tmp: tmp,
            home,
            project,
            outside,
        }
    }

    fn scope(f: &Fixture) -> Scope {
        Scope::open(&f.project, &f.home, DenyGlobs::new(), false).unwrap()
    }

    #[test]
    fn a_project_file_can_be_read() {
        let f = fixture();
        assert_eq!(
            scope(&f).read_file(Path::new("src/main.rs"), 1024).unwrap(),
            b"fn main() {}"
        );
    }

    #[test]
    fn the_filesystem_root_the_home_and_its_ancestors_are_refused() {
        let f = fixture();
        let base = f.home.parent().unwrap();
        for bad in [Path::new("/"), f.home.as_path(), base] {
            let error = Scope::open(bad, &f.home, DenyGlobs::new(), true).unwrap_err();
            assert!(
                error.to_string().contains("home directory"),
                "{bad:?}: {error}"
            );
        }
    }

    #[test]
    fn confirmation_does_not_unlock_the_home_directory() {
        let f = fixture();
        fs::create_dir(f.home.join(".git")).unwrap();
        assert!(Scope::open(&f.home, &f.home, DenyGlobs::new(), true).is_err());
    }

    #[test]
    fn a_directory_without_a_marker_needs_confirmation() {
        let f = fixture();
        let bare = f.home.join("bare");
        fs::create_dir(&bare).unwrap();
        assert!(Scope::open(&bare, &f.home, DenyGlobs::new(), false).is_err());
        assert!(Scope::open(&bare, &f.home, DenyGlobs::new(), true).is_ok());
    }

    #[test]
    fn a_file_is_not_a_scope() {
        let f = fixture();
        assert!(
            Scope::open(
                &f.project.join("src/main.rs"),
                &f.home,
                DenyGlobs::new(),
                true
            )
            .is_err()
        );
    }

    #[test]
    fn dotdot_and_absolute_paths_are_refused() {
        let f = fixture();
        let scope = scope(&f);
        for bad in [
            "../outside/secret.txt",
            "src/../../outside/secret.txt",
            "/etc/passwd",
        ] {
            assert!(scope.open_file(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_symlinked_file_pointing_outside_is_refused() {
        let f = fixture();
        symlink(f.outside.join("secret.txt"), f.project.join("src/link.txt")).unwrap();
        let error = scope(&f).open_file(Path::new("src/link.txt")).unwrap_err();
        assert!(error.to_string().contains("symlink"), "{error}");
    }

    #[test]
    fn a_symlinked_directory_pointing_outside_is_refused() {
        let f = fixture();
        symlink(&f.outside, f.project.join("src/ext")).unwrap();
        assert!(
            scope(&f)
                .open_file(Path::new("src/ext/secret.txt"))
                .is_err()
        );
    }

    #[test]
    fn even_a_symlink_that_stays_inside_is_refused() {
        let f = fixture();
        symlink("main.rs", f.project.join("src/alias.rs")).unwrap();
        assert!(scope(&f).open_file(Path::new("src/alias.rs")).is_err());
    }

    #[test]
    fn denied_files_are_refused_even_though_they_exist() {
        let f = fixture();
        let error = scope(&f).open_file(Path::new(".env")).unwrap_err();
        assert!(error.to_string().contains("denied"), "{error}");
        assert!(!error.to_string().contains("abc"));
    }

    #[test]
    fn a_swapped_root_cannot_redirect_reads() {
        // The scope holds a descriptor: replacing the directory by a symlink afterwards does not
        // change what is read.
        let f = fixture();
        let scope = scope(&f);
        let moved = f.project.with_file_name("project-moved");
        fs::rename(&f.project, &moved).unwrap();
        symlink(&f.outside, &f.project).unwrap();
        assert_eq!(
            scope.read_file(Path::new("src/main.rs"), 1024).unwrap(),
            b"fn main() {}"
        );
    }

    #[test]
    fn a_fifo_does_not_hang_and_is_not_a_regular_file() {
        let f = fixture();
        rustix::fs::mknodat(
            rustix::fs::CWD,
            f.project.join("src/pipe"),
            rustix::fs::FileType::Fifo,
            Mode::from_raw_mode(0o600),
            0,
        )
        .unwrap();
        let error = scope(&f).open_file(Path::new("src/pipe")).unwrap_err();
        assert!(error.to_string().contains("regular file"), "{error}");
    }

    #[test]
    fn directories_are_not_files() {
        let f = fixture();
        assert!(scope(&f).open_file(Path::new("src")).is_err());
    }

    #[test]
    fn oversized_files_are_refused() {
        let f = fixture();
        let error = scope(&f)
            .read_file(Path::new("src/main.rs"), 4)
            .unwrap_err();
        assert!(error.to_string().contains("larger than"), "{error}");
    }

    #[test]
    fn a_missing_file_is_an_io_error() {
        let f = fixture();
        assert!(scope(&f).open_file(Path::new("src/nope.rs")).is_err());
    }
}
