//! Data-directory exclusive lock and process-handoff helpers.

use super::*;

pub(super) const DEFAULT_DATA_LOCK_WAIT: Duration = Duration::from_secs(30);

pub(super) const PRE_R5_PARENT_HANDOFF_WAIT: Duration = Duration::from_secs(60);

pub(super) const STARTUP_WAIT_POLL: Duration = Duration::from_millis(50);

pub(super) struct DataDirLock {
    file: File,
    _path: PathBuf,
}

impl DataDirLock {
    /// Private to `lock.rs`: the only production path to a held data-dir
    /// lock is [`acquire_locked_database`], so the lock -> parent handoff ->
    /// open sequence cannot be skipped by acquiring the lock directly.
    fn acquire(database_path: &Path, wait: Duration) -> GaryxDbResult<Self> {
        let data_dir = database_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(data_dir)?;
        let lock_path = data_dir.join("garyx.lock");
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        set_close_on_exec(&file)?;
        acquire_exclusive_flock(&file, &lock_path, wait)?;

        // The advisory lock is the authority; the PID is diagnostic data for
        // operators and deterministic restart tests only.
        file.set_len(0)?;
        file.write_all(std::process::id().to_string().as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(Self {
            file,
            _path: lock_path,
        })
    }

    #[cfg(any(test, feature = "test-seams"))]
    pub(super) fn close_on_exec(&self) -> GaryxDbResult<bool> {
        close_on_exec_is_set(&self.file)
    }
}

#[cfg(test)]
impl DataDirLock {
    /// Test-only lock handle for the contention and fail-closed behavior
    /// tests. Additive seam: production code cannot name it, so the
    /// `acquire_locked_database` funnel stays the only production path.
    pub(super) fn acquire_for_tests(database_path: &Path, wait: Duration) -> GaryxDbResult<Self> {
        Self::acquire(database_path, wait)
    }
}

impl Drop for DataDirLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        // Closing the file would release flock as well; the explicit unlock
        // makes the ownership boundary clear and lets a waiter proceed before
        // any later field-drop work.
        unsafe {
            libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

#[cfg(unix)]
pub(super) fn set_close_on_exec(file: &File) -> GaryxDbResult<()> {
    let fd = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error().into());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn set_close_on_exec(_file: &File) -> GaryxDbResult<()> {
    Err(GaryxDbError::Configuration(
        "per-data-dir flock is only supported on Unix".to_owned(),
    ))
}

#[cfg(all(unix, any(test, feature = "test-seams")))]
pub(super) fn close_on_exec_is_set(file: &File) -> GaryxDbResult<bool> {
    let flags = unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETFD) };
    if flags < 0 {
        return Err(io::Error::last_os_error().into());
    }
    Ok(flags & libc::FD_CLOEXEC != 0)
}

#[cfg(all(not(unix), any(test, feature = "test-seams")))]
pub(super) fn close_on_exec_is_set(_file: &File) -> GaryxDbResult<bool> {
    Ok(false)
}

#[cfg(unix)]
pub(super) fn acquire_exclusive_flock(
    file: &File,
    lock_path: &Path,
    wait: Duration,
) -> GaryxDbResult<()> {
    let started = Instant::now();
    loop {
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        match error.raw_os_error() {
            Some(code) if code == libc::EINTR => continue,
            Some(code) if code == libc::EWOULDBLOCK || code == libc::EAGAIN => {
                let elapsed = started.elapsed();
                if elapsed >= wait {
                    return Err(GaryxDbError::DataDirLocked {
                        path: lock_path.to_path_buf(),
                        wait_secs: wait.as_secs(),
                    });
                }
                std::thread::sleep(STARTUP_WAIT_POLL.min(wait.saturating_sub(elapsed)));
            }
            _ => return Err(error.into()),
        }
    }
}

#[cfg(not(unix))]
pub(super) fn acquire_exclusive_flock(
    _file: &File,
    _lock_path: &Path,
    _wait: Duration,
) -> GaryxDbResult<()> {
    Err(GaryxDbError::Configuration(
        "per-data-dir flock is only supported on Unix".to_owned(),
    ))
}

pub(super) fn configured_data_lock_wait() -> GaryxDbResult<Duration> {
    let Some(raw) = std::env::var_os("GARYX_DATA_LOCK_WAIT_SECS") else {
        return Ok(DEFAULT_DATA_LOCK_WAIT);
    };
    let raw = raw.to_string_lossy();
    let seconds = raw.trim().parse::<u64>().map_err(|_| {
        GaryxDbError::Configuration(
            "GARYX_DATA_LOCK_WAIT_SECS must be a non-negative integer".to_owned(),
        )
    })?;
    Ok(Duration::from_secs(seconds))
}

