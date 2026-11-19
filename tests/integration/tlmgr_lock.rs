//! The install lock under threads.

use std::sync::Arc;
use std::time::Duration;

use tinytex::test_support::{FakeTex, Reply};
use tinytex::{Config, NullObserver, Tlmgr, UpdateOpts, UpdateOutcome};

use super::tlmgr_model::{dist, fake, model, without_info};

fn pkgs(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn two_threads_needing_the_same_package_install_it_once() {
    let cfg = Config::default();
    let d = dist("lock-recheck");
    let m = model(&[], &["a"], &[]);
    // The first thread's install is slow: it installs into the model, then
    // sleeps, so the second thread waits on the lock.
    let slow = {
        let m = Arc::clone(&m);
        FakeTex::new().with_handler(move |spec| {
            let args: Vec<String> = spec
                .args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            if args.first().is_some_and(|a| a == "install") {
                m.installed.lock().unwrap().insert("a".to_owned());
                return Some(Reply::ok().delayed(Duration::from_millis(250)));
            }
            if args.iter().any(|a| a == "info") {
                let have = m.installed.lock().unwrap().contains("a");
                return Some(Reply::ok().stdout(if have { "a\n" } else { "" }));
            }
            None
        })
    };
    let waiter = fake(&m);
    let out2 = std::thread::scope(|s| {
        let h1 = s.spawn(|| {
            Tlmgr::new(&d, &cfg, &slow, &NullObserver)
                .install(&pkgs(&["a"]))
                .unwrap()
        });
        std::thread::sleep(Duration::from_millis(60));
        let out2 = Tlmgr::new(&d, &cfg, &waiter, &NullObserver)
            .install(&pkgs(&["a"]))
            .unwrap();
        h1.join().unwrap();
        out2
    });
    // One `install` in the recorded sequences of both threads together: the
    // second thread found the package present after it took the lock, and R's
    // algorithm (an update, then a second install) did not run for it.
    assert_eq!(
        slow.count("tlmgr install") + waiter.count("tlmgr install"),
        1
    );
    assert_eq!(waiter.count("tlmgr install"), 0, "{:?}", waiter.lines());
    assert!(
        without_info(waiter.lines()).is_empty(),
        "no update either: {:?}",
        waiter.lines()
    );
    assert_eq!(out2.already_present, ["a"]);
}

#[test]
fn a_poisoned_lock_is_recovered() {
    let cfg = Config::default();
    let d = dist("lock-poison");
    let panicking = FakeTex::new().with_handler(|spec| {
        if spec.args.iter().any(|a| a == "install") {
            panic!("a mock that panics while the lock is held");
        }
        None
    });
    let r = std::thread::scope(|s| {
        s.spawn(|| Tlmgr::new(&d, &cfg, &panicking, &NullObserver).install(&pkgs(&["a"])))
            .join()
    });
    assert!(r.is_err(), "the thread panicked");

    let x = fake(&model(&[], &["a"], &[]));
    let out = Tlmgr::new(&d, &cfg, &x, &NullObserver)
        .install(&pkgs(&["a"]))
        .unwrap();
    assert_eq!(out.installed, ["a"]);
}

#[test]
fn the_too_old_flag_is_shared_by_threads_and_never_reset() {
    let cfg = Config::default();
    let d = dist("lock-tooold");
    let x1 = FakeTex::new();
    x1.reply(
        "tlmgr",
        &["update"],
        Reply::code(1).stderr("Local TeX Live (2023) is older than remote repository (2024)"),
    );
    std::thread::scope(|s| {
        s.spawn(|| {
            let r = Tlmgr::new(&d, &cfg, &x1, &NullObserver)
                .update(UpdateOpts::default())
                .unwrap();
            assert!(matches!(r, UpdateOutcome::TexLiveTooOld { .. }));
        })
        .join()
        .unwrap();
    });
    // Another thread, then another "compile" later on the same thread: both
    // see the flag and run nothing.
    for _ in 0..2 {
        let x = FakeTex::new();
        let r = std::thread::scope(|s| {
            s.spawn(|| {
                Tlmgr::new(&d, &cfg, &x, &NullObserver)
                    .update(UpdateOpts::default())
                    .unwrap()
            })
            .join()
            .unwrap()
        });
        assert!(matches!(
            r,
            UpdateOutcome::TexLiveTooOld {
                local: 2023,
                remote: 2024
            }
        ));
        assert!(x.calls().is_empty());
    }
}
