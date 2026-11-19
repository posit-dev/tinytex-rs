//! Helpers for the crate's own tests. Not part of the public API.

use std::process::ExitStatus;

/// An `ExitStatus` with the given exit code.
#[cfg(unix)]
pub fn exit_status(code: i32) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt;
    ExitStatus::from_raw(code << 8)
}

/// An `ExitStatus` with the given exit code.
#[cfg(windows)]
pub fn exit_status(code: i32) -> ExitStatus {
    use std::os::windows::process::ExitStatusExt;
    ExitStatus::from_raw(code as u32)
}
