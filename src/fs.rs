use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

/// Abstract file system used by [`crate::Site`] so builds can run against disk or memory.
pub trait SiteFs {
    /// Lists all files recursively under `dir` as paths relative to the site root.
    ///
    /// If `dir` does not exist, returns an empty vector.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the underlying storage fails.
    fn list(&self, dir: &Path) -> io::Result<Vec<PathBuf>>;

    /// Reads the entire contents of `path` (relative to the site root).
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the file does not exist or cannot be read.
    fn read(&self, path: &Path) -> io::Result<Vec<u8>>;

    /// Writes `bytes` to `path` (relative to the site root), creating parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if writing fails.
    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()>;

    /// Removes the file at `path` (relative to the site root).
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] if the file does not exist or cannot be removed.
    fn remove(&mut self, path: &Path) -> io::Result<()>;
}

/// Normalizes a relative path into forward-slash separated components.
#[must_use]
pub fn normalize_rel_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        if let Component::Normal(os) = comp {
            out.push(os);
        }
    }
    out
}

/// Converts a relative path to a canonical POSIX string (`a/b/c`).
#[must_use]
pub fn path_to_posix(path: &Path) -> String {
    let mut parts = Vec::new();
    for comp in path.components() {
        if let Component::Normal(os) = comp {
            parts.push(os.to_string_lossy().into_owned());
        }
    }
    parts.join("/")
}

/// Real disk-backed implementation of [`SiteFs`] rooted at a directory.
#[derive(Debug, Clone)]
pub struct DiskFs {
    root: PathBuf,
}

impl DiskFs {
    /// Creates a new [`DiskFs`] rooted at `root`.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Returns the root path of this filesystem.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, rel: &Path) -> PathBuf {
        self.root.join(normalize_rel_path(rel))
    }
}

impl SiteFs for DiskFs {
    fn list(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        let rel_dir = normalize_rel_path(dir);
        let full_dir = self.root.join(&rel_dir);
        if !full_dir.exists() {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        let mut stack = vec![(full_dir, rel_dir)];
        while let Some((abs, rel)) = stack.pop() {
            let mut entries: Vec<_> = fs::read_dir(&abs)?.collect::<Result<Vec<_>, _>>()?;
            entries.sort_by_key(fs::DirEntry::file_name);
            for entry in entries {
                let ft = entry.file_type()?;
                let name = entry.file_name();
                let child_rel = if rel.as_os_str().is_empty() {
                    PathBuf::from(&name)
                } else {
                    rel.join(&name)
                };
                if ft.is_dir() {
                    stack.push((entry.path(), child_rel));
                } else if ft.is_file() {
                    out.push(child_rel);
                }
            }
        }
        out.sort();
        Ok(out)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        fs::read(self.resolve(path))
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let full = self.resolve(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(full, bytes)
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        fs::remove_file(self.resolve(path))
    }
}

/// Deterministic in-memory implementation of [`SiteFs`] backed by a [`BTreeMap`].
///
/// Tracks the number of [`SiteFs::write`] and [`SiteFs::remove`] calls for testing.
#[derive(Debug, Clone, Default)]
pub struct MemFs {
    /// Stored files keyed by normalized POSIX path.
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    /// Total number of `write` operations performed.
    pub write_count: usize,
    /// Total number of `remove` operations performed.
    pub remove_count: usize,
}

impl MemFs {
    /// Creates an empty [`MemFs`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inserts or updates a file without incrementing `write_count` (useful for test setup),
    /// or callers can use [`SiteFs::write`].
    pub fn insert(&mut self, path: impl AsRef<Path>, bytes: impl Into<Vec<u8>>) {
        let norm = normalize_rel_path(path.as_ref());
        self.files.insert(norm, bytes.into());
    }

    /// Reads a file as a UTF-8 string if present.
    #[must_use]
    pub fn read_to_string(&self, path: impl AsRef<Path>) -> Option<String> {
        let norm = normalize_rel_path(path.as_ref());
        self.files
            .get(&norm)
            .and_then(|b| std::str::from_utf8(b).ok().map(ToOwned::to_owned))
    }

    /// Returns whether `path` exists in memory.
    #[must_use]
    pub fn exists(&self, path: impl AsRef<Path>) -> bool {
        let norm = normalize_rel_path(path.as_ref());
        self.files.contains_key(&norm)
    }
}

impl SiteFs for MemFs {
    fn list(&self, dir: &Path) -> io::Result<Vec<PathBuf>> {
        let norm_dir = normalize_rel_path(dir);
        let mut out = Vec::new();
        for k in self.files.keys() {
            if norm_dir.as_os_str().is_empty() || k.starts_with(&norm_dir) {
                out.push(k.clone());
            }
        }
        Ok(out)
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        let norm = normalize_rel_path(path);
        self.files.get(&norm).cloned().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("not found: {}", norm.display()),
            )
        })
    }

    fn write(&mut self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        let norm = normalize_rel_path(path);
        self.files.insert(norm, bytes.to_vec());
        self.write_count += 1;
        Ok(())
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        let norm = normalize_rel_path(path);
        if self.files.remove(&norm).is_some() {
            self.remove_count += 1;
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("not found: {}", norm.display()),
            ))
        }
    }
}
