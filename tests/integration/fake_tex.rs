//! `test_support::FakeTex`, the mocked `Exec` every later mocked test uses.

use std::io;
use std::time::Duration;

use tinytex::test_support::{FakeTex, Reply};
use tinytex::{CommandSpec, Exec};

fn spec(program: &str, args: &[&str]) -> CommandSpec {
    let mut s = CommandSpec::new(program);
    s.args = args.iter().map(Into::into).collect();
    s
}

#[test]
fn records_every_command_in_order() {
    let fake = FakeTex::new();
    fake.run(&spec("/bin/pdflatex", &["-halt-on-error", "doc.tex"]))
        .unwrap();
    fake.run(&spec("/bin/tlmgr.bat", &["install", "a"]))
        .unwrap();
    assert_eq!(fake.calls().len(), 2);
    assert_eq!(
        fake.lines(),
        ["pdflatex -halt-on-error doc.tex", "tlmgr install a"],
        "program names lose the directory and a .bat extension"
    );
    assert_eq!(fake.count("tlmgr"), 1);
}

#[test]
fn unscripted_commands_answer_status_zero_with_no_output() {
    let out = FakeTex::new().run(&spec("tlmgr", &["--version"])).unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
}

#[test]
fn answers_come_from_a_script_keyed_by_program_and_arguments() {
    let fake = FakeTex::new();
    fake.reply("tlmgr", &["install"], Reply::code(255).stderr("boom"));
    fake.reply("tlmgr", &[], Reply::ok().stdout("generic"));
    let out = fake.run(&spec("/x/tlmgr", &["install", "a"])).unwrap();
    assert_eq!(out.status.code(), Some(255));
    assert_eq!(out.stderr, b"boom");
    let out = fake.run(&spec("/x/tlmgr", &["search", "a"])).unwrap();
    assert_eq!(
        out.stdout, b"generic",
        "the more general rule still matches"
    );
}

#[test]
fn a_rule_with_several_replies_uses_them_in_order_and_repeats_the_last() {
    let fake = FakeTex::new();
    fake.on(
        "tlmgr",
        &["update"],
        vec![Reply::code(1), Reply::code(2), Reply::code(0)],
    );
    let codes: Vec<_> = (0..5)
        .map(|_| fake.run(&spec("tlmgr", &["update"])).unwrap().status.code())
        .collect();
    assert_eq!(codes, [Some(1), Some(2), Some(0), Some(0), Some(0)]);
}

#[test]
fn a_reply_can_create_files_in_the_commands_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let fake = FakeTex::new();
    fake.reply(
        "pdflatex",
        &[],
        Reply::ok()
            .creates("doc.log", "This is pdfTeX")
            .creates("doc.aux", ""),
    );
    let mut s = spec("pdflatex", &["doc.tex"]);
    s.cwd = Some(dir.path().to_path_buf());
    fake.run(&s).unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.path().join("doc.log")).unwrap(),
        "This is pdfTeX"
    );
    assert!(dir.path().join("doc.aux").exists());
}

#[test]
fn a_reply_can_fail_and_flip_interrupted_at_a_chosen_command() {
    let fake = FakeTex::new();
    fake.on(
        "tlmgr",
        &["install"],
        vec![
            Reply::ok(),
            Reply::ok().fails(io::ErrorKind::Interrupted).interrupts(),
        ],
    );
    assert!(!fake.interrupted());
    fake.run(&spec("tlmgr", &["install"])).unwrap();
    assert!(!fake.interrupted(), "the first command does not interrupt");
    let err = fake.run(&spec("tlmgr", &["install"])).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    assert!(fake.interrupted(), "interrupted() flips with the second");
    assert_eq!(
        fake.count("tlmgr install"),
        2,
        "failed commands are recorded"
    );
}

#[test]
fn a_handler_answers_commands_no_rule_matches() {
    let fake = FakeTex::new().with_handler(|s| {
        (s.args.first().is_some_and(|a| a == "info")).then(|| Reply::ok().stdout("from handler"))
    });
    fake.reply("tlmgr", &["install"], Reply::code(3));
    assert_eq!(
        fake.run(&spec("tlmgr", &["info"])).unwrap().stdout,
        b"from handler"
    );
    assert_eq!(
        fake.run(&spec("tlmgr", &["install"]))
            .unwrap()
            .status
            .code(),
        Some(3),
        "rules win over the handler"
    );
    assert!(
        fake.run(&spec("tlmgr", &["other"]))
            .unwrap()
            .status
            .success()
    );
}

#[test]
fn a_delayed_reply_sleeps_before_returning() {
    let fake = FakeTex::new();
    fake.reply("tlmgr", &[], Reply::ok().delayed(Duration::from_millis(60)));
    let start = std::time::Instant::now();
    fake.run(&spec("tlmgr", &[])).unwrap();
    assert!(start.elapsed() >= Duration::from_millis(60));
}
