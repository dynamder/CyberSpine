//! Putting an `.csp` on disk: atomic replace, with the platform quirks handled.
//!
//! This lives in the assembler rather than in `cyberspine-core` on purpose. The
//! core crate is the *format* — encode, decode, validate, pure computation — and
//! only a producer ever writes an image. Reading one from a path lives in
//! `cyberspine-vm::io` (a running VM must open a file); nothing writes.
//!
//! Nothing below depends on the assembler, so the whole module can move to its
//! own crate unchanged if a second producer ever appears.
//!
//! # Contract
//!
//! * **Atomic replace.** The bytes go to a temporary file in the destination's
//!   directory and are then renamed over the destination. A crash or a full disk
//!   leaves the old file untouched — never a half-written image, which the
//!   loader would only report as a confusing hash mismatch (C6).
//! * **Same directory, so the same filesystem.** Renaming across devices fails
//!   with `EXDEV`/`CrossesDevices`; a temporary file beside the target is what
//!   makes the rename always legal.
//! * **Symlinks are followed**, like `fs::write` and Cargo: writing through a
//!   link updates its target instead of replacing the link.
//! * **Permissions survive.** On Unix the temporary file is created with the
//!   umask applied (a new image comes out `0644`), and when the target already
//!   exists its mode is copied over — otherwise temp-file writers silently turn
//!   every rebuild into a `0600` artifact. On Windows the read-only attribute is
//!   carried over for the same reason.
//! * **Atomic, not durable.** Nothing here calls `fsync`: `rename` guarantees a
//!   reader sees the old file or the new one, not that either survives a power
//!   loss. Cargo's `write_atomic` makes the same trade.
//!
//! # Deliberately not preserved
//!
//! The replacement is a *new inode*: the owner, ACLs and extended attributes of
//! an existing destination do not carry over (the temporary file's do). Opening
//! the destination and writing in place would keep them, at the cost of no
//! longer being atomic. Cargo makes the same trade.

use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use cyberspine_core::binfmt::image::Image;
use cyberspine_core::binfmt::load::LoadError;
use thiserror::Error;

/// How many names to try before giving up on creating the temporary file.
const TEMP_ATTEMPTS: u32 = 16;

/// How many times a `rename` that failed with a transient error is retried.
const RENAME_ATTEMPTS: u32 = 8;

/// Everything that can stop an image from reaching the disk.
#[derive(Debug, Error)]
pub enum WriteError {
    /// The destination could not be replaced. The old file, if there was one, is
    /// intact.
    #[error("cannot write `{}`: {source}{hint}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
        /// Platform text for a failure the raw message does not explain. On
        /// Windows, 5/32/33 ("Access is denied") almost always mean another
        /// process has the file open, which the OS message never says.
        hint: &'static str,
    },

    /// The encoder produced bytes the loader rejects, so nothing was written.
    ///
    /// This cannot be caused by a source file: it means the assembler built an
    /// image that violates `docs/binfmt.md` §9, which is a bug in the assembler.
    #[error("the generated image does not pass the loader: {source}")]
    NotCanonical {
        #[source]
        source: LoadError,
    },
}

/// Serialize `image` and atomically replace `path` with it.
///
/// The bytes are checked against the loader first: an image the loader would
/// reject is never written, so a broken build fails here instead of at load
/// time.
pub fn write_image(image: &Image, path: impl AsRef<Path>) -> Result<(), WriteError> {
    let path = path.as_ref();
    let bytes = image.to_bytes();

    // The same property the fuzz oracle asserts, enforced on the one path that
    // actually ships.
    image
        .validate()
        .map_err(|source| WriteError::NotCanonical { source })?;

    write_atomic(path, &bytes).map_err(|source| WriteError::Io {
        path: path.to_path_buf(),
        hint: open_hint(&source),
        source,
    })
}

/// An actionable suffix for the platform's opaque I/O errors.
fn open_hint(error: &io::Error) -> &'static str {
    #[cfg(windows)]
    if matches!(error.raw_os_error(), Some(5 | 32 | 33)) {
        return " (the file may be open in another process)";
    }

    #[cfg(not(windows))]
    let _ = error;

    ""
}

