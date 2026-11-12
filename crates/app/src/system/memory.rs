//! RAM used by AgentDeck itself, for the status bar.

/// Working set of this process in bytes (what Task Manager shows), `None` when unavailable.
#[cfg(windows)]
pub fn working_set() -> Option<u64> {
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::GetCurrentProcess;
    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: the pseudo handle needs no closing; `counters` is sized by `cb`.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, size) } != 0;
    ok.then_some(counters.WorkingSetSize as u64)
}

#[cfg(not(windows))]
pub fn working_set() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(windows)]
    fn reads_own_working_set() {
        assert!(super::working_set().is_some_and(|bytes| bytes > 1024 * 1024));
    }
}
