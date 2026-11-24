//! The fixture loader and the raw per-call override of `tlmgr_model`: captured
//! real `tlmgr` replies played through the model's fake.

use std::path::PathBuf;

use tinytex::log::SearchTerm;
use tinytex::{CommandSpec, Config, Exec, NullObserver, Tlmgr};

use super::tlmgr_model::{dist, fake, fixture, model};

fn spec(args: &[&str]) -> CommandSpec {
    let mut s = CommandSpec::new("tlmgr");
    s.args = args.iter().map(Into::into).collect();
    s
}

fn fixture_bytes(name: &str, ext: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/tlmgr")
        .join(format!("{name}.{ext}"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn fixture_replays_the_status_and_both_streams_byte_for_byte() {
    let m = model(&[], &["a"], &[]);
    m.raw("install", fixture("install-readonly"));
    let x = fake(&m);
    let out = x.run(&spec(&["install", "a"])).unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(out.stdout, fixture_bytes("install-readonly", "stdout"));
    assert_eq!(out.stderr, fixture_bytes("install-readonly", "stderr"));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("You don't have permission"),
        "the streams are not swapped"
    );
}

#[test]
fn a_raw_reply_is_used_once_and_then_the_model_answers_again() {
    let m = model(&["a"], &[], &[]);
    m.raw("info", fixture("info-missing"));
    let x = fake(&m);
    let info = ["info", "--list", "--only-installed", "--data", "name", "a"];
    let first = x.run(&spec(&info)).unwrap();
    assert_eq!(
        first.status.code(),
        Some(1),
        "the raw reply: animate missing"
    );
    assert!(first.stdout.is_empty());
    let second = x.run(&spec(&info)).unwrap();
    assert_eq!(second.status.code(), Some(0), "the model's own reply");
    assert_eq!(String::from_utf8_lossy(&second.stdout), "a\n");
}

#[test]
fn a_raw_install_reply_keeps_the_models_state_change() {
    // The reply says "permission denied" while the model still installs `a`:
    // the override replaces what the caller sees, not what the script does.
    let m = model(&[], &["a"], &[(0, true)]);
    m.raw("install", fixture("install-readonly"));
    let x = fake(&m);
    let out = x.run(&spec(&["install", "a"])).unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(m.installed.lock().unwrap().contains("a"));
}

#[test]
fn a_raw_reply_reaches_a_command_the_model_has_no_handler_for() {
    // `search` has no branch in the model; the captured reply still plays, and
    // `search_file` skips the `tlmgr: package repository URL (...)` line only
    // because it requires a trailing colon on a package line.
    let captured = String::from_utf8(fixture_bytes("search-ocgbase", "stdout")).unwrap();
    assert!(
        captured.starts_with("tlmgr: package repository "),
        "the fixture keeps the repository line: {captured:?}"
    );
    let d = dist("fixture-search");
    let m = model(&[], &[], &[]);
    m.raw("search", fixture("search-ocgbase"));
    let x = fake(&m);
    let cfg = Config::default();
    let found = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .search_file(&SearchTerm::File("/ocgbase.sty".to_owned()))
        .unwrap();
    assert_eq!(found, ["ocgx2"]);
}

#[test]
fn a_search_that_finds_nothing_is_an_empty_list() {
    let d = dist("fixture-search-empty");
    let m = model(&[], &[], &[]);
    m.raw("search", fixture("search-empty"));
    let x = fake(&m);
    let cfg = Config::default();
    let found = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .search_file(&SearchTerm::File("/nosuchfile-zzqq.sty".to_owned()))
        .unwrap();
    assert!(found.is_empty());
}

#[test]
fn fixtures_are_not_subject_to_line_ending_conversion() {
    // Tests compare fixtures byte for byte, so a Windows checkout must not
    // turn their line endings into CRLF.
    let attrs =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".gitattributes"))
            .expect(".gitattributes exists");
    assert!(
        attrs
            .lines()
            .any(|l| l.split_whitespace().collect::<Vec<_>>() == ["tests/fixtures/**", "-text"]),
        "missing `tests/fixtures/** -text`: {attrs:?}"
    );
}
