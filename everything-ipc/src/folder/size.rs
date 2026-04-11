/*!
Folder size batch lookup and cache.

- Batch lookup of all folder sizes in the parent folder.
- Thread-local cache.
- For drivers (like `C:\`), it uses Windows API directly.

## References
- [`IbDOpusExt/ViewerPlugin/DOpusExt.cpp`](https://github.com/Chaoses-Ib/IbDOpusExt/blob/421397f1f73d49b1351ec6cebdf35a74dddb9019/ViewerPlugin/DOpusExt.cpp#L40-L113)
*/

use std::{cell::UnsafeCell, io, path::Path, time::Duration};

use bon::builder;
use rapidhash::{HashMapExt, RapidHashMap as HashMap};
use thiserror::Error;
use tracing::{debug, info, warn};
use widestring::U16CString;
use windows::{Win32::Storage::FileSystem::GetDiskFreeSpaceExW, core::PCWSTR};

use crate::{
    search,
    wm::{self, EverythingClient, RequestFlags, SearchFlags},
};

#[derive(Error, Debug)]
pub enum Error {
    #[error("path is relative")]
    RelativePath,

    #[error("folder not found")]
    NotFound,

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Ipc(#[from] wm::IpcError),
}

thread_local! {
    static EVERYTHING: UnsafeCell<Option<EverythingClient>> = const { UnsafeCell::new(None) };
    static LAST_PARENT: UnsafeCell<std::path::PathBuf> = const { UnsafeCell::new(std::path::PathBuf::new()) };
    static RESULT_MAP: UnsafeCell<Option<HashMap<String, u64>>> = const { UnsafeCell::new(None) };
}

/// Get the size of a folder.
///
/// See [`folder::size`](super::size) for details.
///
/// ## Arguments
/// - `path`: An absolute path to a folder
/// - `timeout`: Optional timeout for IPC queries.
///   If `None`, uses the default timeout.
/// - `parent_max_size`: Optional mutable reference to receive the maximum size of folders in the parent folder.
///
/// ## Returns
/// - `Ok(u64)`: The size in bytes
/// - `Err(Error)`: If the path is invalid
#[builder]
pub fn get_folder_size(
    #[builder(start_fn)] path: &Path,
    timeout: Option<Duration>,
    parent_max_size: Option<&mut u64>,
) -> Result<u64, Error> {
    debug_assert_eq!(search::normalize_path_ev(path), path);

    // Get the parent directory
    let parent = match path.parent() {
        Some(p) if p.as_os_str().is_empty() => return Err(Error::RelativePath),
        Some(p) => p,
        None => {
            // Handle 3-character paths (e.g., "C:\")
            if path.as_os_str().len() == 3 {
                let path_u16 = U16CString::from_os_str(path).unwrap();
                let mut size = 0u64;
                if unsafe {
                    GetDiskFreeSpaceExW(PCWSTR(path_u16.as_ptr()), None, Some(&mut size), None)
                }
                .is_ok()
                {
                    return Ok(size);
                }
            }
            return Err(Error::RelativePath);
        }
    };

    // Get or create the Everything client for this thread
    let everything = EVERYTHING.with(|cell| -> Result<&EverythingClient, wm::IpcError> {
        let opt = unsafe { &mut *cell.get() };
        if opt.is_none() {
            *opt = Some(EverythingClient::new()?);
        }
        Ok(&*opt.as_ref().unwrap())
    })?;

    // Check if we need to query for a new parent
    let needs_query = LAST_PARENT.with(|cell| unsafe {
        let last_path = &mut *cell.get();
        last_path != parent
    });

    if needs_query {
        // Clear and rebuild cache for new parent
        LAST_PARENT.with(|cell| unsafe {
            *cell.get() = parent.to_path_buf();
        });

        RESULT_MAP.with(|cell| unsafe {
            *cell.get() = None;
        });

        // Query Everything for files in the folder
        let search_query = format!(r#"folder:infolder:"{}""#, parent.display());
        let query_list = everything
            .query_wait(&search_query)
            .search_flags(SearchFlags::empty())
            .request_flags(RequestFlags::FileName | RequestFlags::Size)
            .maybe_timeout(timeout)
            .call()
            .inspect_err(|e| warn!(%e, ?parent, "query failed"))?;
        info!(len = query_list.len(), "query");

        // Build result map from query results
        let mut result_map = HashMap::with_capacity(query_list.len());
        for item in query_list.iter() {
            if let (Some(filename), Some(file_size)) = (
                item.get_str(RequestFlags::FileName),
                item.get_size(RequestFlags::Size),
            ) {
                let filename_str = filename.to_string_lossy();
                result_map.insert(filename_str, file_size);
            }
        }
        RESULT_MAP.with(|cell| unsafe {
            *cell.get() = Some(result_map);
        });
    }

    // Look up the file in the result map
    // Or PathFindFileNameW()
    let filename = path
        .file_name()
        .and_then(|f| f.to_str())
        .ok_or(Error::RelativePath)?
        .to_string();

    match RESULT_MAP.with(|cell| {
        let map = unsafe { &*cell.get() }.as_ref();

        if let Some(max_size) = parent_max_size {
            *max_size = map
                .and_then(|m| m.values().max().copied())
                .unwrap_or_default();
        }

        map.and_then(|m| m.get(&filename)).copied()
    }) {
        // If size is 0, try with realpath
        Some(0) => {
            let realpath = search::canonicalize_path_ev(path)?;
            if realpath != path {
                debug!(?realpath);
                // TODO: pipe?
                let size = everything
                    .get_folder_size(&realpath)
                    .maybe_timeout(timeout)
                    .call()?;
                return Ok(size);
            }
        }
        Some(size) => return Ok(size),
        None => {
            RESULT_MAP.with(|cell| {
                let map = unsafe { &*cell.get() };
                debug!(filename, ?map);
            });
        }
    }

    // TODO: May be new folder
    Err(Error::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_log::test]
    #[test_log(default_log_filter = "trace")]
    fn get_folder_size_root() {
        let r = get_folder_size(Path::new(r"C:\")).call();
        dbg!(&r);
        // Just verify it returns something without panicking
        assert!(r.unwrap() > 0);
    }

    #[test_log::test]
    #[test_log(default_log_filter = "trace")]
    fn get_folder_size_ev() {
        let r = get_folder_size(Path::new(r"C:\Windows")).call();
        dbg!(&r);
        assert!(r.unwrap() > 0);

        let r = get_folder_size(Path::new(r"C:\Users")).call();
        dbg!(&r);
        assert!(r.unwrap() > 0);
    }

    #[test_log::test]
    #[test_log(default_log_filter = "trace")]
    fn get_folder_size_ev_max() {
        let mut max_size: u64 = 0;
        let r = get_folder_size(Path::new(r"C:\Windows"))
            .parent_max_size(&mut max_size)
            .call();
        dbg!(&r, max_size);
        assert!(r.unwrap() > 0);

        let mut max_size2: u64 = 0;
        let r = get_folder_size(Path::new(r"C:\Users"))
            .parent_max_size(&mut max_size2)
            .call();
        dbg!(&r, max_size2);
        assert!(r.unwrap() > 0);

        assert_eq!(max_size, max_size2);
    }

    #[test_log::test]
    #[test_log(default_log_filter = "trace")]
    fn get_folder_size_ev_realpath() {
        // Test realpath resolution: "C:\Documents and Settings" -> "C:\Users"
        let r = get_folder_size(Path::new(r"C:\Documents and Settings"))
            .call()
            .unwrap();
        dbg!(&r);
        assert!(r > 0);

        let r2 = get_folder_size(Path::new(r"C:\Users")).call().unwrap();
        dbg!(&r2);
        assert!(r2 > 0);
        assert_eq!(r, r2);
    }
}