/// Atomically replace `path` with `bytes`.
///
/// Public because the other artifacts this project will grow (`.cspd`, tool
/// output) need exactly this and nothing about it is image-specific. See the
/// module docs for the guarantees.
pub fn write_atomic(path: impl AsRef<Path>, bytes: &[u8]) -> io::Result<()> {
    let destination = resolve_link(path.as_ref())?;

    let file_name = destination.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("`{}` is not a file path", destination.display()),
        )
    })?;
    let parent = match destination.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };

    let (temp_path, file) = create_temp(parent, file_name)?;
    // The guard deletes the temporary file on every early return. On success it
    // is disarmed *after* the rename, which is why it is created before the
    // first fallible write.
    let temp = TempFile::new(temp_path);

    // `fill` closed the handle: Windows refuses to rename a file that is still
    // open, and the guard should never have to delete one either.
    fill(&destination, bytes, file)?;

    // Windows refuses to replace a read-only destination, even though replacing
    // the directory entry is exactly what this function promises. Clear the
    // attribute for the rename and put it back if the rename fails; on success
    // the temporary file already carries the attribute (`copy_mode`).
    let cleared_readonly = clear_readonly(&destination)?;
    match rename_with_retry(temp.path(), &destination) {
        Ok(()) => {
            temp.keep();
            Ok(())
        }
        Err(error) => {
            if cleared_readonly {
                restore_readonly(&destination);
            }
            Err(error)
        }
    }
}

/// Write the contents, close the handle, and apply the destination's mode.
///
/// Takes the `File` by value so that it is closed on every path — including the
/// error ones, where the guard is about to delete it.
fn fill(destination: &Path, bytes: &[u8], mut file: File) -> io::Result<()> {
    file.write_all(bytes)?;
    copy_mode(destination, &file)
}

/// Follow a symlink to the real file, so that writing to a link updates its
/// target instead of replacing the link (the behaviour of `fs::write` and of
/// Cargo's `write_atomic`).
fn resolve_link(path: &Path) -> io::Result<PathBuf> {
    // Only "the destination does not exist yet" means "not a symlink". Any other
    // error would otherwise replace the link instead of its target, which is the
    // opposite of what this module promises.
    let is_link = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata.file_type().is_symlink(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if !is_link {
        return Ok(path.to_path_buf());
    }

    // `canonicalize` follows the whole chain; for a dangling link it fails, and
    // the single hop `read_link` gives us still lets the target be created.
    if let Ok(resolved) = fs::canonicalize(path) {
        return Ok(resolved);
    }

    let target = fs::read_link(path)?;
    if target.is_absolute() {
        return Ok(target);
    }
    Ok(path.parent().unwrap_or_else(|| Path::new("")).join(target))
}

/// A per-process-unique tag for a temporary name: pid, counter, nanoseconds.
///
/// No `rand` dependency: the tag only has to avoid colliding with a sibling
/// writer, and `create_new` turns any collision that does happen into a retry.
fn unique_tag() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    format!("{}.{counter}.{nanos}", std::process::id())
}

/// `.<name>.<tag>.<attempt>.tmp`, beside the destination.
///
/// The leading dot keeps the temporary file out of casual `ls` output, and the
/// suffix is deliberately short: Windows still has a 260-character path limit,
/// and the destination's own path is not something this module can shorten.
fn temp_path(parent: &Path, file_name: &OsStr, tag: &str, attempt: u32) -> PathBuf {
    let mut name = OsString::from(".");
    name.push(file_name);
    name.push(format!(".{tag}.{attempt}.tmp"));
    parent.join(name)
}

/// Create a fresh temporary file in `parent`, retrying if the name is taken.
fn create_temp(parent: &Path, file_name: &OsStr) -> io::Result<(PathBuf, File)> {
    let tag = unique_tag();
    let mut collision = None;

    for attempt in 0..TEMP_ATTEMPTS {
        let path = temp_path(parent, file_name, &tag, attempt);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                collision = Some(error);
            }
            Err(error) => return Err(error),
        }
    }

    Err(collision.unwrap_or_else(|| io::Error::other("could not create a temporary file")))
}

