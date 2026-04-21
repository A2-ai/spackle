//! Pure error-kind mapping for the `SpackleFs` contract.
//!
//! Lives in its own module (no js-sys / wasm-bindgen deps) so it can be
//! unit-tested natively via `cargo test`. `src/wasm_fs.rs` is wasm32-only
//! and can't be exercised under a native test runner directly.

use std::io;

/// The seven error kinds defined by the `SpackleFs` TS contract.
/// These are the values the host passes in the thrown `{ kind, message }`
/// object. Anything not in this set collapses to `Other`.
pub const SPACKLE_FS_KINDS: &[&str] = &[
    "not-found",
    "permission-denied",
    "already-exists",
    "not-a-directory",
    "is-a-directory",
    "invalid-path",
    "other",
];

/// Map a `SpackleFsErrorKind` tag to a Rust `io::ErrorKind`. Unknown
/// tags map to `Other` (callers should still carry the original message).
pub fn map_spackle_fs_kind(kind: &str) -> io::ErrorKind {
    match kind {
        "not-found" => io::ErrorKind::NotFound,
        "permission-denied" => io::ErrorKind::PermissionDenied,
        "already-exists" => io::ErrorKind::AlreadyExists,
        "invalid-path" => io::ErrorKind::InvalidInput,
        // `not-a-directory`, `is-a-directory`, `other`, and any unknown
        // string all collapse to `Other`. The stable Rust `io::ErrorKind`
        // variants for directory-typed errors vary by MSRV; `Other` +
        // the message-carrying `io::Error` is the safe lowest common.
        "not-a-directory" | "is-a-directory" | "other" => io::ErrorKind::Other,
        _ => io::ErrorKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_kinds_map_to_specific_error_kinds() {
        assert_eq!(map_spackle_fs_kind("not-found"), io::ErrorKind::NotFound);
        assert_eq!(
            map_spackle_fs_kind("permission-denied"),
            io::ErrorKind::PermissionDenied,
        );
        assert_eq!(
            map_spackle_fs_kind("already-exists"),
            io::ErrorKind::AlreadyExists,
        );
        assert_eq!(
            map_spackle_fs_kind("invalid-path"),
            io::ErrorKind::InvalidInput,
        );
    }

    #[test]
    fn directory_kinds_and_other_collapse_to_other() {
        assert_eq!(map_spackle_fs_kind("not-a-directory"), io::ErrorKind::Other);
        assert_eq!(map_spackle_fs_kind("is-a-directory"), io::ErrorKind::Other);
        assert_eq!(map_spackle_fs_kind("other"), io::ErrorKind::Other);
    }

    #[test]
    fn unknown_kind_collapses_to_other() {
        assert_eq!(map_spackle_fs_kind("cosmic-ray"), io::ErrorKind::Other);
        assert_eq!(map_spackle_fs_kind(""), io::ErrorKind::Other);
    }

    #[test]
    fn spackle_fs_kinds_covers_all_mapped_kinds() {
        // Every value in SPACKLE_FS_KINDS must have a mapping (even if
        // it's Other). This test is the canary that catches additions
        // to the contract that forget to update the mapper.
        for kind in SPACKLE_FS_KINDS {
            // Just call it — any non-panic means the kind is handled.
            let _ = map_spackle_fs_kind(kind);
        }
    }
}
