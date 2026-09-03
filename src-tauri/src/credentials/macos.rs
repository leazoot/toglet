//! macOS credential storage: a generic password item in the login Keychain.
//!
//! **Not verified on a real machine.** Every claim below is from the Security framework's
//! documented contract, not from a run. The module compiles for `aarch64-apple-darwin`;
//! nothing more than that has been demonstrated.
//!
//! Raw framework calls rather than a wrapper crate: this needs precise control over the access
//! scope and over error mapping, and `OSStatus` gives exactly that. It also keeps the
//! dependency surface of the credential path as small as the Windows side's.
//!
//! Items are stored with `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`, which keeps them off
//! iCloud Keychain and out of device migrations. Credentials for one machine stay on it.

use std::ffi::c_void;

use super::secret::{CredentialRef, Secret};
use super::store::{SecretStore, unavailable};
use crate::diagnostics::Result;

type CFTypeRef = *const c_void;
type CFStringRef = *const c_void;
type CFDictionaryRef = *const c_void;
type CFDataRef = *const c_void;
type CFAllocatorRef = *const c_void;
type OSStatus = i32;

const ERR_SEC_SUCCESS: OSStatus = 0;
const ERR_SEC_ITEM_NOT_FOUND: OSStatus = -25300;
const ERR_SEC_DUPLICATE_ITEM: OSStatus = -25299;
const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;

/// Every Toglet item shares this service name, so the account attribute is the credential
/// reference and nothing else has to be encoded into it.
const SERVICE: &str = "com.toglet.desktop";

// Each block names the framework that defines its symbols. Without this the link only
// succeeds by accident, when some other crate in the graph happens to pull the same framework
// in; the application binary linked against neither and failed on every `kSec*` constant.
#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: CFStringRef;
    static kSecClassGenericPassword: CFStringRef;
    static kSecAttrService: CFStringRef;
    static kSecAttrAccount: CFStringRef;
    static kSecAttrAccessible: CFStringRef;
    static kSecAttrAccessibleWhenUnlockedThisDeviceOnly: CFStringRef;
    static kSecValueData: CFStringRef;
    static kSecReturnData: CFStringRef;
    static kSecMatchLimit: CFStringRef;
    static kSecMatchLimitOne: CFStringRef;

    fn SecItemAdd(attributes: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
    fn SecItemCopyMatching(query: CFDictionaryRef, result: *mut CFTypeRef) -> OSStatus;
    fn SecItemDelete(query: CFDictionaryRef) -> OSStatus;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: CFTypeRef;

    fn CFRelease(cf: CFTypeRef);
    fn CFStringCreateWithBytes(
        alloc: CFAllocatorRef,
        bytes: *const u8,
        num_bytes: isize,
        encoding: u32,
        is_external_representation: u8,
    ) -> CFStringRef;
    fn CFDataCreate(alloc: CFAllocatorRef, bytes: *const u8, length: isize) -> CFDataRef;
    fn CFDataGetLength(data: CFDataRef) -> isize;
    fn CFDataGetBytePtr(data: CFDataRef) -> *const u8;
    fn CFDictionaryCreate(
        alloc: CFAllocatorRef,
        keys: *const CFTypeRef,
        values: *const CFTypeRef,
        num_values: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> CFDictionaryRef;
}

/// A Core Foundation object this code owns and must release.
struct Owned(CFTypeRef);

impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: only created from a Create/Copy call, so exactly one release is owed.
            unsafe { CFRelease(self.0) };
        }
    }
}

pub struct MacosSecretStore;

impl MacosSecretStore {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MacosSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SecretStore for MacosSecretStore {
    fn store(&self, reference: &CredentialRef, secret: &Secret) -> Result<()> {
        // `SecItemUpdate` would need a second attribute dictionary and its own error mapping.
        // Delete-then-add reaches the same end state with one code path, and a delete of
        // something absent already succeeds.
        self.delete(reference)?;

        let account = cf_string(reference.as_str())?;
        let service = cf_string(SERVICE)?;
        let data = cf_data(secret.expose())?;
        let query = cf_dictionary(&[
            // SAFETY: framework constants, valid for the process lifetime.
            (unsafe { kSecClass }, unsafe { kSecClassGenericPassword }),
            (unsafe { kSecAttrService }, service.0),
            (unsafe { kSecAttrAccount }, account.0),
            (unsafe { kSecValueData }, data.0),
            (unsafe { kSecAttrAccessible }, unsafe {
                kSecAttrAccessibleWhenUnlockedThisDeviceOnly
            }),
        ])?;

        // SAFETY: `query` is a valid dictionary; no result is requested.
        let status = unsafe { SecItemAdd(query.0, std::ptr::null_mut()) };
        check(status)
    }

