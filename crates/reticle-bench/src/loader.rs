//! Loading benchmark tasks and suites from disk.
//!
//! A task is one TOML file that deserializes into a [`BenchTask`]; a suite is a
//! directory holding a `manifest.toml` ([`SuiteManifest`]) plus one `<id>.toml` per
//! task the manifest names. [`load_task`] reads a single task file, [`load_suite`]
//! reads a whole suite and returns the manifest paired with its tasks in manifest
//! order. Every failure is a structured [`LoadError`] that names the offending path,
//! so a malformed suite reports where it broke rather than panicking.

use std::fs;
use std::path::{Path, PathBuf};

use crate::{BenchTask, SuiteManifest};

/// The manifest file name expected at the root of a suite directory.
pub const MANIFEST_FILE: &str = "manifest.toml";

/// A failure loading a task or a suite from disk.
#[derive(Debug)]
pub enum LoadError {
    /// A file could not be read.
    Io {
        /// The path that failed.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// A TOML file did not parse into the expected schema.
    Parse {
        /// The path that failed to parse.
        path: PathBuf,
        /// The parser's message.
        message: String,
    },
    /// A loaded task's `id` did not match its file stem or its manifest entry.
    IdMismatch {
        /// The path whose contents disagreed with its name.
        path: PathBuf,
        /// The id the file declared.
        declared: String,
        /// The id the name (stem or manifest entry) implied.
        expected: String,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io { path, source } => {
                write!(f, "reading {}: {source}", path.display())
            }
            LoadError::Parse { path, message } => {
                write!(f, "parsing {}: {message}", path.display())
            }
            LoadError::IdMismatch {
                path,
                declared,
                expected,
            } => write!(
                f,
                "task id mismatch in {}: file declares `{declared}` but `{expected}` was expected",
                path.display()
            ),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Reads and parses a single task TOML file into a [`BenchTask`].
///
/// The file stem must equal the task's declared `id` (the schema calls the id "also
/// the file stem"); a mismatch is a [`LoadError::IdMismatch`] so a renamed file
/// cannot silently address the wrong task.
///
/// # Errors
///
/// Returns [`LoadError::Io`] if the file cannot be read, [`LoadError::Parse`] if it
/// is not valid task TOML, or [`LoadError::IdMismatch`] if the stem and id disagree.
pub fn load_task(path: &Path) -> Result<BenchTask, LoadError> {
    let text = fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let task: BenchTask = toml::from_str(&text).map_err(|e| LoadError::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    if let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        && stem != task.id
    {
        return Err(LoadError::IdMismatch {
            path: path.to_path_buf(),
            declared: task.id,
            expected: stem.to_owned(),
        });
    }
    Ok(task)
}

/// Reads and parses a suite's `manifest.toml` into a [`SuiteManifest`].
///
/// `dir` is the suite directory; the manifest is read from `dir/manifest.toml`.
///
/// # Errors
///
/// Returns [`LoadError::Io`] or [`LoadError::Parse`] on a missing or malformed
/// manifest.
pub fn load_manifest(dir: &Path) -> Result<SuiteManifest, LoadError> {
    let path = dir.join(MANIFEST_FILE);
    let text = fs::read_to_string(&path).map_err(|source| LoadError::Io {
        path: path.clone(),
        source,
    })?;
    toml::from_str(&text).map_err(|e| LoadError::Parse {
        path,
        message: e.to_string(),
    })
}

/// Loads a whole suite: its manifest and every task the manifest lists, in order.
///
/// Each id in the manifest is read from `dir/<id>.toml`; the loaded task's `id` must
/// match the manifest entry (and, via [`load_task`], its own file stem). The tasks
/// come back in manifest order so a run is reproducible regardless of directory
/// enumeration order.
///
/// # Errors
///
/// Propagates any [`LoadError`] from the manifest or from loading an individual task
/// (a missing task file listed in the manifest is a [`LoadError::Io`]).
pub fn load_suite(dir: &Path) -> Result<(SuiteManifest, Vec<BenchTask>), LoadError> {
    let manifest = load_manifest(dir)?;
    let mut tasks = Vec::with_capacity(manifest.tasks.len());
    for id in &manifest.tasks {
        let path = dir.join(format!("{id}.toml"));
        let task = load_task(&path)?;
        if &task.id != id {
            return Err(LoadError::IdMismatch {
                path,
                declared: task.id,
                expected: id.clone(),
            });
        }
        tasks.push(task);
    }
    Ok((manifest, tasks))
}

#[cfg(test)]
mod tests {
    use super::{LoadError, load_manifest, load_suite, load_task};
    use crate::Tier;
    use std::fs;

    /// Writes `contents` to `dir/name` and returns the path.
    fn write(dir: &std::path::Path, name: &str, contents: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        fs::write(&path, contents).expect("write fixture");
        path
    }

    const SAMPLE_TASK: &str = r#"
id = "t1_place_rect"
tier = 1
prompt = "Place a 1um met1 rectangle."
technology = "sky130.tech"
checker = "rect_present"
"#;

    #[test]
    fn load_task_parses_fields() {
        let dir = tempdir();
        let path = write(&dir, "t1_place_rect.toml", SAMPLE_TASK);
        let task = load_task(&path).expect("load");
        assert_eq!(task.id, "t1_place_rect");
        assert_eq!(task.tier, Tier(1));
        assert_eq!(task.checker, "rect_present");
        assert_eq!(task.technology, "sky130.tech");
        assert!(task.intent.is_none());
    }

    #[test]
    fn load_task_rejects_stem_mismatch() {
        let dir = tempdir();
        // File named differently from the declared id.
        let path = write(&dir, "wrong_name.toml", SAMPLE_TASK);
        let err = load_task(&path).expect_err("mismatch must error");
        assert!(matches!(err, LoadError::IdMismatch { .. }));
    }

    #[test]
    fn load_task_rejects_malformed_toml() {
        let dir = tempdir();
        let path = write(&dir, "broken.toml", "this is = not [valid");
        let err = load_task(&path).expect_err("malformed must error");
        assert!(matches!(err, LoadError::Parse { .. }));
    }

    #[test]
    fn load_suite_returns_manifest_and_tasks_in_order() {
        let dir = tempdir();
        write(
            &dir,
            "manifest.toml",
            "version = \"0.1.0\"\ntasks = [\"t1_place_rect\", \"t1_two\"]\n",
        );
        write(&dir, "t1_place_rect.toml", SAMPLE_TASK);
        write(
            &dir,
            "t1_two.toml",
            "id = \"t1_two\"\ntier = 1\nprompt = \"p\"\ntechnology = \"sky130.tech\"\nchecker = \"drc_clean\"\n",
        );
        let (manifest, tasks) = load_suite(&dir).expect("load suite");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(tasks.len(), 2);
        // Manifest order is preserved.
        assert_eq!(tasks[0].id, "t1_place_rect");
        assert_eq!(tasks[1].id, "t1_two");
    }

    #[test]
    fn load_manifest_missing_is_io_error() {
        let dir = tempdir();
        let err = load_manifest(&dir).expect_err("missing manifest must error");
        assert!(matches!(err, LoadError::Io { .. }));
    }

    #[test]
    fn load_suite_missing_task_file_errors() {
        let dir = tempdir();
        write(
            &dir,
            "manifest.toml",
            "version = \"0.1.0\"\ntasks = [\"absent\"]\n",
        );
        let err = load_suite(&dir).expect_err("absent task must error");
        assert!(matches!(err, LoadError::Io { .. }));
    }

    /// A unique temp directory under the OS temp root, created fresh per test.
    fn tempdir() -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("reticle-bench-loader-{}-{n}", std::process::id()));
        fs::create_dir_all(&dir).expect("create tempdir");
        dir
    }
}
