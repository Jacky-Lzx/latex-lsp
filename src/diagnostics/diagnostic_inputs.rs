//! Observe filesystem inputs actually consulted by one diagnostic calculation.
use std::{
    collections::BTreeMap,
    fs::Metadata,
    path::{Path, PathBuf},
    sync::Arc,
    time::SystemTime,
};

const MAX_FILES: usize = 4096;
const MAX_BYTES: usize = 512 * 1024;

#[derive(PartialEq)]
struct Stamp {
    modified: Option<SystemTime>,
    length: u64,
    file: bool,
    target: Option<PathBuf>,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl Stamp {
    fn read(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            modified: metadata.modified().ok(),
            length: metadata.len(),
            file: metadata.is_file(),
            target: path.canonicalize().ok(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (
                    metadata.dev(),
                    metadata.ino(),
                    metadata.ctime(),
                    metadata.ctime_nsec(),
                )
            },
        })
    }
}

#[derive(Default)]
pub(crate) struct Files {
    paths: BTreeMap<PathBuf, Option<Stamp>>,
    pub bytes: usize,
    overflow: bool,
}

impl Files {
    pub(crate) fn capture<T>(work: impl FnOnce() -> T) -> (T, Arc<Self>) {
        let context = crate::analysis::AnalysisContext::observing();
        let result = context.run(|_| work());
        (result, context.take_files())
    }

    pub(crate) fn current(&self) -> bool {
        !self.overflow
            && self
                .paths
                .iter()
                .all(|(path, old)| Stamp::read(path) == *old)
    }
}

impl Files {
    pub(crate) fn observe(&mut self, path: &Path) {
        if self.overflow || self.paths.contains_key(path) {
            return;
        }
        let stamp = Stamp::read(path);
        let bytes = std::mem::size_of::<Stamp>()
            + path.as_os_str().len()
            + stamp
                .as_ref()
                .and_then(|stamp| stamp.target.as_ref())
                .map_or(0, |path| path.as_os_str().len());
        if self.paths.len() >= MAX_FILES || self.bytes + bytes > MAX_BYTES {
            self.overflow = true;
            return;
        }
        self.bytes += bytes;
        self.paths.insert(path.to_owned(), stamp);
    }
}

pub(crate) fn observe(path: &Path) {
    if let Some(context) = crate::analysis::AnalysisContext::current() {
        context.observe(path);
    }
}

pub(crate) fn metadata(path: &Path) -> std::io::Result<Metadata> {
    observe(path);
    std::fs::metadata(path)
}

pub(crate) fn is_file(path: &Path) -> bool {
    observe(path);
    path.is_file()
}

pub(crate) fn canonicalize(path: &Path) -> std::io::Result<PathBuf> {
    observe(path);
    path.canonicalize()
}
