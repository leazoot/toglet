//! Holds off idle sleep while automatic continuation is waiting.
//!
//! Only idle sleep is prevented: a closed lid or a sleep the user asks for still sleeps.
//! The Windows implementation is unverified.

use std::fmt;

use crate::diagnostics::{ErrorCode, Phase, Result, TogletError, UserAction};

/// Keeps the system from idling into sleep until dropped; deliberately has no explicit release.
pub trait PowerHold: Send {}

/// On Windows the state is per thread, so a hold must be acquired and dropped on the same thread.
pub trait PowerAssertion: Send + Sync {
    /// `reason` appears in the platform's assertion listing; a fixed English phrase, never user
    /// data.
    fn acquire(&self, reason: &'static str) -> Result<Box<dyn PowerHold>>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemPowerAssertion;

impl PowerAssertion for SystemPowerAssertion {
    fn acquire(&self, reason: &'static str) -> Result<Box<dyn PowerHold>> {
        platform::acquire(reason)
    }
}

/// Counting fake for tests.
#[derive(Debug, Default)]
pub struct FakePowerAssertion {
    live: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    acquired: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl FakePowerAssertion {
    /// Holds currently open.
    pub fn live(&self) -> usize {
        self.live.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Holds ever taken.
    pub fn acquired(&self) -> usize {
        self.acquired.load(std::sync::atomic::Ordering::SeqCst)
    }
}

struct FakeHold {
    live: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl PowerHold for FakeHold {}

impl Drop for FakeHold {
    fn drop(&mut self) {
        self.live.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

impl PowerAssertion for FakePowerAssertion {
    fn acquire(&self, _reason: &'static str) -> Result<Box<dyn PowerHold>> {
        self.live.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.acquired
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Box::new(FakeHold {
            live: std::sync::Arc::clone(&self.live),
        }))
    }
}

impl fmt::Debug for dyn PowerHold {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PowerHold")
    }
}

fn unavailable(detail: &str) -> TogletError {
    // The scheduler continues without the assertion and the limitation is shown to the user.
    TogletError::new(ErrorCode::Internal, Phase::Storage, false, UserAction::None)
        .with_detail(detail)
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::{CString, c_void};

    use super::{PowerHold, unavailable};
    use crate::diagnostics::Result;

    type CFStringRef = *const c_void;
    type IOPMAssertionID = u32;
    type IOReturn = i32;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    /// `kIOPMAssertionLevelOn`.
    const ASSERTION_ON: u32 = 255;
    const K_IO_RETURN_SUCCESS: IOReturn = 0;
    /// `kIOPMAssertionTypePreventUserIdleSystemSleep`: idle sleep only; display sleep is left
    /// alone.
    const ASSERTION_TYPE: &str = "PreventUserIdleSystemSleep";

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            c_str: *const std::os::raw::c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            assertion_type: CFStringRef,
            assertion_level: u32,
            assertion_name: CFStringRef,
            assertion_id: *mut IOPMAssertionID,
        ) -> IOReturn;
        fn IOPMAssertionRelease(assertion_id: IOPMAssertionID) -> IOReturn;
    }

    struct CfString(CFStringRef);

    impl CfString {
        fn new(text: &str) -> Option<Self> {
            let c_text = CString::new(text).ok()?;
            // SAFETY: `c_text` is NUL-terminated and outlives the call, which copies it; a null
            // allocator means the default one.
            let cf = unsafe {
                CFStringCreateWithCString(
                    std::ptr::null(),
                    c_text.as_ptr(),
                    K_CF_STRING_ENCODING_UTF8,
                )
            };
            (!cf.is_null()).then_some(Self(cf))
        }
    }

    impl Drop for CfString {
        fn drop(&mut self) {
            // SAFETY: `self.0` is a live object this value owns and nothing else references.
            unsafe { CFRelease(self.0) };
        }
    }

    struct Hold(IOPMAssertionID);

    // SAFETY: IOKit assertion ids are process-wide and not tied to the creating thread.
    unsafe impl Send for Hold {}

    impl PowerHold for Hold {}

    impl Drop for Hold {
        fn drop(&mut self) {
            // SAFETY: `self.0` came from a successful create and is released exactly once. A
            // refused release cannot be handled, and process exit releases it anyway.
            unsafe { IOPMAssertionRelease(self.0) };
        }
    }

    pub fn acquire(reason: &'static str) -> Result<Box<dyn PowerHold>> {
        let kind = CfString::new(ASSERTION_TYPE)
            .ok_or_else(|| unavailable("the assertion type string could not be created"))?;
        let name = CfString::new(reason)
            .ok_or_else(|| unavailable("the assertion name string could not be created"))?;
        let mut id: IOPMAssertionID = 0;
        // SAFETY: both strings outlive the call and `id` is a valid out-pointer.
        let status = unsafe { IOPMAssertionCreateWithName(kind.0, ASSERTION_ON, name.0, &mut id) };
        if status != K_IO_RETURN_SUCCESS {
            return Err(unavailable("the system refused the sleep assertion"));
        }
        Ok(Box::new(Hold(id)))
    }
}

#[cfg(windows)]
mod platform {
    //! Unverified: written from the `SetThreadExecutionState` documentation, not run on Windows.

    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };

    use super::{PowerHold, unavailable};
    use crate::diagnostics::Result;

    /// Dropping clears the per-thread state, so it must happen on the acquiring thread.
    struct Hold;

    impl PowerHold for Hold {}

    impl Drop for Hold {
        fn drop(&mut self) {
            // SAFETY: plain Win32 call; setting only `ES_CONTINUOUS` is the documented release.
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
    }

    pub fn acquire(_reason: &'static str) -> Result<Box<dyn PowerHold>> {
        // SAFETY: plain Win32 call with constant flags; returns 0 on failure.
        let previous = unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
        if previous == 0 {
            return Err(unavailable(
                "the system refused the execution-state request",
            ));
        }
        Ok(Box::new(Hold))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::{PowerHold, unavailable};
    use crate::diagnostics::Result;

    /// Unsupported platform: the scheduler runs without holding off sleep.
    pub fn acquire(_reason: &'static str) -> Result<Box<dyn PowerHold>> {
        Err(unavailable("no sleep assertion on this platform"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fake_hold_is_live_until_dropped() {
        let power = FakePowerAssertion::default();
        assert_eq!(power.live(), 0);
        let hold = power.acquire("test").expect("acquired");
        assert_eq!(power.live(), 1);
        assert_eq!(power.acquired(), 1);
        drop(hold);
        assert_eq!(power.live(), 0);
        assert_eq!(power.acquired(), 1);
    }

    // Only creation and release are checked, not the system's assertion listing.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_system_assertion_can_be_taken_and_released() {
        let hold = SystemPowerAssertion
            .acquire("Toglet test assertion")
            .expect("macOS grants idle-sleep assertions to any process");
        drop(hold);
    }
}
