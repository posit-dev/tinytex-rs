//! The process-wide install lock and the per-distribution update flags.
//!
//! One registry entry per distribution holds the lock that serializes
//! installation operations (install, update, format rebuild, remove) and the
//! "update done" and "TeX Live is too old" flags, which every compile of the
//! distribution in the process shares and which are never reset.
//!
//! The lock protects an operation, not data, so a poisoned `std` mutex is
//! recovered from. Waiting polls `try_lock` and gives up when the host's
//! [`Exec::interrupted`] turns true. The guard is its own type so that a
//! cross-process advisory file lock can replace it later (strand
//! `bd-sgmdpjrw`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard, TryLockError};
use std::time::Duration;

use crate::detect::Distribution;
use crate::exec::Exec;

/// How long a waiter sleeps between `try_lock` attempts.
const POLL: Duration = Duration::from_millis(5);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Root(PathBuf),
    BinDir(PathBuf),
}

/// What the process has learned about one distribution's updates.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Flags {
    /// Some `tlmgr update` has run to completion.
    pub update_done: bool,
    /// An update reported `(local, remote)`: the local TeX Live is older than
    /// the remote repository, so every later update is skipped.
    pub too_old: Option<(u32, u32)>,
}

/// One distribution's lock and flags.
#[derive(Default)]
pub(crate) struct Entry {
    lock: Mutex<()>,
    flags: Mutex<Flags>,
}

static REGISTRY: LazyLock<Mutex<HashMap<Key, Arc<Entry>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn key_of(dist: &Distribution) -> Key {
    let canon = |p: &PathBuf| p.canonicalize().unwrap_or_else(|_| p.clone());
    match &dist.root {
        Some(root) => Key::Root(canon(root)),
        None => Key::BinDir(canon(&dist.bin_dir)),
    }
}

/// The registry entry for `dist`: the root when known, the canonicalized
/// `bin_dir` otherwise.
pub(crate) fn entry_for(dist: &Distribution) -> Arc<Entry> {
    let mut registry = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
    registry.entry(key_of(dist)).or_default().clone()
}

/// Proof that the install lock is held. Inner steps take `&InstallGuard`
/// instead of locking again, because `std::sync::Mutex` is not re-entrant.
pub(crate) struct InstallGuard<'a> {
    _held: MutexGuard<'a, ()>,
}

/// Why a wait for the lock ended without the lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WaitInterrupted;

impl Entry {
    /// Takes the lock, polling until it is free or `x.interrupted()` is true.
    pub(crate) fn acquire<'a>(&'a self, x: &dyn Exec) -> Result<InstallGuard<'a>, WaitInterrupted> {
        loop {
            match self.lock.try_lock() {
                Ok(held) => return Ok(InstallGuard { _held: held }),
                Err(TryLockError::Poisoned(poisoned)) => {
                    return Ok(InstallGuard {
                        _held: poisoned.into_inner(),
                    });
                }
                Err(TryLockError::WouldBlock) => {}
            }
            if x.interrupted() {
                return Err(WaitInterrupted);
            }
            std::thread::sleep(POLL);
        }
    }

    pub(crate) fn flags(&self) -> Flags {
        *self.flags.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn update_flags(&self, f: impl FnOnce(&mut Flags)) {
        f(&mut self.flags.lock().unwrap_or_else(|e| e.into_inner()));
    }
}
