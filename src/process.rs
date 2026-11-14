//! Small questions about other processes, for the updater.

use std::path::PathBuf;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, HLOCAL, LocalFree, WAIT_OBJECT_0};
use windows::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows::Win32::Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, OpenMutexW, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, QueryFullProcessImageNameW, SYNCHRONIZATION_SYNCHRONIZE,
    WaitForSingleObject,
};
use windows::core::{PWSTR, w};

/// Where the exe of a running process lives.
pub fn image_path(pid: u32) -> Option<PathBuf> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let read = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        read.ok()?;
        Some(PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
    }
}

/// When a process started, which tells it from a later one given the same id.
pub fn started(pid: u32) -> Option<u64> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let (mut start, mut exit, mut kernel, mut user) = Default::default();
        let ok = GetProcessTimes(process, &mut start, &mut exit, &mut kernel, &mut user).is_ok();
        let _ = CloseHandle(process);
        let start: FILETIME = start;
        ok.then(|| (u64::from(start.dwHighDateTime) << 32) | u64::from(start.dwLowDateTime))
    }
}

/// The user a process runs as, as the text form of its SID.
pub fn user_sid(process: HANDLE) -> Option<String> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut len = 0u32;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut len);
        // u64 so the buffer is aligned for the pointer inside TOKEN_USER.
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        let read = GetTokenInformation(token, TokenUser, Some(buf.as_mut_ptr().cast()), len, &mut len);
        let _ = CloseHandle(token);
        read.ok()?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut text = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut text).ok()?;
        let sid = text.to_string().ok();
        let _ = LocalFree(Some(HLOCAL(text.0.cast())));
        sid
    }
}

/// The process runs as the same user as this one.
pub fn runs_as_us(pid: u32) -> bool {
    unsafe {
        let Some(ours) = user_sid(GetCurrentProcess()) else { return false };
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let theirs = user_sid(process);
        let _ = CloseHandle(process);
        theirs.is_some_and(|sid| sid == ours)
    }
}

/// A tray is running in this session: it holds the single-instance mutex.
pub fn tray_running() -> bool {
    unsafe {
        let Ok(mutex) = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, w!("Local\\Seam.SingleInstance")) else {
            return false;
        };
        let _ = CloseHandle(mutex);
        true
    }
}

/// Wait for a process to exit. A process that can't be opened is already gone.
pub fn wait_for_exit(pid: u32, timeout_ms: u32) -> bool {
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else { return true };
        let exited = WaitForSingleObject(process, timeout_ms) == WAIT_OBJECT_0;
        let _ = CloseHandle(process);
        exited
    }
}