    fn load(&self, reference: &CredentialRef) -> Result<Secret> {
        let account = cf_string(reference.as_str())?;
        let service = cf_string(SERVICE)?;
        let query = cf_dictionary(&[
            // SAFETY: framework constants.
            (unsafe { kSecClass }, unsafe { kSecClassGenericPassword }),
            (unsafe { kSecAttrService }, service.0),
            (unsafe { kSecAttrAccount }, account.0),
            (unsafe { kSecReturnData }, unsafe { kCFBooleanTrue }),
            (unsafe { kSecMatchLimit }, unsafe { kSecMatchLimitOne }),
        ])?;

        let mut result: CFTypeRef = std::ptr::null();
        // SAFETY: `query` is valid and `result` is a valid out-pointer.
        let status = unsafe { SecItemCopyMatching(query.0, &raw mut result) };
        check(status)?;

        let data = Owned(result);
        if data.0.is_null() {
            return Err(unavailable("the keychain returned no data for the entry"));
        }
        // SAFETY: `kSecReturnData` makes the result a CFData.
        let length = unsafe { CFDataGetLength(data.0) };
        let bytes = unsafe { CFDataGetBytePtr(data.0) };
        if bytes.is_null() || length < 0 {
            return Err(unavailable("the keychain returned an unreadable payload"));
        }
        // SAFETY: the pointer and length come from the same CFData, alive until `data` drops.
        let copied = unsafe { std::slice::from_raw_parts(bytes, length as usize) }.to_vec();
        Ok(Secret::new(copied))
    }

    fn delete(&self, reference: &CredentialRef) -> Result<()> {
        let account = cf_string(reference.as_str())?;
        let service = cf_string(SERVICE)?;
        let query = cf_dictionary(&[
            // SAFETY: framework constants.
            (unsafe { kSecClass }, unsafe { kSecClassGenericPassword }),
            (unsafe { kSecAttrService }, service.0),
            (unsafe { kSecAttrAccount }, account.0),
        ])?;

        // SAFETY: `query` is a valid dictionary.
        let status = unsafe { SecItemDelete(query.0) };
        // Removing something that is not there is the state the caller asked for.
        if status == ERR_SEC_ITEM_NOT_FOUND {
            return Ok(());
        }
        check(status)
    }

    fn contains(&self, reference: &CredentialRef) -> Result<bool> {
        let account = cf_string(reference.as_str())?;
        let service = cf_string(SERVICE)?;
        let query = cf_dictionary(&[
            // SAFETY: framework constants.
            (unsafe { kSecClass }, unsafe { kSecClassGenericPassword }),
            (unsafe { kSecAttrService }, service.0),
            (unsafe { kSecAttrAccount }, account.0),
            (unsafe { kSecMatchLimit }, unsafe { kSecMatchLimitOne }),
        ])?;

        // SAFETY: `query` is valid; no data is requested back, so nothing needs releasing.
        let status = unsafe { SecItemCopyMatching(query.0, std::ptr::null_mut()) };
        match status {
            ERR_SEC_SUCCESS => Ok(true),
            ERR_SEC_ITEM_NOT_FOUND => Ok(false),
            other => Err(status_error(other)),
        }
    }
}

/// Maps an `OSStatus` to the same stable code Windows uses for a store failure.
///
/// The numeric status is kept in the redacted detail so a support report can name the exact
/// condition (locked keychain, denied access, cancelled prompt) without a second error code
/// per platform.
fn check(status: OSStatus) -> Result<()> {
    if status == ERR_SEC_SUCCESS {
        Ok(())
    } else {
        Err(status_error(status))
    }
}

fn status_error(status: OSStatus) -> crate::diagnostics::TogletError {
    let cause = match status {
        ERR_SEC_ITEM_NOT_FOUND => "no such credential entry",
        ERR_SEC_DUPLICATE_ITEM => "the credential entry already exists",
        // -25308 errSecInteractionNotAllowed, -25293 errSecAuthFailed, -128 errSecUserCanceled.
        -25308 => "the keychain is locked",
        -25293 => "the keychain denied access",
        -128 => "the keychain prompt was cancelled",
        _ => "the keychain refused the operation",
    };
    unavailable(&format!("{cause} (OSStatus {status})"))
}

fn cf_string(value: &str) -> Result<Owned> {
    let length =
        isize::try_from(value.len()).map_err(|_| unavailable("value too long for the keychain"))?;
    // SAFETY: the slice is valid for `length` bytes; the default allocator is used.
    let created = unsafe {
        CFStringCreateWithBytes(
            std::ptr::null(),
            value.as_ptr(),
            length,
            K_CF_STRING_ENCODING_UTF8,
            0,
        )
    };
    non_null(created)
}

fn cf_data(value: &[u8]) -> Result<Owned> {
    let length =
        isize::try_from(value.len()).map_err(|_| unavailable("value too long for the keychain"))?;
    // SAFETY: the slice is valid for `length` bytes and CFData copies it.
    let created = unsafe { CFDataCreate(std::ptr::null(), value.as_ptr(), length) };
    non_null(created)
}

/// Builds a query dictionary.
///
/// The callback pointers are null on purpose: the dictionary then neither retains nor releases
/// its entries, and compares keys by pointer. Both are correct here - every key is one of the
/// framework's own constant `CFStringRef`s, so pointer comparison is what matches, and every
/// value is owned by the caller for the whole duration of the framework call.
fn cf_dictionary(entries: &[(CFTypeRef, CFTypeRef)]) -> Result<Owned> {
    let keys: Vec<CFTypeRef> = entries.iter().map(|(key, _)| *key).collect();
    let values: Vec<CFTypeRef> = entries.iter().map(|(_, value)| *value).collect();
    let count =
        isize::try_from(entries.len()).map_err(|_| unavailable("query is unexpectedly large"))?;

    // SAFETY: `keys` and `values` are both `count` long and live across the call.
    let created = unsafe {
        CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            count,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    non_null(created)
}

fn non_null(created: CFTypeRef) -> Result<Owned> {
    if created.is_null() {
        Err(unavailable("core foundation refused to allocate"))
    } else {
        Ok(Owned(created))
    }
}
