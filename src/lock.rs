use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::domain::Domain;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
struct LockFileState {
    locked: bool,
    held_by_pid: Option<u32>,
}

fn lock_file_path(root: &Path, domain: Domain) -> PathBuf {
    let name = match domain {
        Domain::Tech => "techlock",
        Domain::Business => "businesslock",
    };
    root.join(".brained").join(name)
}

fn read_state(path: &Path) -> anyhow::Result<LockFileState> {
    if !path.exists() {
        return Ok(LockFileState::default());
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?)
}

fn write_state(path: &Path, state: &LockFileState) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let text = serde_json::to_string(state).context("serializing lock state")?;
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Returns `true` if `pid` is a real, currently-running process. Injected as a parameter
/// everywhere so unit tests can simulate a crashed process without ever spawning a real one.
pub type IsAlive<'a> = &'a dyn Fn(u32) -> bool;

pub fn real_is_alive(pid: u32) -> bool {
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(pid)]), true);
    system.process(sysinfo::Pid::from_u32(pid)).is_some()
}

fn is_free(state: &LockFileState, is_alive: IsAlive) -> bool {
    !state.locked || state.held_by_pid.map(|pid| !is_alive(pid)).unwrap_or(false)
}

/// Try-lock for the `locktech()`/`lockbusiness()` MCP tools. Returns `true` if the domain was
/// free (or held by a now-dead PID) and is now held by this process, `false` if another live
/// holder already has it. On success, also sets `held_by_us` — the in-memory flag this same
/// process consults before allowing a `write_work_file` call.
pub fn try_lock(root: &Path, domain: Domain, held_by_us: &AtomicBool, own_pid: u32, is_alive: IsAlive) -> anyhow::Result<bool> {
    let path = lock_file_path(root, domain);
    let state = read_state(&path)?;
    if !is_free(&state, is_alive) {
        return Ok(false);
    }
    write_state(&path, &LockFileState { locked: true, held_by_pid: Some(own_pid) })?;
    held_by_us.store(true, Ordering::SeqCst);
    Ok(true)
}

/// Release for the `unlocktech()`/`unlockbusiness()` MCP tools: only releases if this process
/// is the one that holds `held_by_us`. Returns `false` (no-op) if this process never held it.
pub fn unlock(root: &Path, domain: Domain, held_by_us: &AtomicBool) -> anyhow::Result<bool> {
    if !held_by_us.swap(false, Ordering::SeqCst) {
        return Ok(false);
    }
    let path = lock_file_path(root, domain);
    write_state(&path, &LockFileState { locked: false, held_by_pid: None })?;
    Ok(true)
}

/// `brd locktech`/`brd lockbusiness` CLI subcommands: purely disk-based (no in-memory state —
/// there's no running server in this process). Records `held_by_pid: None`, which means it is
/// never liveness-reclaimed — it persists until an explicit `cli_unlock`. Returns `true` if
/// newly acquired, `false` if already held by a live MCP-server process.
pub fn cli_lock(root: &Path, domain: Domain, is_alive: IsAlive) -> anyhow::Result<bool> {
    let path = lock_file_path(root, domain);
    let state = read_state(&path)?;
    if !is_free(&state, is_alive) {
        return Ok(false);
    }
    write_state(&path, &LockFileState { locked: true, held_by_pid: None })?;
    Ok(true)
}

/// `brd unlocktech`/`brd unlockbusiness` CLI subcommands: the human override escape hatch.
/// Unconditionally clears the lock file regardless of who currently holds it.
pub fn cli_unlock(root: &Path, domain: Domain) -> anyhow::Result<()> {
    let path = lock_file_path(root, domain);
    write_state(&path, &LockFileState { locked: false, held_by_pid: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn always_alive(_pid: u32) -> bool {
        true
    }
    fn never_alive(_pid: u32) -> bool {
        false
    }

    #[test]
    fn fresh_acquire_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let held = AtomicBool::new(false);
        assert!(try_lock(dir.path(), Domain::Tech, &held, 111, &always_alive).unwrap());
        assert!(held.load(Ordering::SeqCst));
    }

    #[test]
    fn second_independent_process_sees_it_held() {
        let dir = tempfile::tempdir().unwrap();
        let held_a = AtomicBool::new(false);
        let held_b = AtomicBool::new(false);
        assert!(try_lock(dir.path(), Domain::Tech, &held_a, 111, &always_alive).unwrap());
        // A second process (own_pid 222) contending for the same domain: not free.
        assert!(!try_lock(dir.path(), Domain::Tech, &held_b, 222, &always_alive).unwrap());
        assert!(!held_b.load(Ordering::SeqCst));
    }

    #[test]
    fn release_then_reacquire_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let held = AtomicBool::new(false);
        assert!(try_lock(dir.path(), Domain::Business, &held, 111, &always_alive).unwrap());
        assert!(unlock(dir.path(), Domain::Business, &held).unwrap());
        assert!(!held.load(Ordering::SeqCst));
        let held2 = AtomicBool::new(false);
        assert!(try_lock(dir.path(), Domain::Business, &held2, 222, &always_alive).unwrap());
    }

    #[test]
    fn unlock_by_non_holder_is_a_noop() {
        let dir = tempfile::tempdir().unwrap();
        let held_a = AtomicBool::new(false);
        let held_b = AtomicBool::new(false);
        assert!(try_lock(dir.path(), Domain::Tech, &held_a, 111, &always_alive).unwrap());
        // held_b never acquired anything, so releasing it must be a no-op.
        assert!(!unlock(dir.path(), Domain::Tech, &held_b).unwrap());
        // The real holder's lock is untouched.
        let held_c = AtomicBool::new(false);
        assert!(!try_lock(dir.path(), Domain::Tech, &held_c, 333, &always_alive).unwrap());
    }

    #[test]
    fn stale_pid_is_auto_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let held_a = AtomicBool::new(false);
        assert!(try_lock(dir.path(), Domain::Tech, &held_a, 111, &always_alive).unwrap());
        // Simulate the holder's process having crashed: liveness check now says it's dead.
        let held_b = AtomicBool::new(false);
        assert!(try_lock(dir.path(), Domain::Tech, &held_b, 222, &never_alive).unwrap());
        assert!(held_b.load(Ordering::SeqCst));
    }

    #[test]
    fn cli_lock_ignores_in_memory_state_and_inspects_disk_only() {
        let dir = tempfile::tempdir().unwrap();
        assert!(cli_lock(dir.path(), Domain::Business, &always_alive).unwrap());
        // A subsequent MCP try-lock from a live process must see it as held.
        let held = AtomicBool::new(false);
        assert!(!try_lock(dir.path(), Domain::Business, &held, 999, &always_alive).unwrap());
    }

    #[test]
    fn cli_unlock_always_clears_regardless_of_prior_state() {
        let dir = tempfile::tempdir().unwrap();
        let held = AtomicBool::new(false);
        assert!(try_lock(dir.path(), Domain::Tech, &held, 111, &always_alive).unwrap());
        cli_unlock(dir.path(), Domain::Tech).unwrap();
        let held2 = AtomicBool::new(false);
        assert!(try_lock(dir.path(), Domain::Tech, &held2, 222, &always_alive).unwrap());
    }

    #[test]
    fn locking_one_domain_does_not_affect_the_other() {
        let dir = tempfile::tempdir().unwrap();
        let held = AtomicBool::new(false);
        assert!(try_lock(dir.path(), Domain::Tech, &held, 111, &always_alive).unwrap());
        let held_business = AtomicBool::new(false);
        assert!(try_lock(dir.path(), Domain::Business, &held_business, 111, &always_alive).unwrap());
    }
}
