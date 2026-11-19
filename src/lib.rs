//! Blocking driver for TeX Live and TinyTeX.
//!
//! The crate has no async runtime and no dependency on any Quarto crate. It is
//! being built milestone by milestone; see `docs/design.md`.
//!
//! # Conventions
//!
//! * Every public request, config, event and result type derives `Serialize`
//!   and `Deserialize`. Error types that wrap an `io::Error` derive
//!   `Serialize` only (the impl is hand-written).
//! * Every error carries a stable string [`ErrorCode`] that non-Rust hosts can
//!   switch on.
//! * Observer events carry the data, and a `Display` text, a host needs to
//!   render its own messages.

#![forbid(unsafe_code)]

pub mod config;
pub mod detect;
pub mod exec;
mod lock;
pub mod log;
#[doc(hidden)]
pub mod test_support;
pub mod tlmgr;

pub use config::{Config, Prefer, TlmgrUpdate};
pub use detect::{Distribution, Kind};
pub use exec::{CommandSpec, Event, Exec, NullObserver, Observer, StdExec};
pub use tlmgr::{
    FmtutilWarning, InstallOutcome, PathAction, Tlmgr, TlmgrError, UpdateOpts, UpdateOutcome,
};

use std::fmt;

use serde::{Deserialize, Serialize};

/// The crate's name, as it appears in diagnostics and the CLI.
pub const CRATE_NAME: &str = "tinytex";

/// A stable, dotted, lowercase string code that identifies an error kind, such
/// as `latex.missing_package`. Codes are part of the public contract: a host
/// in another language switches on them, so a code is never renamed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ErrorCode(String);

impl ErrorCode {
    /// Creates a code from its string form.
    pub fn new(code: impl Into<String>) -> Self {
        Self(code.into())
    }

    /// The string form of the code.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
