//! Process-memory probes shared by the post-run reports and the memory
//! watchdog. All probes return `None` on unsupported platforms or syscall
//! failure — a report must state `unavailable`/`null` rather than fabricate a
//! number, and the watchdog must never fire on a failed read. macOS reports
//! `phys_footprint` (the Activity Monitor / jetsam figure, which keeps counting
//! compressed pages that leave the resident set) plus the getrusage peak RSS;
//! Linux exposes only the resident set and its high-water mark, and Windows
//! only the working set and its peak, so the footprint fields are `None` there.

pub(crate) use imp::{
    current_footprint_bytes, current_rss_bytes, peak_footprint_bytes, peak_rss_bytes,
};

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::c_void;
    use std::mem::MaybeUninit;

    // Mirrors <mach/task_info.h> `struct task_vm_info` through the rev3
    // fields. The kernel fills fields up to the boundary implied by the
    // caller's count, so requesting through rev3 yields `phys_footprint`
    // (rev1) and `ledger_phys_footprint_peak` (rev3); older kernels report a
    // smaller filled count, which the per-field offset checks turn into `None`.
    #[repr(C)]
    struct TaskVmInfo {
        virtual_size: u64,
        region_count: i32,
        page_size: i32,
        resident_size: u64,
        resident_size_peak: u64,
        device: u64,
        device_peak: u64,
        internal: u64,
        internal_peak: u64,
        external: u64,
        external_peak: u64,
        reusable: u64,
        reusable_peak: u64,
        purgeable_volatile_pmap: u64,
        purgeable_volatile_resident: u64,
        purgeable_volatile_virtual: u64,
        compressed: u64,
        compressed_peak: u64,
        compressed_lifetime: u64,
        phys_footprint: u64,
        min_address: u64,
        max_address: u64,
        ledger_phys_footprint_peak: i64,
        ledger_tail: [i64; 20],
    }

    #[repr(C)]
    struct Timeval {
        tv_sec: i64,
        tv_usec: i32,
    }

    #[repr(C)]
    struct Rusage {
        ru_utime: Timeval,
        ru_stime: Timeval,
        ru_maxrss: i64,
        ru_tail: [i64; 13],
    }

    const RUSAGE_SELF: i32 = 0;
    const TASK_VM_INFO: u32 = 22;
    const KERN_SUCCESS: i32 = 0;

    unsafe extern "C" {
        fn getrusage(who: i32, usage: *mut Rusage) -> i32;
        static mach_task_self_: u32;
        fn task_info(task: u32, flavor: u32, info: *mut c_void, count: *mut u32) -> i32;
    }

    fn task_vm_info() -> Option<(TaskVmInfo, usize)> {
        let mut info = MaybeUninit::<TaskVmInfo>::zeroed();
        let mut count = (std::mem::size_of::<TaskVmInfo>() / 4) as u32;
        let kr = unsafe {
            task_info(
                mach_task_self_,
                TASK_VM_INFO,
                info.as_mut_ptr().cast(),
                &mut count,
            )
        };
        if kr != KERN_SUCCESS {
            return None;
        }
        Some((unsafe { info.assume_init() }, count as usize * 4))
    }

    pub(crate) fn current_footprint_bytes() -> Option<u64> {
        let (info, filled) = task_vm_info()?;
        let end = std::mem::offset_of!(TaskVmInfo, phys_footprint) + 8;
        (filled >= end).then_some(info.phys_footprint)
    }

    pub(crate) fn current_rss_bytes() -> Option<u64> {
        let (info, filled) = task_vm_info()?;
        let end = std::mem::offset_of!(TaskVmInfo, resident_size) + 8;
        (filled >= end).then_some(info.resident_size)
    }

    pub(crate) fn peak_footprint_bytes() -> Option<u64> {
        let (info, filled) = task_vm_info()?;
        let end = std::mem::offset_of!(TaskVmInfo, ledger_phys_footprint_peak) + 8;
        if filled < end {
            return None;
        }
        u64::try_from(info.ledger_phys_footprint_peak).ok()
    }

    pub(crate) fn peak_rss_bytes() -> Option<u64> {
        let mut usage = MaybeUninit::<Rusage>::zeroed();
        if unsafe { getrusage(RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return None;
        }
        // BSD getrusage reports ru_maxrss in bytes (Linux reports kilobytes).
        u64::try_from(unsafe { usage.assume_init() }.ru_maxrss).ok()
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub(crate) fn current_footprint_bytes() -> Option<u64> {
        None
    }

    pub(crate) fn peak_footprint_bytes() -> Option<u64> {
        None
    }

    fn status_field_bytes(field: &str) -> Option<u64> {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let line = status.lines().find_map(|line| line.strip_prefix(field))?;
        let kb: u64 = line.trim().trim_end_matches("kB").trim().parse().ok()?;
        Some(kb * 1024)
    }

    pub(crate) fn current_rss_bytes() -> Option<u64> {
        status_field_bytes("VmRSS:")
    }

    pub(crate) fn peak_rss_bytes() -> Option<u64> {
        status_field_bytes("VmHWM:")
    }
}

#[cfg(windows)]
mod imp {
    use std::mem::MaybeUninit;

    // Mirrors <psapi.h> `PROCESS_MEMORY_COUNTERS`. The working set is the
    // resident set; like Linux there is no footprint figure that also counts
    // paged-out memory, so the footprint fields are `None`.
    #[repr(C)]
    struct ProcessMemoryCounters {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> *mut std::ffi::c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut std::ffi::c_void,
            counters: *mut ProcessMemoryCounters,
            cb: u32,
        ) -> i32;
    }

    fn memory_counters() -> Option<ProcessMemoryCounters> {
        let size = std::mem::size_of::<ProcessMemoryCounters>() as u32;
        let mut counters = MaybeUninit::<ProcessMemoryCounters>::zeroed();
        let ok = unsafe {
            K32GetProcessMemoryInfo(GetCurrentProcess(), counters.as_mut_ptr(), size)
        };
        (ok != 0).then(|| unsafe { counters.assume_init() })
    }

    pub(crate) fn current_footprint_bytes() -> Option<u64> {
        None
    }

    pub(crate) fn peak_footprint_bytes() -> Option<u64> {
        None
    }

    pub(crate) fn current_rss_bytes() -> Option<u64> {
        memory_counters().map(|counters| counters.working_set_size as u64)
    }

    pub(crate) fn peak_rss_bytes() -> Option<u64> {
        memory_counters().map(|counters| counters.peak_working_set_size as u64)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod imp {
    pub(crate) fn current_footprint_bytes() -> Option<u64> {
        None
    }

    pub(crate) fn peak_footprint_bytes() -> Option<u64> {
        None
    }

    pub(crate) fn current_rss_bytes() -> Option<u64> {
        None
    }

    pub(crate) fn peak_rss_bytes() -> Option<u64> {
        None
    }
}