/// The held data-dir lock plus the first SQLite connection of its database.
///
/// Obtainable only through [`acquire_locked_database`], whose body is the
/// startup sequence lock -> pre-R5 parent handoff -> open. The handoff
/// barrier itself is private to this module, so code outside `lock.rs` can
/// neither invoke it out of order nor skip it while still presenting a
/// locked database. The fail-closed behavior test (failed handoff leaves the
/// database untouched and releases the lock) pins the observable property.
pub(super) struct LockedDatabase {
    pub(super) lock: DataDirLock,
    pub(super) conn: Connection,
}

/// The only way to obtain an on-disk SQLite connection under the data-dir
/// lock: acquire the lock, run the pre-R5 parent handoff barrier, then open.
pub(super) fn acquire_locked_database(
    path: &Path,
    lock_wait: Duration,
) -> GaryxDbResult<LockedDatabase> {
    let lock = DataDirLock::acquire(path, lock_wait)?;
    wait_for_pre_r5_parent_handoff()?;
    let conn = Connection::open(path)?;
    Ok(LockedDatabase { lock, conn })
}

#[cfg(unix)]
fn wait_for_pre_r5_parent_handoff() -> GaryxDbResult<()> {
    let parent_pid = unsafe { libc::getppid() };
    if parent_pid <= 1 || !parent_has_same_executable_name(parent_pid as u32)? {
        return Ok(());
    }
    wait_for_parent_exit(parent_pid as u32, PRE_R5_PARENT_HANDOFF_WAIT, || {
        process_is_alive(parent_pid as u32)
    })
}

#[cfg(not(unix))]
fn wait_for_pre_r5_parent_handoff() -> GaryxDbResult<()> {
    Ok(())
}

pub(super) fn wait_for_parent_exit(
    parent_pid: u32,
    wait: Duration,
    mut is_alive: impl FnMut() -> bool,
) -> GaryxDbResult<()> {
    let started = Instant::now();
    loop {
        if !is_alive() {
            return Ok(());
        }
        let elapsed = started.elapsed();
        if elapsed >= wait {
            return Err(GaryxDbError::ParentHandoffTimedOut {
                parent_pid,
                wait_secs: wait.as_secs(),
            });
        }
        std::thread::sleep(STARTUP_WAIT_POLL.min(wait.saturating_sub(elapsed)));
    }
}

#[cfg(unix)]
pub(super) fn process_is_alive(pid: u32) -> bool {
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return true;
    }
    // EPERM still proves the process exists. Unknown errors are treated as
    // alive so the handoff barrier fails closed.
    !matches!(io::Error::last_os_error().raw_os_error(), Some(libc::ESRCH))
}

#[cfg(unix)]
pub(super) fn parent_has_same_executable_name(parent_pid: u32) -> GaryxDbResult<bool> {
    parent_has_same_executable_name_with(parent_pid, parent_executable_path)
}

#[cfg(unix)]
pub(super) fn parent_has_same_executable_name_with(
    parent_pid: u32,
    resolve_parent: impl FnOnce(u32) -> GaryxDbResult<PathBuf>,
) -> GaryxDbResult<bool> {
    let current = std::env::current_exe()?;
    let current_name = current.file_name().ok_or_else(|| {
        GaryxDbError::Configuration(format!(
            "current executable path has no file name: {}",
            current.display()
        ))
    })?;
    let parent = resolve_parent(parent_pid)?;
    let parent_name = parent.file_name().ok_or_else(|| {
        GaryxDbError::Configuration(format!(
            "parent executable path has no file name: {}",
            parent.display()
        ))
    })?;
    Ok(parent_name == current_name)
}

#[cfg(target_os = "linux")]
pub(super) fn parent_executable_path(parent_pid: u32) -> GaryxDbResult<PathBuf> {
    Ok(std::fs::read_link(format!("/proc/{parent_pid}/exe"))?)
}

#[cfg(all(unix, not(target_os = "linux")))]
pub(super) fn parent_executable_path(parent_pid: u32) -> GaryxDbResult<PathBuf> {
    let output = std::process::Command::new("ps")
        .args(["-p", &parent_pid.to_string(), "-o", "comm="])
        .output()
        .map_err(|error| {
            GaryxDbError::Configuration(format!(
                "failed to resolve parent executable with ps for pid {parent_pid}: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(GaryxDbError::Configuration(format!(
            "ps failed while resolving parent executable for pid {parent_pid}: {}",
            output.status
        )));
    }
    let path = String::from_utf8(output.stdout).map_err(|error| {
        GaryxDbError::Configuration(format!(
            "ps returned non-UTF-8 parent executable for pid {parent_pid}: {error}"
        ))
    })?;
    let path = path.trim();
    if path.is_empty() {
        return Err(GaryxDbError::Configuration(format!(
            "ps returned an empty parent executable for pid {parent_pid}"
        )));
    }
    Ok(PathBuf::from(path))
}