/// Delete the temporary file unless the write succeeded.
struct TempFile {
    path: PathBuf,
    armed: bool,
}

impl TempFile {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn keep(mut self) {
        self.armed = false;
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        if self.armed {
            // Best effort: the original I/O error is the one worth reporting.
            let _ = remove_temp(&self.path);
        }
    }
}

/// Delete a temporary file. On Windows the guard has to clear the read-only
/// attribute first, because `copy_mode` may have copied it from the
/// destination and `DeleteFile` refuses to remove a read-only file.
#[allow(clippy::permissions_set_readonly_false)]
fn remove_temp(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    if let Ok(metadata) = fs::metadata(path) {
        let mut permissions = metadata.permissions();
        if permissions.readonly() {
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }
    }

    fs::remove_file(path)
}

/// Give the new file the destination's permissions, where the platform has any.
#[cfg(unix)]
fn copy_mode(destination: &Path, file: &File) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    // Only the permission bits: setuid/setgid/sticky are not ours to propagate.
    const MODE_MASK: u32 = 0o777;

    match fs::metadata(destination) {
        Ok(metadata) => file.set_permissions(fs::Permissions::from_mode(
            metadata.permissions().mode() & MODE_MASK,
        )),
        // A brand-new file keeps what `create_new` gave it: `0o666` minus umask.
        // Anything else is an error about the destination, not "no destination".
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Windows has no mode bits; the only thing worth carrying over is the read-only
/// attribute, or a read-only artifact would silently become writable.
#[cfg(windows)]
fn copy_mode(destination: &Path, file: &File) -> io::Result<()> {
    let readonly = match fs::metadata(destination) {
        Ok(metadata) => metadata.permissions().readonly(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    if !readonly {
        return Ok(());
    }

    let mut permissions = file.metadata()?.permissions();
    permissions.set_readonly(true);
    file.set_permissions(permissions)
}

#[cfg(not(any(unix, windows)))]
fn copy_mode(_destination: &Path, _file: &File) -> io::Result<()> {
    Ok(())
}

/// Clear a Windows read-only destination before replacing it. Returns whether
/// the attribute changed, so a failed rename can restore it; Unix replaces the
/// directory entry and never needs this.
#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)]
fn clear_readonly(path: &Path) -> io::Result<bool> {
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(false);
    };
    let mut permissions = metadata.permissions();
    if !permissions.readonly() {
        return Ok(false);
    }
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)?;
    Ok(true)
}

#[cfg(not(windows))]
fn clear_readonly(_path: &Path) -> io::Result<bool> {
    Ok(false)
}

/// Best effort: put the old destination's read-only attribute back after a
/// failed replace. The rename error is the one worth reporting.
#[cfg(windows)]
fn restore_readonly(path: &Path) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        return;
    }
    permissions.set_readonly(true);
    let _ = fs::set_permissions(path, permissions);
}

#[cfg(not(windows))]
fn restore_readonly(_path: &Path) {}

/// Rename, retrying the *transient* failures Windows produces when another
/// process has the destination open for a moment.
fn rename_with_retry(from: &Path, to: &Path) -> io::Result<()> {
    let mut attempts_left = RENAME_ATTEMPTS;
    let mut delay_ms = 10u64;

    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => {
                attempts_left -= 1;
                if attempts_left == 0 || !is_transient(&error) {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(delay_ms));
                delay_ms = (delay_ms * 2).min(80);
            }
        }
    }
}

/// Whether a failed `rename` may succeed if retried.
///
/// Unix `EACCES`/`EPERM` are permanent, so nothing is retried there. Windows is
/// different: `ERROR_ACCESS_DENIED` (5), `ERROR_SHARING_VIOLATION` (32) and
/// `ERROR_LOCK_VIOLATION` (33) are routinely transient because a scanner or the
/// search indexer opened the file microseconds earlier. 32 and 33 decode to
/// `ErrorKind::Uncategorized`, so the raw code is the only reliable check.
fn is_transient(error: &io::Error) -> bool {
    #[cfg(windows)]
    {
        matches!(error.raw_os_error(), Some(5 | 32 | 33))
    }

    #[cfg(not(windows))]
    {
        let _ = error;
        false
    }
}
