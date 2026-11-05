//! Official authorization API, independently reported from launchd loading.
use super::*;
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    Enabled,
    RequiresApproval,
    NotRegistered,
    NotFound,
    Unknown,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OsStatus {
    pub permission: Permission,
    pub loaded: Option<bool>,
    pub permission_supported: bool,
    pub query_failed: bool,
}
fn permission(value: isize) -> Permission {
    match value {
        0 => Permission::NotRegistered,
        1 => Permission::Enabled,
        2 => Permission::RequiresApproval,
        3 => Permission::NotFound,
        _ => Permission::Unknown,
    }
}
#[cfg(target_os = "macos")]
fn load_framework() -> bool {
    static FRAMEWORK: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *FRAMEWORK.get_or_init(|| unsafe {
        libc::dlopen(
            c"/System/Library/Frameworks/ServiceManagement.framework/ServiceManagement".as_ptr(),
            libc::RTLD_LAZY | libc::RTLD_LOCAL,
        ) as usize
    }) != 0
}
#[cfg(target_os = "macos")]
pub fn open_settings() -> Result<()> {
    use std::ffi::c_void;
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const i8) -> *mut c_void;
        fn sel_registerName(name: *const i8) -> *mut c_void;
        fn class_getClassMethod(class: *mut c_void, sel: *mut c_void) -> *mut c_void;
        fn objc_msgSend();
    }
    if !load_framework() {
        return Err("login_service_settings_unavailable".into());
    }
    unsafe {
        let class = objc_getClass(c"SMAppService".as_ptr());
        let selector = sel_registerName(c"openSystemSettingsLoginItems".as_ptr());
        if class.is_null() || class_getClassMethod(class, selector).is_null() {
            return Err("login_service_settings_unavailable".into());
        }
        let send: unsafe extern "C" fn(*mut c_void, *mut c_void) =
            std::mem::transmute(objc_msgSend as *const ());
        send(class, selector);
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn legacy_permission(path: &Path) -> Option<Permission> {
    use std::{ffi::c_void, os::unix::ffi::OsStrExt};
    #[link(name = "objc")]
    extern "C" {
        fn objc_getClass(name: *const i8) -> *mut c_void;
        fn sel_registerName(name: *const i8) -> *mut c_void;
        fn class_getClassMethod(class: *mut c_void, sel: *mut c_void) -> *mut c_void;
        fn objc_msgSend();
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFURLCreateFromFileSystemRepresentation(
            allocator: *const c_void,
            bytes: *const u8,
            length: isize,
            is_directory: u8,
        ) -> *const c_void;
        fn CFRelease(value: *const c_void);
    }
    if !load_framework() {
        return None;
    }
    unsafe {
        let class = objc_getClass(c"SMAppService".as_ptr());
        let selector = sel_registerName(c"statusForLegacyURL:".as_ptr());
        if class.is_null() || class_getClassMethod(class, selector).is_null() {
            return None;
        }
        let bytes = path.as_os_str().as_bytes();
        let url = CFURLCreateFromFileSystemRepresentation(
            std::ptr::null(),
            bytes.as_ptr(),
            bytes.len() as isize,
            0,
        );
        if url.is_null() {
            return None;
        }
        let pool = objc_autoreleasePoolPush();
        let send: unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void) -> isize =
            std::mem::transmute(objc_msgSend as *const ());
        let status = send(class, selector, url);
        CFRelease(url);
        objc_autoreleasePoolPop(pool);
        Some(permission(status))
    }
}
#[cfg(target_os = "macos")]
pub(super) fn authorization(plan: &Plan) -> Option<Permission> {
    if matches!(
        plan.registration,
        Registration::Matches | Registration::Repairable
    ) {
        legacy_permission(&plan.registration_file)
    } else {
        None
    }
}
#[cfg(target_os = "macos")]
pub async fn inspect_os(plan: &Plan) -> OsStatus {
    let permission = authorization(plan);
    let (loaded, failed) = super::control::loaded(plan).await;
    OsStatus {
        permission: permission.unwrap_or(Permission::Unknown),
        permission_supported: permission.is_some(),
        loaded,
        query_failed: failed,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_authorization_without_conflating_registration_and_loading() {
        assert_eq!(permission(0), Permission::NotRegistered);
        assert_eq!(permission(1), Permission::Enabled);
        assert_eq!(permission(2), Permission::RequiresApproval);
        assert_eq!(permission(3), Permission::NotFound);
        assert_eq!(permission(99), Permission::Unknown);
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn official_api_handles_nonexistent_disposable_path_without_registration() {
        let path = account_home()
            .unwrap()
            .join("Library/LaunchAgents")
            .join(format!("clamav-status-only-{}.plist", uuid::Uuid::new_v4()));
        let status = legacy_permission(&path);
        // Valid legacy prefix, but no plist is created and no service registered.
        assert!(!path.exists());
        assert!(matches!(
            status,
            None | Some(Permission::NotFound | Permission::NotRegistered | Permission::Unknown)
        ));
    }
}
