//! Host filesystem implementation of the storage backend.

use super::*;

/// Real host filesystem implementation.
pub struct FileSystem;

impl Backend for FileSystem {
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(path)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        std::fs::read_to_string(path)
    }

    fn read_bounded(&self, path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
        read_open_file_bounded(File::open(path)?, maximum)
    }

    fn read_bounded_no_follow(&self, path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
        read_bounded_no_follow(path, maximum)
    }

    fn fingerprint_bounded_no_follow(
        &self,
        path: &Path,
        maximum: u64,
    ) -> io::Result<FileFingerprint> {
        fingerprint_bounded_no_follow(path, maximum)
    }

    fn write_atomic(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        atomic_write(path, contents)
    }

    fn write_atomic_private(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        atomic_write_private(path, contents)
    }

    fn write_new_private(&self, path: &Path, contents: &[u8]) -> io::Result<()> {
        write_new_private(path, contents)
    }

    fn write_new_private_stream(
        &self,
        path: &Path,
        source: &mut dyn io::Read,
        maximum: u64,
    ) -> io::Result<FileFingerprint> {
        write_new_private_stream(path, source, maximum)
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }

    fn create_dir_all_private(&self, path: &Path) -> io::Result<()> {
        create_dir_all_private(path)
    }

    fn rename(&self, source: &Path, destination: &Path) -> io::Result<()> {
        std::fs::rename(source, destination)
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }

    fn remove_file_durable(&self, path: &Path) -> io::Result<()> {
        remove_file_durable(path)
    }

    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(path)
    }

    fn copy(&self, source: &Path, destination: &Path) -> io::Result<u64> {
        copy_no_clobber(source, destination)
    }
}
