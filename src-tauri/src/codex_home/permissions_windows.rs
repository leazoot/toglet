//! Windows implementation: a protected DACL granting the current user and nobody else.
//!
//! `Everyone` and `Users` are excluded by construction rather than by removal - the DACL is
//! built from scratch and contains exactly one access-allowed ACE, for the token user of the
//! running process.
//!
//! `SE_DACL_PROTECTED` is not optional here. When a security descriptor is supplied at object
//! creation, Windows still merges the parent directory's inheritable ACEs into it unless the
//! DACL is marked protected. Without that flag a temporary directory under `%TEMP%` would
//! silently inherit whatever the parent grants.

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAceEx, GetLengthSid,
    GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor, PSECURITY_DESCRIPTOR,
    SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR, SetSecurityDescriptorControl,
    SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_WRITE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Full control over the created object, granted to the token user only.
const FILE_ALL_ACCESS: u32 = 0x001F_01FF;

/// The only security descriptor revision Windows accepts. `windows-sys` does not re-export
/// the `SECURITY_DESCRIPTOR_REVISION` constant, and it is fixed at 1 by the ABI.
const SECURITY_DESCRIPTOR_REVISION: u32 = 1;

pub(super) fn create_private_dir(path: &Path) -> io::Result<()> {
    let wide = to_wide(path);
    with_private_security_attributes(|attributes| {
        // SAFETY: `wide` is NUL-terminated and `attributes` points at a SECURITY_ATTRIBUTES
        // that outlives the call.
        if unsafe { CreateDirectoryW(wide.as_ptr(), attributes) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })?
}

pub(super) fn create_private_file(path: &Path) -> io::Result<File> {
    let wide = to_wide(path);
    // Share mode 0: nothing else may open the file while Toglet is writing it. CREATE_NEW
    // means the call fails rather than truncating a file somebody else placed here.
    let handle = with_private_security_attributes(|attributes| {
        // SAFETY: as above; the template handle is optional and passed as null.
        unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_GENERIC_WRITE,
                0,
                attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        }
    })?;
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: the handle is a freshly created, exclusively owned file handle. Ownership moves
    // into `File`, which closes it exactly once on drop.
    Ok(unsafe { File::from_raw_handle(handle) })
}

/// Builds a security descriptor granting only the current token user, and hands a pointer to
/// it to `f`. The descriptor and the ACL it references are alive for the duration of the call
/// and dropped immediately afterwards.
fn with_private_security_attributes<R>(
    f: impl FnOnce(*const SECURITY_ATTRIBUTES) -> R,
) -> io::Result<R> {
    let token_user = TokenUserInfo::current()?;
    let sid = token_user.sid();

    // SAFETY: `sid` points into a TOKEN_USER the OS just filled in.
    let sid_len = unsafe { GetLengthSid(sid) } as usize;
    // An ACCESS_ALLOWED_ACE ends with the first DWORD of the SID, so the SID length replaces
    // that member rather than adding to it.
    let acl_len = size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + sid_len;
    // An ACL must be DWORD-aligned. A Vec<u8> gives no such guarantee; a Vec<u32> does.
    let mut acl_buffer = vec![0u32; acl_len.div_ceil(size_of::<u32>())];
    let acl_bytes = u32::try_from(acl_buffer.len() * size_of::<u32>())
        .map_err(|_| io::Error::other("ACL larger than a DWORD can describe"))?;
    let acl = acl_buffer.as_mut_ptr().cast::<ACL>();

    // SAFETY: `acl` points at a correctly sized and aligned buffer, and `sid` at a valid SID.
    // No ACE inheritance flags are set, so the ACE applies to this object only.
    unsafe {
        if InitializeAcl(acl, acl_bytes, ACL_REVISION) == 0 {
            return Err(io::Error::last_os_error());
        }
        if AddAccessAllowedAceEx(acl, ACL_REVISION, 0, FILE_ALL_ACCESS, sid) == 0 {
            return Err(io::Error::last_os_error());
        }
    }

    // SAFETY: an absolute security descriptor initialised in place; the DACL pointer it stores
    // refers to `acl_buffer`, whose heap allocation outlives the call to `f`.
    let mut descriptor = unsafe { std::mem::zeroed::<SECURITY_DESCRIPTOR>() };
    let descriptor_ptr: PSECURITY_DESCRIPTOR = (&raw mut descriptor).cast();
    unsafe {
        if InitializeSecurityDescriptor(descriptor_ptr, SECURITY_DESCRIPTOR_REVISION) == 0 {
            return Err(io::Error::last_os_error());
        }
        if SetSecurityDescriptorDacl(descriptor_ptr, 1, acl, 0) == 0 {
            return Err(io::Error::last_os_error());
        }
        if SetSecurityDescriptorControl(descriptor_ptr, SE_DACL_PROTECTED, SE_DACL_PROTECTED) == 0 {
            return Err(io::Error::last_os_error());
        }
    }

    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
            .map_err(|_| io::Error::other("SECURITY_ATTRIBUTES size overflow"))?,
        lpSecurityDescriptor: descriptor_ptr,
        bInheritHandle: 0,
    };
    Ok(f(&raw const attributes))
}

