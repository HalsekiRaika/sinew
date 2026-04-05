//! Cross-platform process liveness checking.
//!
//! Provides `is_process_alive(pid)` using OS-native APIs:
//! - Unix: `kill(pid, 0)` (signal 0 checks existence without sending a signal)
//! - Windows: `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)`

/// Check if a process with the given PID is alive.
#[cfg(unix)]
pub fn is_process_alive(pid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;

    let pid = Pid::from_raw(pid as i32);
    // Signal 0: no signal sent, but checks existence + permissions.
    // Ok(()) = alive, Err(EPERM) = alive but no permission, Err(ESRCH) = dead.
    match kill(pid, None) {
        Ok(()) => true,
        Err(nix::errno::Errno::EPERM) => true,
        Err(_) => false,
    }
}

/// Check if a process with the given PID is alive.
#[cfg(windows)]
pub fn is_process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if !handle.is_null() {
            CloseHandle(handle);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_process_is_alive() {
        let pid = std::process::id();
        assert!(is_process_alive(pid));
    }

    #[test]
    fn nonexistent_pid_is_dead() {
        // PID 0 is the system idle process (Windows) or kernel (Unix).
        // Use a very high PID that is extremely unlikely to exist.
        assert!(!is_process_alive(u32::MAX - 1));
    }
}
