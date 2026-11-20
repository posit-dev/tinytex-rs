//! The coverage walk: every flag of the `clap` definition is either in the
//! parity table or on the exception list, and every field of `CompileRequest`
//! and `Config` has a flag or is on the environment-only list.
#![cfg(feature = "cli")]

use std::collections::BTreeSet;

use clap::CommandFactory;
use tinytex::cli::Cli;
use tinytex::{CompileRequest, Config};

/// `(flag, field it sets)`: the parity table of the design note.
const PARITY: &[(&str, &str)] = &[
    ("pdf-engine", "engine"),
    ("pdf-engine-opt", "engine_args"),
    ("bib-engine", "bib"),
    ("index-engine", "index"),
    ("index-engine-opt", "index"),
    ("min", "min_runs"),
    ("max", "max_runs"),
    ("driver", "driver"),
    ("recover", "recover"),
    ("on-bib-failure", "on_bib_failure"),
    ("keep-log", "keep_log"),
    ("output", "output"),
    ("output-dir", "output_dir"),
    ("pdf-file", "pdf_file"),
    ("input-path", "input_paths"),
    ("clean", "clean"),
    ("no-clean", "clean"),
    ("auto-install", "auto_install"),
    ("no-auto-install", "auto_install"),
    ("tool", "tools"),
    ("tlmgr-opt", "tlmgr_opts"),
    ("tlmgr-update", "tlmgr_update"),
    ("prefer", "prefer"),
    ("install-dir", "install_dir"),
    ("repository", "repository"),
    ("tlmgr-path", "tlmgr_path"),
    ("usermode", "usermode"),
    ("delete-tlpdb", "delete_tlpdb"),
    ("verbose", "verbose"),
];

/// Flags that choose how the command takes input or prints output and set no
/// field.
const EXCEPTIONS: &[&str] = &["request", "print-request", "json", "events", "timeout"];

/// Fields set by the environment only.
const ENV_ONLY: &[&str] = &["prevent_install"];

fn compile_flags() -> BTreeSet<String> {
    let cmd = Cli::command();
    let mut flags = BTreeSet::new();
    for a in cmd.get_arguments() {
        if let Some(l) = a.get_long() {
            flags.insert(l.to_owned());
        }
    }
    let compile = cmd
        .find_subcommand("compile")
        .expect("a `compile` subcommand");
    for a in compile.get_arguments() {
        if let Some(l) = a.get_long() {
            flags.insert(l.to_owned());
        }
    }
    flags
}

fn keys<T: serde::Serialize>(value: &T) -> BTreeSet<String> {
    serde_json::to_value(value)
        .unwrap()
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

#[test]
fn every_flag_is_in_the_parity_table_or_on_the_exception_list() {
    let known: BTreeSet<&str> = PARITY
        .iter()
        .map(|(f, _)| *f)
        .chain(EXCEPTIONS.iter().copied())
        .chain(["help", "version"])
        .collect();
    for flag in compile_flags() {
        assert!(
            known.contains(flag.as_str()),
            "--{flag} is neither in the parity table nor on the exception list"
        );
    }
}

#[test]
fn every_table_flag_exists_in_the_clap_definition() {
    let flags = compile_flags();
    for (flag, _) in PARITY {
        assert!(
            flags.contains(*flag),
            "--{flag} is in the table but not defined"
        );
    }
    for flag in EXCEPTIONS {
        assert!(
            flags.contains(*flag),
            "--{flag} is an exception but not defined"
        );
    }
}

#[test]
fn every_field_has_a_flag_or_is_environment_only() {
    let covered: BTreeSet<&str> = PARITY.iter().map(|(_, f)| *f).collect();
    let mut all = keys(&CompileRequest::default());
    all.extend(keys(&Config::default()));
    for field in &all {
        if ENV_ONLY.contains(&field.as_str()) {
            continue;
        }
        assert!(
            covered.contains(field.as_str()),
            "field `{field}` has no flag in the parity table"
        );
    }
    for (flag, field) in PARITY {
        assert!(
            all.contains(*field),
            "--{flag} claims the field `{field}`, which does not exist"
        );
    }
}

#[test]
fn the_environment_only_list_names_real_fields_without_flags() {
    let flags = compile_flags();
    let all = keys(&Config::default());
    for field in ENV_ONLY {
        assert!(all.contains(*field), "{field} is not a Config field");
        assert!(
            !PARITY.iter().any(|(_, f)| f == field),
            "{field} is environment-only but has a flag"
        );
    }
    assert!(!flags.contains("prevent-install"));
}
