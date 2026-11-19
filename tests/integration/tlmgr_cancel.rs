//! Cancellation (design section 2): installation commands run to completion
//! and the operation stops before its next step; after any `Err` or non-zero
//! exit, `interrupted()` is tested first.

use std::io;
use std::sync::Arc;
use std::time::Duration;

use tinytex::log::SearchTerm;
use tinytex::test_support::{FakeTex, Reply};
use tinytex::{Config, NullObserver, Tlmgr, TlmgrError, UpdateOpts};

use super::tlmgr_model::{dist, fake, model};

fn pkgs(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn an_installation_command_completes_then_the_operation_stops() {
    let d = dist("cancel-install");
    let m = model(&[], &["a"], &[]);
    let x = fake(&m);
    // The install itself succeeds and flips interrupted().
    x.reply("tlmgr", &["install"], Reply::ok().interrupts());
    let cfg = Config::default();
    let err = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a"]))
        .unwrap_err();
    assert!(matches!(err, TlmgrError::Interrupted), "{err:?}");
    let lines = x.lines();
    assert_eq!(
        lines.last().map(String::as_str),
        Some("tlmgr install a"),
        "nothing runs after the interrupted command: {lines:?}"
    );

    // The lock was released: another operation on the distribution proceeds.
    let x2 = fake(&model(&[], &["a"], &[]));
    Tlmgr::new(&d, &cfg, &x2, &NullObserver)
        .install(&pkgs(&["a"]))
        .unwrap();
}

#[test]
fn an_interrupted_update_does_not_go_on_to_fmtutil() {
    let d = dist("cancel-update");
    let x = FakeTex::new();
    x.reply("tlmgr", &["update"], Reply::ok().interrupts());
    let cfg = Config::default();
    let err = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .update(UpdateOpts::default())
        .unwrap_err();
    assert!(matches!(err, TlmgrError::Interrupted));
    assert_eq!(x.count("fmtutil"), 0);
}

#[test]
fn an_interrupted_install_retry_stops_before_the_second_install() {
    let d = dist("cancel-retry");
    let x = fake(&model(&[], &["a"], &[(1, false)]));
    x.reply("tlmgr", &["update"], Reply::ok().interrupts());
    let cfg = Config::default();
    let err = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a"]))
        .unwrap_err();
    assert!(matches!(err, TlmgrError::Interrupted));
    assert_eq!(x.count("tlmgr install"), 1);
}

type Call = Box<dyn Fn(&Tlmgr) -> Result<(), TlmgrError>>;

fn read_only_calls() -> Vec<(&'static str, Call)> {
    vec![
        ("version", Box::new(|t| t.version().map(drop))),
        (
            "search",
            Box::new(|t| {
                t.search_file(&SearchTerm::File("/framed.sty".into()))
                    .map(drop)
            }),
        ),
        (
            "info",
            Box::new(|t| t.info(&pkgs(&["a"]), &["name"], true).map(drop)),
        ),
        ("repository", Box::new(|t| t.repository().map(drop))),
        ("run", Box::new(|t| t.run(&["--version"], false).map(drop))),
    ]
}

#[test]
fn a_killed_child_or_failed_exit_while_interrupted_is_interrupted_never_io_or_command() {
    let cfg = Config::default();
    for (i, (what, call)) in read_only_calls().iter().enumerate() {
        for reply in [
            Reply::ok().fails(io::ErrorKind::Interrupted),
            Reply::ok().fails(io::ErrorKind::Other),
            Reply::code(1),
            Reply::code(137),
        ] {
            let d = dist(&format!("cancel-ro-{i}"));
            let x = FakeTex::new();
            x.reply("tlmgr", &[], reply.clone());
            x.set_interrupted(true);
            let err = call(&Tlmgr::new(&d, &cfg, &x, &NullObserver)).unwrap_err();
            assert!(
                matches!(err, TlmgrError::Interrupted),
                "{what} with {reply:?}: {err:?}"
            );
        }
    }
}

#[test]
fn without_an_interruption_the_same_failures_are_io_and_command_errors() {
    let cfg = Config::default();
    let d = dist("cancel-plain");
    let x = FakeTex::new();
    x.reply("tlmgr", &[], Reply::ok().fails(io::ErrorKind::NotFound));
    let err = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .repository()
        .unwrap_err();
    assert!(matches!(err, TlmgrError::Io(_)), "{err:?}");

    let x = FakeTex::new();
    x.reply("tlmgr", &[], Reply::code(1).stderr("tlmgr: nope"));
    let err = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .repository()
        .unwrap_err();
    let TlmgrError::Command { code, stderr, .. } = err else {
        panic!("{err:?}")
    };
    assert_eq!(code, Some(1));
    assert_eq!(stderr, "tlmgr: nope");
}

#[test]
fn a_thread_waiting_for_the_lock_returns_when_interrupted_turns_true() {
    let cfg = Config::default();
    let d = dist("cancel-wait");
    // Thread A holds the lock for a while inside its install command.
    let a = fake(&model(&[], &["a"], &[]));
    a.reply(
        "tlmgr",
        &["install"],
        Reply::ok().delayed(Duration::from_millis(600)),
    );
    let b = Arc::new(fake(&model(&[], &["a"], &[])));
    std::thread::scope(|s| {
        s.spawn(|| {
            Tlmgr::new(&d, &cfg, &a, &NullObserver)
                .install(&pkgs(&["a"]))
                .unwrap();
        });
        std::thread::sleep(Duration::from_millis(100));
        let waiter = s.spawn(|| Tlmgr::new(&d, &cfg, &*b, &NullObserver).install(&pkgs(&["a"])));
        std::thread::sleep(Duration::from_millis(100));
        b.set_interrupted(true);
        let start = std::time::Instant::now();
        let err = waiter.join().unwrap().unwrap_err();
        assert!(matches!(err, TlmgrError::Interrupted), "{err:?}");
        assert!(
            start.elapsed() < Duration::from_millis(400),
            "the waiter did not wait out the lock holder"
        );
    });
    assert!(
        b.calls().is_empty(),
        "the waiter ran nothing: {:?}",
        b.lines()
    );
}
