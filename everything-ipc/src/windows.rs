use windows::{Win32::Foundation::HANDLE, core::Free};

/*
/// Get the handle of the current executable or DLL.
///
/// Ref: https://github.com/compio-rs/winio/issues/35
pub fn get_current_module_handle() -> HMODULE {
    let mut module = HMODULE::default();
    _ = unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            PCWSTR(get_current_module_handle as *const _),
            &mut module,
        )
    };
    module
}
*/

/// A Windows handle wrapper
///
/// # Safety
///
/// This type automatically closes the handle when dropped.
#[derive(Debug)]
pub struct Handle {
    inner: HANDLE,
}

unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

impl Handle {
    pub fn new(handle: HANDLE) -> Self {
        Handle { inner: handle }
    }

    pub fn get(&self) -> HANDLE {
        self.inner
    }

    pub fn is_null(&self) -> bool {
        self.inner.0.is_null()
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { self.inner.free() };
    }
}
