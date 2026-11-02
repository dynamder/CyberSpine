//! Reading a `.csp` image from disk.
//!
//! `cyberspine-core`'s [`Image::from_bytes`] is the only format entry point: it
//! is `no_std` and therefore cannot name a path. This module is the thin host
//! wrapper around it, and it exists for one reason that has nothing to do with
//! parsing — **the size gate must run before the read**.
//!
//! §9.1 C5 already stops the loader allocating from a *declared* count, but it
//! cannot help with the read itself: by the time `from_bytes` runs, the bytes are
//! already in memory. A 64 GiB sparse file would be read in full before any
//! format check fired. So the gate lives here, on the file's metadata.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

use cyberspine_core::binfmt::load::{LoadError, VerifiedImage};
use thiserror::Error;

/// The largest file this loader will read, in bytes.
///
/// This is a host policy, not a format rule: a 64 MiB `.csp` is already
/// pathological (8 Mi instructions), and anything far beyond it is more likely a
/// mistake than an action.
pub const MAX_FILE_LEN: usize = 64 * 1024 * 1024;

/// Every way reading an image from disk can fail.
///
/// The format-level failures are deliberately kept as [`LoadError`] rather than
/// re-spelled here: a test that asserts "rejected" should assert *which rule*
/// rejected it, and re-wrapping would give two names for the same thing.
#[derive(Debug, Error)]
pub enum LoadFileError {
    /// The file could not be opened or read.
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The file is larger than [`MAX_FILE_LEN`], refused before reading.
    #[error("file is {len} bytes, above the {max}-byte limit this loader will read")]
    FileTooLarge { len: usize, max: usize },
    /// The bytes were read but are not a valid image (§9).
    #[error(transparent)]
    Image(#[from] LoadError),
}

/// Read and fully verify a `.csp` file from disk.
///
/// The size gate runs on the file's metadata *before* it is read, so an
/// oversized file never becomes an allocation.
pub fn read_image(path: impl AsRef<Path>) -> Result<VerifiedImage, LoadFileError> {
    let path = path.as_ref();
    let file = File::open(path).map_err(|source| LoadFileError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    let len = file
        .metadata()
        .map_err(|source| LoadFileError::Io {
            path: path.to_path_buf(),
            source,
        })?
        .len();

    // Gate on the length we are about to trust, before trusting it.
    if (len as usize) > MAX_FILE_LEN {
        return Err(LoadFileError::FileTooLarge {
            len: len as usize,
            max: MAX_FILE_LEN,
        });
    }

    // Read one byte past the declared length: if the file grew between the
    // metadata call and now, the extra byte shows up and C1 rejects the mismatch
    // instead of the loader silently reading a partial image.
    let mut bytes = Vec::with_capacity(len as usize + 1);
    file.take(len + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| LoadFileError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(VerifiedImage::from_bytes(&bytes)?)
}
