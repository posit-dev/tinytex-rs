//! One integration-test binary for the crate; each file is a `pub mod` below,
//! alphabetized.

pub mod cli_coverage;
pub mod cli_errors;
pub mod cli_json;
pub mod cli_parity;
pub mod cli_support;
pub mod command;
pub mod compile_engines;
pub mod compile_errors;
pub mod compile_latexmk;
pub mod compile_loop;
pub mod compile_outputs;
pub mod compile_support;
pub mod compile_types;
pub mod config;
pub mod conformance;
pub mod conventions;
pub mod detect;
pub mod events;
pub mod exec;
pub mod fake_tex;
pub mod log;
pub mod real_tex;
pub mod sandbox;
pub mod sandbox_gate;
pub mod sandbox_script;
pub mod smoke;
pub mod tlmgr;
pub mod tlmgr_cancel;
pub mod tlmgr_fixtures;
pub mod tlmgr_lock;
pub mod tlmgr_model;
