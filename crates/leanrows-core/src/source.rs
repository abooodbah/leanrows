#[cfg(unix)]
use crate::SourceIdentity;
use crate::{SourceFingerprint, SourceRevision};
use std::fs::{self, File, Metadata};
use std::io;
use std::path::Path;

/// Random-access bytes used by scanners and viewport reconstruction.
pub trait PositionedRead {
    /// Returns the source's currently observable byte length.
    ///
    /// # Errors
    /// Returns an I/O error when metadata cannot be read.
    fn size(&self) -> io::Result<u64>;

    /// Returns bounded evidence for the source's current state.
    ///
    /// The default is intentionally size-only. File-backed and revision-aware
    /// implementations should override this method so callers can detect
    /// same-size rewrites and object replacement.
    ///
    /// # Errors
    /// Returns an I/O error when source evidence cannot be read.
    fn fingerprint(&self) -> io::Result<SourceFingerprint> {
        Ok(SourceFingerprint::new(self.size()?))
    }

    /// Reads into `destination` from an absolute byte offset.
    ///
    /// Implementations must never return a count greater than
    /// `destination.len()`.
    ///
    /// # Errors
    /// Returns an I/O error from the underlying source.
    fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize>;
}

pub(crate) fn validate_read_count(count: usize, capacity: usize) -> io::Result<usize> {
    if count > capacity {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("positioned read returned {count} bytes for a {capacity}-byte destination"),
        ));
    }
    Ok(count)
}

/// An opened file supporting positioned reads without a shared cursor.
#[derive(Debug)]
pub struct FileSource {
    file: File,
}

impl FileSource {
    /// Opens a file for read-only positioned access.
    ///
    /// # Errors
    /// Returns an I/O error if open or metadata lookup fails.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let file = File::open(path)?;
        file.metadata()?;
        Ok(Self { file })
    }

    /// Wraps an existing read-only file handle.
    ///
    /// # Errors
    /// Returns an I/O error if metadata lookup fails.
    pub fn from_file(file: File) -> io::Result<Self> {
        file.metadata()?;
        Ok(Self { file })
    }

    /// Captures evidence for the object currently addressed by `path`.
    ///
    /// This complements handle evidence: a retained handle remains bound to
    /// the old object after a path is replaced, while this lookup observes the
    /// path's current target.
    ///
    /// # Errors
    /// Returns an I/O error when path metadata cannot be read.
    pub fn fingerprint_path(path: impl AsRef<Path>) -> io::Result<SourceFingerprint> {
        fs::metadata(path).map(|metadata| metadata_fingerprint(&metadata))
    }

    /// Borrows the retained file handle for platform-specific read-only
    /// metadata checks.
    #[must_use]
    pub const fn as_file(&self) -> &File {
        &self.file
    }
}

impl PositionedRead for FileSource {
    fn size(&self) -> io::Result<u64> {
        self.file.metadata().map(|metadata| metadata.len())
    }

    fn fingerprint(&self) -> io::Result<SourceFingerprint> {
        self.file
            .metadata()
            .map(|metadata| metadata_fingerprint(&metadata))
    }

    fn read_at(&self, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
        positioned_file_read(&self.file, offset, destination)
    }
}

#[cfg(unix)]
fn metadata_fingerprint(metadata: &Metadata) -> SourceFingerprint {
    use std::os::unix::fs::MetadataExt;

    let identity = SourceIdentity::new(u128::from(metadata.dev()), u128::from(metadata.ino()));
    let revision = SourceRevision::new(
        encode_unix_time(metadata.mtime(), metadata.mtime_nsec()),
        encode_unix_time(metadata.ctime(), metadata.ctime_nsec()),
    );
    SourceFingerprint::with_evidence(metadata.len(), Some(identity), Some(revision))
}

#[cfg(unix)]
fn encode_unix_time(seconds: i64, nanoseconds: i64) -> u128 {
    (u128::from(seconds.cast_unsigned()) << 64) | u128::from(nanoseconds.cast_unsigned())
}

#[cfg(windows)]
fn metadata_fingerprint(metadata: &Metadata) -> SourceFingerprint {
    use std::os::windows::fs::MetadataExt;

    let revision = SourceRevision::new(
        u128::from(metadata.last_write_time()),
        u128::from(metadata.creation_time()),
    );
    // Stable std exposes Windows write/creation times but not file identity on
    // the workspace's pinned toolchain. The Win32 adapter adds handle-based
    // volume/file-ID evidence; core remains dependency-free and size/revision
    // safe on Windows.
    SourceFingerprint::with_evidence(metadata.len(), None, Some(revision))
}

#[cfg(not(any(unix, windows)))]
fn metadata_fingerprint(metadata: &Metadata) -> SourceFingerprint {
    SourceFingerprint::new(metadata.len())
}

#[cfg(unix)]
fn positioned_file_read(file: &File, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;
    file.read_at(destination, offset)
}

#[cfg(windows)]
fn positioned_file_read(file: &File, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;
    file.seek_read(destination, offset)
}

#[cfg(not(any(unix, windows)))]
fn positioned_file_read(file: &File, offset: u64, destination: &mut [u8]) -> io::Result<usize> {
    use std::io::{Read, Seek, SeekFrom};
    let mut private = file.try_clone()?;
    private.seek(SeekFrom::Start(offset))?;
    private.read(destination)
}

#[cfg(test)]
mod tests {
    use super::{FileSource, PositionedRead};
    use crate::SourceChange;
    use std::fs::{self, OpenOptions};
    use std::io::{self, Seek, SeekFrom, Write};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct Fixture(PathBuf);

    impl Fixture {
        fn new(bytes: &[u8]) -> io::Result<Self> {
            let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "leanrows-file-source-{}-{sequence}.tmp",
                std::process::id()
            ));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(bytes)?;
            Ok(Self(path))
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn file_source_reports_live_append_and_truncate_lengths()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = Fixture::new(b"abcd")?;
        let source = FileSource::open(&fixture.0)?;
        let baseline = source.fingerprint()?;
        assert_eq!(source.size()?, 4);

        let mut writer = OpenOptions::new().write(true).open(&fixture.0)?;
        writer.seek(SeekFrom::End(0))?;
        writer.write_all(b"ef")?;
        writer.sync_all()?;
        assert_eq!(source.size()?, 6);
        assert_eq!(baseline.compare(source.fingerprint()?), SourceChange::Grew);

        writer.set_len(2)?;
        writer.sync_all()?;
        assert_eq!(source.size()?, 2);
        assert_eq!(
            baseline.compare(source.fingerprint()?),
            SourceChange::Shrank
        );
        Ok(())
    }

    #[test]
    fn handle_and_path_fingerprints_agree_for_an_unchanged_file()
    -> Result<(), Box<dyn std::error::Error>> {
        let fixture = Fixture::new(b"same object")?;
        let source = FileSource::open(&fixture.0)?;
        assert_eq!(
            source.fingerprint()?,
            FileSource::fingerprint_path(&fixture.0)?
        );
        Ok(())
    }
}
