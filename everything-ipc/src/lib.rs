/*!
Rust port of voidtools' Everything's IPC SDK.
Can be used to search user files quickly on Windows.

## Features
- Support both Everything v1.4 and v1.5, including Alpha version.
- Higher performance than Everything v1.4's official SDK:
  - Hot query time is about 30% shorter.
  - Sending blocking time is 60% shorter for async queries.
- Support both sync and async (Tokio) querying.
- Search text generating utilities.
- Folder-based batch IPC and cache.

## APIs
- [`IpcWindow`]: A minimal IPC interface for Everything v1.4+.
- [`wm`]: Everything's window message IPC interface, supported by Everything v1.4+.
- [`pipe`]: Everything's named pipe IPC interface, supported by Everything v1.5+.
- [`search`]: Search text generating utilities.
- [`folder`]: Folder-based batch IPC and cache.

## Examples
```no_run
// cargo add everything-ipc
use everything_ipc::wm::{EverythingClient, RequestFlags, Sort};

let everything = EverythingClient::new().expect("not available");

let list = everything
    .query_wait(r"C:\Windows\ *.exe")
    .request_flags(RequestFlags::FileName | RequestFlags::Size | RequestFlags::Path)
    .sort(Sort::SizeDescending)
    .max_results(10)
    .call()
    .expect("query");

println!("Found {} items:", list.len());
println!("{:<25} {:>10}  {}", "Filename", "Size", "Path");
for item in list.iter() {
    // get_string() for String, get_str() for &U16CStr
    let filename = item.get_string(RequestFlags::FileName).unwrap();
    let path = item.get_str(RequestFlags::Path).unwrap().display();
    let size = item.get_size(RequestFlags::Size).unwrap();
    println!("{:<25} {:>10}  {}", filename, size, path);
}
println!("Total: {} items", list.total_len());
/*
Found 5 items:
Filename                        Size  Path
MRT.exe                    223939376  C:\Windows\System32
MRT-KB890830.exe           133315992  C:\Windows\System32
OneDriveSetup.exe           89771848  C:\Windows\WinSxS\amd64_microsoft-windows-onedrive-setup_31bf3856ad364e35_10.0.26100.5074_none_c1340e9ad5f0a5d0
OneDriveSetup.exe           89771848  C:\Windows\System32
OneDriveSetup.exe           60357040  C:\Windows\WinSxS\amd64_microsoft-windows-onedrive-setup_31bf3856ad364e35_10.0.26100.1_none_2233e98c8e9ce5f5
Total: 5742 items
*/
```
*/
//!
//! ## Crate features
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "doc", doc = document_features::document_features!())]

use ::windows::{
    Win32::{
        Foundation::{FALSE, HWND, LPARAM, TRUE, WPARAM},
        System::Threading::GetCurrentThreadId,
        UI::WindowsAndMessaging::{
            EnumThreadWindows, FindWindowW, GetClassNameW, SendMessageW, WM_USER,
        },
    },
    core::{BOOL, PCWSTR},
};
use tracing::debug;
use widestring::{U16Str, u16str};

use crate::wm::{
    EVERYTHING_IPC_GET_MINOR_VERSION, EVERYTHING_IPC_IS_DB_LOADED,
    EVERYTHING_IPC_IS_FILE_INFO_INDEXED,
};

#[cfg(feature = "folder")]
pub mod folder;
#[cfg(feature = "tokio")]
pub mod pipe;
pub mod search;
mod windows;
pub mod wm;

const IPC_CLASS_PREFIX: &U16Str = u16str!("EVERYTHING_TASKBAR_NOTIFICATION");

const EVERYTHING_WM_IPC: u32 = WM_USER;

struct EnumWindowsData {
    result: Option<IpcWindow>,
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let data = unsafe { &mut *(lparam.0 as *mut EnumWindowsData) };

    let mut buf = [0; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len > 0 {
        let class_name = U16Str::from_slice(&buf[..len as usize]);
        // debug!(?hwnd, ?class_name, "enum_windows_proc");
        if class_name
            .as_slice()
            .starts_with(IPC_CLASS_PREFIX.as_slice())
        {
            data.result = Some(IpcWindow {
                hwnd,
                class_name: class_name.to_string().unwrap(),
            });
            return FALSE;
        }
    }

    TRUE
}

/**
An IPC window of Everything.

This struct is a minimal IPC interface for Everything v1.4+.
It only supports:
- Detecting Everything.
- Retrieving its version and instance name.
- Retrieving its DB status.
*/
#[derive(Debug, Clone)]
pub struct IpcWindow {
    hwnd: HWND,
    class_name: String,
}

impl IpcWindow {
    /// Find an [`IpcWindow`] by trying common instance names.
    pub fn new() -> Option<Self> {
        match Self::with_instance(None) {
            Some(window) => Some(window),
            None => Self::with_instance(Some("1.5a")),
        }
    }

    /// Find an [`IpcWindow`] globally using [`FindWindowW`].
    pub fn with_instance(instance_name: Option<&str>) -> Option<Self> {
        let mut class_name = Vec::<u16>::with_capacity(IPC_CLASS_PREFIX.len() + 7);
        class_name.extend_from_slice(IPC_CLASS_PREFIX.as_slice());

        if let Some(name) = instance_name {
            class_name.push('(' as u16);
            for ch in name.encode_utf16() {
                class_name.push(ch);
            }
            class_name.push(')' as u16);
        }
        // Null terminator
        class_name.push(0);

        let hwnd = unsafe { FindWindowW(PCWSTR(class_name.as_ptr()), None).ok() }?;
        if hwnd.is_invalid() {
            return None;
        }

        Some(IpcWindow {
            hwnd,
            class_name: String::from_utf16_lossy(&class_name[..class_name.len() - 1]),
        })
    }

    /// Find an [`IpcWindow`] from current thread, mainly for plugins.
    pub fn from_current_thread() -> Option<Self> {
        let mut data = EnumWindowsData { result: None };

        let tid = unsafe { GetCurrentThreadId() };
        debug!(?tid, "from_current_thread");
        _ = unsafe {
            EnumThreadWindows(
                tid,
                Some(enum_windows_proc),
                LPARAM(&mut data as *mut _ as isize),
            )
        };

        data.result
    }

    /// [`HWND`] of the IPC window.
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Class name of the IPC window.
    pub fn class_name(&self) -> &str {
        &self.class_name
    }

    pub fn instance_name(&self) -> Option<&str> {
        // e.g. "EVERYTHING_TASKBAR_NOTIFICATION_(1.5a)"
        self.class_name
            .strip_prefix("EVERYTHING_TASKBAR_NOTIFICATION_(")
            .and_then(|s| s.strip_suffix(')'))
    }

    /// Check if IPC is available
    pub fn is_ipc_available(&self) -> bool {
        unsafe {
            SendMessageW(
                self.hwnd,
                EVERYTHING_WM_IPC,
                Some(WPARAM(EVERYTHING_IPC_GET_MINOR_VERSION as usize)),
                None,
            )
        }
        .0 >= 4
    }

    /// Get the version of Everything
    pub fn get_version(&self) -> Version {
        const EVERYTHING_IPC_GET_MAJOR_VERSION: u32 = 0;
        const EVERYTHING_IPC_GET_MINOR_VERSION: u32 = 1;
        const EVERYTHING_IPC_GET_REVISION: u32 = 2;
        const EVERYTHING_IPC_GET_BUILD_NUMBER: u32 = 3;
        // const EVERYTHING_IPC_GET_TARGET_MACHINE: u32 = 5;

        let send_u32 = |command: u32| unsafe {
            SendMessageW(
                self.hwnd,
                EVERYTHING_WM_IPC,
                Some(WPARAM(command as usize)),
                None,
            )
            .0 as u32
        };

        Version {
            major: send_u32(EVERYTHING_IPC_GET_MAJOR_VERSION),
            minor: send_u32(EVERYTHING_IPC_GET_MINOR_VERSION),
            revision: send_u32(EVERYTHING_IPC_GET_REVISION),
            build: send_u32(EVERYTHING_IPC_GET_BUILD_NUMBER),
        }
    }

    /// Check if the database is loaded
    pub fn is_db_loaded(&self) -> bool {
        unsafe {
            SendMessageW(
                self.hwnd,
                EVERYTHING_WM_IPC,
                Some(WPARAM(EVERYTHING_IPC_IS_DB_LOADED as usize)),
                Some(LPARAM(0)),
            )
        }
        .0 != 0
    }

    /// Check if info is indexed
    pub fn is_file_info_indexed(&self, info: wm::FileInfo) -> bool {
        unsafe {
            SendMessageW(
                self.hwnd,
                EVERYTHING_WM_IPC,
                Some(WPARAM(EVERYTHING_IPC_IS_FILE_INFO_INDEXED as usize)),
                Some(LPARAM(info as isize)),
            )
        }
        .0 != 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub revision: u32,
    pub build: u32,
}

impl Version {
    pub fn new(major: u32, minor: u32, revision: u32, build: u32) -> Self {
        Self {
            major,
            minor,
            revision,
            build,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::wm::FileInfo;

    use super::*;

    #[test]
    fn ipc_window_find_global() {
        let window = IpcWindow::with_instance(None);
        assert!(window.is_some(), "Should find Everything IPC window");

        let window = window.unwrap();
        let class_name = window.class_name();
        println!("IPC Window class name: {}", class_name);
        // Either exact match or starts with prefix
        assert!(
            class_name == "EVERYTHING_TASKBAR_NOTIFICATION"
                || class_name.starts_with("EVERYTHING_TASKBAR_NOTIFICATION_(")
        );

        let version = window.get_version();
        println!(
            "Everything version: {}.{}.{}.{}",
            version.major, version.minor, version.revision, version.build
        );
    }

    #[test]
    fn get_version() {
        let everything = IpcWindow::new().unwrap();
        let version = everything.get_version();

        // Everything v1.5+ should have major >= 1
        assert!(version.major >= 1, "Expected Everything v1+");
        println!(
            "Version: {}.{}.{}.{}",
            version.major, version.minor, version.revision, version.build
        );
    }

    #[test]
    fn is_db_loaded() {
        let everything = IpcWindow::new().unwrap();
        let loaded = everything.is_db_loaded();
        assert!(loaded, "Everything DB should be loaded");
    }

    #[test]
    fn is_file_info_indexed() {
        let everything = IpcWindow::new().unwrap();

        // Test various info types
        let _ = everything.is_file_info_indexed(FileInfo::FileSize);
        let _ = everything.is_file_info_indexed(FileInfo::FolderSize);
        let _ = everything.is_file_info_indexed(FileInfo::DateCreated);
        let _ = everything.is_file_info_indexed(FileInfo::DateModified);
        let _ = everything.is_file_info_indexed(FileInfo::DateAccessed);
        let _ = everything.is_file_info_indexed(FileInfo::Attributes);
    }
}
