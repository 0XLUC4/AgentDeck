//! Single instance: a second launch pings the running AgentDeck, which shows its window, then exits.

use std::net::{TcpListener, TcpStream};
use std::sync::OnceLock;
use std::time::Duration;

use gpui_kit::*;

/// Port held for the app lifetime: a second instance fails to bind.
/// shortcut: a TCP port as the instance lock; switch to a named mutex if the port ever collides.
const INSTANCE_PORT: u16 = 47799;

static LISTENER: OnceLock<TcpListener> = OnceLock::new();
/// Passed to the new exe by the updater: the old instance still holds the port while it quits.
pub(super) const AFTER_UPDATE: &str = "--after-update";

/// Returns the lock to keep alive for the whole run, or None when AgentDeck already runs
/// (the running instance is asked to come to the front).
pub fn single_instance() -> Option<TcpListener> {
    // The 0.2.0 updater starts the downloaded exe from %TEMP% as an NSIS installer (`/P /R`).
    let legacy_update = std::env::args().any(|a| a == "/R");
    // Read now: the old instance is about to quit, and its path is where this exe belongs.
    let old_exe = if legacy_update { other_instance_exe() } else { None };
    let mut bound = TcpListener::bind(("127.0.0.1", INSTANCE_PORT));
    if legacy_update || std::env::args().any(|a| a == AFTER_UPDATE) {
        for _ in 0..200 {
            // Up to 20 s: the old instance stops its agents before it frees the port.
            if bound.is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
            bound = TcpListener::bind(("127.0.0.1", INSTANCE_PORT));
        }
    }
    if bound.is_ok()
        && let Some(old_exe) = old_exe
        && take_place_of(&old_exe)
    {
        std::process::exit(0); // frees the port for the relaunched exe
    }
    match bound {
        Ok(listener) => {
            if let Ok(clone) = listener.try_clone() {
                let _ = LISTENER.set(clone);
            }
            Some(listener)
        }
        Err(_) => {
            let addr = ([127, 0, 0, 1], INSTANCE_PORT).into();
            if let Err(e) = TcpStream::connect_timeout(&addr, Duration::from_secs(1)) {
                eprintln!("cannot reach the running AgentDeck: {e}");
            }
            None
        }
    }
}

/// Copies this exe over `target` (retrying while the old process lets go of its file), then starts
/// it. False when the copy fails: this exe keeps running from where it is.
fn take_place_of(target: &std::path::Path) -> bool {
    let Ok(current) = std::env::current_exe() else { return false };
    for _ in 0..30 {
        if std::fs::copy(&current, target).is_ok() {
            return std::process::Command::new(target).arg(AFTER_UPDATE).spawn().is_ok();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// Path of another running agentdeck.exe (the instance holding the port), if any.
#[cfg(windows)]
fn other_instance_exe() -> Option<std::path::PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };
    let current = std::env::current_exe().ok()?;
    // SAFETY: snapshot and process handles are closed below; buffers are sized for the calls.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = None;
        let mut more = Process32FirstW(snapshot, &mut entry) != 0;
        while more && found.is_none() {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case("agentdeck.exe") && entry.th32ProcessID != GetCurrentProcessId() {
                let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                if !process.is_null() {
                    let mut buffer = [0u16; 1024];
                    let mut size = buffer.len() as u32;
                    if QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut size) != 0 {
                        let path = std::path::PathBuf::from(std::ffi::OsString::from_wide(&buffer[..size as usize]));
                        if path != current {
                            found = Some(path);
                        }
                    }
                    CloseHandle(process);
                }
            }
            more = Process32NextW(snapshot, &mut entry) != 0;
        }
        CloseHandle(snapshot);
        found
    }
}

#[cfg(not(windows))]
fn other_instance_exe() -> Option<std::path::PathBuf> {
    None
}

/// Every connection to the instance port means "another launch": show the main window.
pub(super) fn listen(cx: &mut App) {
    let Some(listener) = LISTENER.get() else {
        return;
    };
    let Ok(listener) = listener.try_clone() else {
        return;
    };
    let (tx, rx) = smol::channel::unbounded::<()>();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            if stream.is_ok() && tx.send_blocking(()).is_err() {
                break;
            }
        }
    });
    cx.spawn(async move |cx| {
        while rx.recv().await.is_ok() {
            cx.update(super::show_main);
        }
    })
    .detach();
}
