//! Reading a `.csp` image from disk.
//!
//! `cyberspine-binfmt`'s [`VerifiedImage::from_bytes`] is the only format entry point: it
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

use cyberspine_core::binfmt::image::MAX_IMAGE_LEN;
use cyberspine_core::binfmt::load::{LoadError, VerifiedImage};
use thiserror::Error;

/// The largest file this loader will read, in bytes.
///
/// The format has its own cap ([`MAX_IMAGE_LEN`], §9.1 C0) and it is the smaller
/// one: it exists because a decodable image expands to about four times its size,
/// so the number that matters is the one checked before a byte is in memory. This
/// constant restates it for the file path, where the gate can run on the file's
/// metadata — a 64 MiB sparse file is refused without ever being read.
pub const MAX_FILE_LEN: usize = MAX_IMAGE_LEN;

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
    FileTooLarge { len: u64, max: usize },
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

    // Gate on the length we are about to trust, before trusting it. The
    // comparison is in `u64`: `usize` is 32 bits on wasm32 and on the
    // microcontroller-class hosts, so narrowing first would let a 4 GiB file
    // wrap to a small `usize` and pass the gate.
    if len > MAX_FILE_LEN as u64 {
        return Err(LoadFileError::FileTooLarge {
            len,
            max: MAX_FILE_LEN,
        });
    }
    let len = len as usize;

    // Read one byte past the declared length: if the file grew between the
    // metadata call and now, the extra byte shows up and C1 rejects the mismatch
    // instead of the loader silently reading a partial image. The capacity is
    // capped so a sparse file at the limit does not allocate the whole limit
    // before a single byte is readable.
    let mut bytes = Vec::with_capacity(len.min(1024 * 1024) + 1);
    file.take(len as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| LoadFileError::Io {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(VerifiedImage::from_bytes(&bytes)?)
}