/// The SID of the process token, kept alive by the buffer the OS wrote it into.
struct TokenUserInfo {
    // u64 elements so the allocation is pointer-aligned: TOKEN_USER contains a pointer.
    buffer: Vec<u64>,
}

impl TokenUserInfo {
    fn current() -> io::Result<Self> {
        let mut token: HANDLE = ptr::null_mut();
        // SAFETY: GetCurrentProcess returns a pseudo-handle that must not be closed; `token`
        // is a valid out-parameter.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = TokenHandle(token);

        let mut needed = 0u32;
        // First call is expected to fail with ERROR_INSUFFICIENT_BUFFER; only `needed` matters.
        // SAFETY: a null buffer with length 0 is the documented way to ask for the size.
        unsafe {
            GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &raw mut needed);
        }
        let mut buffer = vec![0u64; (needed as usize).div_ceil(size_of::<u64>()).max(1)];

        // SAFETY: the buffer is at least `needed` bytes and correctly aligned for TOKEN_USER.
        let ok = unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &raw mut needed,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { buffer })
    }

    fn sid(&self) -> *mut c_void {
        // SAFETY: the buffer holds a TOKEN_USER the OS wrote, so `User.Sid` points into it.
        unsafe { (*self.buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid }
    }
}

struct TokenHandle(HANDLE);

impl Drop for TokenHandle {
    fn drop(&mut self) {
        // SAFETY: the handle came from OpenProcessToken and is closed exactly once.
        unsafe { CloseHandle(self.0) };
    }
}

fn to_wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Reads the DACL and reports whether it grants the current user and nobody else.
///
/// Production code, not a test helper: the same reading is what a diagnostics run needs in
/// order to say honestly whether a stored credential is still protected.
pub(super) fn is_private(path: &Path) -> io::Result<bool> {
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetSecurityDescriptorControl,
    };

    let wide = to_wide(path);
    let mut dacl: *mut ACL = ptr::null_mut();
    let mut descriptor: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: all out-parameters are valid; the owner, group and SACL are not requested.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            ptr::null_mut(),
            ptr::null_mut(),
            &raw mut dacl,
            ptr::null_mut(),
            &raw mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    // Frees the descriptor on every path below, including the early returns.
    let descriptor = LocalDescriptor(descriptor);

    // A NULL DACL is not "no permissions"; it grants everyone full access.
    if dacl.is_null() {
        return Ok(false);
    }

    let mut control = 0u16;
    let mut revision = 0u32;
    // SAFETY: the descriptor is alive for the whole function.
    if unsafe { GetSecurityDescriptorControl(descriptor.0, &raw mut control, &raw mut revision) }
        == 0
    {
        return Err(io::Error::last_os_error());
    }
    // Without this the parent's inheritable ACEs are merged in and widen the object.
    if control & SE_DACL_PROTECTED == 0 {
        return Ok(false);
    }

    // Exactly one ACE, belonging to the token user. This is also what rules out `Everyone`,
    // `Users` and `Authenticated Users`: an ACE for any of them would already fail the count,
    // so resolving those well-known SIDs separately would add unsafe code and no assurance.
    // SAFETY: `dacl` points at a valid ACL for as long as the descriptor lives.
    if unsafe { (*dacl).AceCount } != 1 {
        return Ok(false);
    }

    let token_user = TokenUserInfo::current()?;
    let mut ace: *mut c_void = ptr::null_mut();
    // SAFETY: index 0 exists because AceCount is 1.
    if unsafe { GetAce(dacl, 0, &raw mut ace) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: an access-allowed ACE stores its SID inline starting at `SidStart`.
    let ace_sid = unsafe { (&raw mut (*ace.cast::<ACCESS_ALLOWED_ACE>()).SidStart).cast() };
    // SAFETY: both pointers refer to valid SIDs.
    Ok(unsafe { EqualSid(ace_sid, token_user.sid()) } != 0)
}

/// Releases a security descriptor the API allocated with `LocalAlloc`.
struct LocalDescriptor(PSECURITY_DESCRIPTOR);

impl Drop for LocalDescriptor {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::LocalFree;

        // SAFETY: allocated by GetNamedSecurityInfoW and freed exactly once.
        unsafe { LocalFree(self.0.cast()) };
    }
}
