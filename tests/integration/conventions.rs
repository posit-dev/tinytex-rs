//! The serialization convention (design section 9.4, item 5), as a test every
//! later plan extends.
//!
//! * Every public request, `Config`, event and result type is `Serialize` and
//!   `Deserialize`: list it in [`serde_both!`].
//! * `CompileError` and error types that wrap an `io::Error` are `Serialize`
//!   only: list them in [`serde_serialize_only!`].
//!
//! A new public type that is missing from both tables is a review question; a
//! listed type that lacks a derive is a compile error here, so the convention
//! cannot rot into a review comment.

use serde::{Deserialize, Serialize};

fn assert_serialize<T: Serialize>() {}
fn assert_deserialize<T: for<'de> Deserialize<'de>>() {}

/// Types that must derive both `Serialize` and `Deserialize`.
macro_rules! serde_both {
    ($($ty:ty),* $(,)?) => {
        #[test]
        fn public_data_types_are_serialize_and_deserialize() {
            $( assert_serialize::<$ty>(); assert_deserialize::<$ty>(); )*
        }
    };
}

/// Types that must derive `Serialize` only (they hold an `io::Error`).
macro_rules! serde_serialize_only {
    ($($ty:ty),* $(,)?) => {
        #[test]
        fn error_types_are_serialize() {
            $( assert_serialize::<$ty>(); )*
        }
    };
}

// Later plans append here as they add public types, for example
// `tinytex::CompileRequest` (plan 5.4a) and `tinytex::CompileError` (below).
serde_both!(
    tinytex::ErrorCode,
    tinytex::log::AccessibilityWarnings,
    tinytex::log::Hint,
    tinytex::log::LatexError,
    tinytex::log::SearchTerm,
    tinytex::tlmgr::FmtutilWarning,
    tinytex::tlmgr::InstallOutcome,
    tinytex::tlmgr::PathAction,
    tinytex::tlmgr::TlVersion,
    tinytex::tlmgr::UpdateOpts,
    tinytex::tlmgr::UpdateOutcome,
);
serde_serialize_only!(tinytex::tlmgr::TlmgrError);

#[test]
fn error_codes_are_stable_strings() {
    let code = tinytex::ErrorCode::new("latex.missing_package");
    assert_eq!(code.as_str(), "latex.missing_package");
    assert_eq!(
        serde_json::to_string(&code).unwrap(),
        "\"latex.missing_package\""
    );
    let back: tinytex::ErrorCode = serde_json::from_str("\"latex.missing_package\"").unwrap();
    assert_eq!(back, code);
    assert_eq!(code.to_string(), "latex.missing_package");
}

#[test]
fn tlmgr_errors_serialize_with_a_stable_code_and_a_message() {
    let err = tinytex::TlmgrError::Interrupted;
    assert_eq!(err.code().as_str(), "tlmgr.interrupted");
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(json["code"], "tlmgr.interrupted");
    assert_eq!(json["message"], "interrupted");
    let io = tinytex::TlmgrError::from(std::io::Error::other("x"));
    assert_eq!(io.code().as_str(), "tlmgr.io");
}
