use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::lock::IsAlive;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct InstanceLockState {
    pid: u32,
}

fn instance_lock_path(root: &Path) -> PathBuf {
    root.join(".brained").join("instance.lock")
}

fn read_pid(path: &Path) -> anyhow::Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let state: InstanceLockState =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(state.pid))
}

fn write_pid(path: &Path, pid: u32) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let text = serde_json::to_string(&InstanceLockState { pid }).context("serializing instance lock state")?;
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// Held for the lifetime of a `brd scan`/`brd mcp` process against one repo root. Only one of
/// either command may run against the same root at a time — `scan_once` isn't safe to run
/// concurrently against the same `.brained/state.db`/lancedb directory from two processes (the
/// exact scenario this guards against: a manual `brd scan` racing an already-running `brd mcp`
/// spawned by Claude Code). Dropping the guard releases the lock; a killed/crashed process
/// (SIGKILL, SIGILL, etc.) skips that and instead relies on the next `acquire`'s PID-liveness
/// check to reclaim it, mirroring the domain locks in `lock.rs`.
#[derive(Debug)]
pub struct InstanceLock {
    path: PathBuf,
}

impl InstanceLock {
    pub fn acquire(root: &Path, own_pid: u32, is_alive: IsAlive) -> anyhow::Result<InstanceLock> {
        let path = instance_lock_path(root);
        if let Some(existing_pid) = read_pid(&path)? {
            if is_alive(existing_pid) {
                anyhow::bail!(
                    "another `brd` process (pid {existing_pid}) is already running against this repo; \
                     only one `brd scan`/`brd mcp` instance may run per repo at a time"
                );
            }
        }
        write_pid(&path, own_pid)?;
        Ok(InstanceLock { path })
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
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
        let _guard = InstanceLock::acquire(dir.path(), 111, &always_alive).unwrap();
        assert!(instance_lock_path(dir.path()).exists());
    }

    #[test]
    fn second_live_process_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = InstanceLock::acquire(dir.path(), 111, &always_alive).unwrap();
        let err = InstanceLock::acquire(dir.path(), 222, &always_alive).unwrap_err();
        assert!(err.to_string().contains("pid 111"));
    }

    #[test]
    fn dropping_the_guard_releases_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let guard = InstanceLock::acquire(dir.path(), 111, &always_alive).unwrap();
        drop(guard);
        assert!(!instance_lock_path(dir.path()).exists());
        let _guard2 = InstanceLock::acquire(dir.path(), 222, &always_alive).unwrap();
    }

    #[test]
    fn stale_pid_is_auto_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let guard = InstanceLock::acquire(dir.path(), 111, &always_alive).unwrap();
        // Simulate the holder crashing without cleanup: leak the guard so its file stays put.
        std::mem::forget(guard);
        let _guard2 = InstanceLock::acquire(dir.path(), 222, &never_alive).unwrap();
    }
}
