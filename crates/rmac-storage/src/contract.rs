//! Shared [`Backend`] contract, run against both [`InMemoryBackend`] and
//! the real host [`FileSystem`] (in a scratch directory — never against
//! anything durable, so unlike the network/Bluetooth contracts this one
//! is safe to run unconditionally, not just `#[ignore]`d).

use super::*;

/// Exercises create-directory, atomic write/overwrite, read, remove, and
/// no-clobber write-new against `backend`, rooted at `root` (which the
/// caller must be able to have created — a fresh in-memory root, or an
/// empty scratch directory on disk).
pub fn assert_backend_contract(backend: &impl Backend, root: &Path) {
    backend
        .create_dir_all(root)
        .expect("create_dir_all succeeds");

    let settings = root.join("settings.json");
    backend
        .write_atomic(&settings, b"{\"version\":1}")
        .expect("first write_atomic succeeds");
    assert_eq!(
        backend.read_to_string(&settings).unwrap(),
        "{\"version\":1}"
    );

    backend
        .write_atomic(&settings, b"{\"version\":2}")
        .expect("write_atomic overwrites an existing file");
    assert_eq!(backend.read(&settings).unwrap(), b"{\"version\":2}");

    let created_once = root.join("new-private.bin");
    backend
        .write_new_private(&created_once, b"first")
        .expect("write_new_private creates a fresh file");
    let clobber = backend.write_new_private(&created_once, b"second");
    assert_eq!(
        clobber.unwrap_err().kind(),
        io::ErrorKind::AlreadyExists,
        "write_new_private never replaces an existing file"
    );
    assert_eq!(backend.read(&created_once).unwrap(), b"first");

    backend
        .remove_file(&settings)
        .expect("remove_file succeeds");
    assert_eq!(
        backend.read(&settings).unwrap_err().kind(),
        io::ErrorKind::NotFound,
        "a removed file reads as not found"
    );

    let missing_parent = root.join("does-not-exist").join("child.json");
    assert!(
        backend.write_atomic(&missing_parent, b"{}").is_err(),
        "writing under a directory that was never created fails"
    );
}
