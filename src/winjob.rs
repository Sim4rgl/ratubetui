//! Tie every child process we spawn to our own lifetime on Windows.
//!
//! Windows does **not** kill child processes when their parent dies — close
//! the terminal hosting ytui and, without this, mpv (and any in-flight
//! yt-dlp) keeps running. The fix is a job object configured with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. We:
//!
//!   1. create the job,
//!   2. set the kill-on-close policy,
//!   3. assign **our own** process to the job.
//!
//! On Windows 8+, any process we then spawn (with default flags) is
//! automatically a member of our job too. When our process exits — clean
//! shutdown, panic, taskkill /f, terminal close — the OS closes the last
//! handle to the job, the kill-on-close policy fires, and every process in
//! the job is terminated by the kernel.

use anyhow::{anyhow, Result};
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::Console::{
    SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_CLOSE_EVENT, CTRL_C_EVENT, CTRL_LOGOFF_EVENT,
    CTRL_SHUTDOWN_EVENT,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectBasicLimitInformation,
    SetInformationJobObject, JOBOBJECT_BASIC_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, TerminateProcess, PROCESS_TERMINATE,
};

pub struct KillOnExit {
    handle: HANDLE,
}

impl KillOnExit {
    pub fn install() -> Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(anyhow!("CreateJobObjectW returned NULL"));
        }

        let mut info: JOBOBJECT_BASIC_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectBasicLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_BASIC_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            unsafe { CloseHandle(handle) };
            return Err(anyhow!("SetInformationJobObject failed"));
        }

        let me = unsafe { GetCurrentProcess() };
        let ok = unsafe { AssignProcessToJobObject(handle, me) };
        if ok == 0 {
            unsafe { CloseHandle(handle) };
            return Err(anyhow!(
                "AssignProcessToJobObject failed on current process \
                 (parent process may already be in an exclusive job)"
            ));
        }

        Ok(Self { handle })
    }
}

impl Drop for KillOnExit {
    fn drop(&mut self) {
        // Closing the last handle to the job triggers KILL_ON_JOB_CLOSE for
        // every process still in it.
        unsafe { CloseHandle(self.handle) };
    }
}

// HANDLE is `*mut c_void`. We only ever call CloseHandle on it (which is
// thread-safe), so it is safe to move/share across threads.
unsafe impl Send for KillOnExit {}
unsafe impl Sync for KillOnExit {}

/// PID of mpv, read by the console-control handler. 0 means "not set".
static MPV_PID: AtomicU32 = AtomicU32::new(0);

/// Tell the console-control handler which mpv to kill when the terminal
/// closes.
pub fn set_mpv_pid(pid: u32) {
    MPV_PID.store(pid, Ordering::SeqCst);
}

/// Register a console-control handler that kills mpv when the terminal is
/// closed (or Ctrl-C / log-off / shutdown is received), as a fallback for
/// the job-object kill-on-close. The two mechanisms are independent — if
/// either works on this system, mpv dies.
pub fn install_ctrl_handler() -> Result<()> {
    let ok = unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
    if ok == 0 {
        return Err(anyhow!("SetConsoleCtrlHandler failed"));
    }
    Ok(())
}

extern "system" fn handler(event: u32) -> i32 {
    if matches!(
        event,
        CTRL_CLOSE_EVENT
            | CTRL_LOGOFF_EVENT
            | CTRL_SHUTDOWN_EVENT
            | CTRL_C_EVENT
            | CTRL_BREAK_EVENT
    ) {
        kill_mpv();
    }
    // For CTRL_C / CTRL_BREAK, returning 0 (FALSE) lets default handling
    // take over and terminate us. For CTRL_CLOSE / CTRL_LOGOFF /
    // CTRL_SHUTDOWN the OS terminates us anyway after this returns. Either
    // way the job-object kill-on-close fires too.
    0
}

fn kill_mpv() {
    let pid = MPV_PID.load(Ordering::SeqCst);
    if pid == 0 {
        return;
    }
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}
