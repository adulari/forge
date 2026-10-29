//! Windows: tie every shell child to the daemon's lifetime.
//!
//! A Windows child is not killed when its parent dies, so a crashed or killed `forge serve` left
//! dev servers and build tools running with nothing to stop them. Children are put in one job
//! object created with `KILL_ON_JOB_CLOSE`; when the daemon exits, however it exits, the handle
//! closes and Windows ends everything still in the job. On every other platform this is a no-op
//! (Unix children are already killed through their process group).

#[cfg(windows)]
mod imp {
    use std::mem::size_of;
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    struct Job(HANDLE);

    // SAFETY: a job-object handle is a process-wide kernel handle; the OS serialises its use.
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    fn job() -> Option<&'static Job> {
        static JOB: OnceLock<Option<Job>> = OnceLock::new();
        JOB.get_or_init(|| {
            // SAFETY: null attributes and name are documented as valid; the returned handle is
            // owned by this static and never closed, so the OS closes it when the process ends.
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle.is_null() {
                return None;
            }
            // SAFETY: zeroed is a valid value for this plain-data struct.
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `info` outlives the call and the size matches the information class.
            let set = unsafe {
                SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    (&raw const info).cast(),
                    size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                )
            };
            (set != 0).then_some(Job(handle))
        })
        .as_ref()
    }

    pub(crate) fn contain(child: &tokio::process::Child) {
        let (Some(job), Some(process)) = (job(), child.raw_handle()) else {
            return;
        };
        // SAFETY: both handles are valid; a failure (the process is already gone, or is in an
        // incompatible job on an old Windows) just leaves it uncontained.
        unsafe { AssignProcessToJobObject(job.0, process as HANDLE) };
    }
}

/// Put a freshly spawned child in the daemon-lifetime job. No-op off Windows.
#[cfg(windows)]
pub(crate) fn contain(child: &tokio::process::Child) {
    imp::contain(child);
}

#[cfg(not(windows))]
pub(crate) fn contain(_child: &tokio::process::Child) {}
