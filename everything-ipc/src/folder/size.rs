/*!
Folder size batch lookup and cache.

- Batch lookup of all folder sizes in the parent folder.
- Thread-local cache with [`get_folder_size()`].
- For drivers (like `C:\`), it uses Windows API directly.

## References
- [`IbDOpusExt/ViewerPlugin/DOpusExt.cpp`](https://github.com/Chaoses-Ib/IbDOpusExt/blob/421397f1f73d49b1351ec6cebdf35a74dddb9019/ViewerPlugin/DOpusExt.cpp#L40-L113)
*/

use std::{
    cell::UnsafeCell,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use bon::{bon, builder};
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

// #[cfg(any(doc, not(feature = "drop-join-thread")))]
thread_local! {
    static CLIENT: UnsafeCell<FolderSizeClient> = const { UnsafeCell::new(FolderSizeClient::new()) };
}

/// Get the size of a folder.
///
/// Uses a thread-local [`FolderSizeClient`] for caching.
///
/// See [`folder::size`](super::size) for details.
///
/// ## Arguments
/// - `path`: An absolute path to a folder
/// - `timeout`: Optional timeout for IPC queries.
///   If `None`, uses the default timeout.
/// - `parent_max_size`: Optional mutable reference to receive the maximum size of folders in the parent folder.
///
///   You may also want to set `eager_get_links`.
///
/// - `eager_get_links`: If `true`, for folders in the parent folder with size 0,
///   eagerly resolve symlinks/junctions and query the resolved path's size.
///
/// ## Returns
/// - `Ok(u64)`: The size in bytes
/// - `Err(Error)`: If the path is invalid
///
/// ## Note
/// If `drop-join-thread` feature is enabled, you need to call
/// [`wm::EverythingClient::shared_quit_join_thread()`]
/// before process exit / DLL unload to avoid deadlock,
/// or use [`FolderSizeClient`] instead.
///
// #[cfg(any(doc, not(feature = "drop-join-thread")))]
#[builder]
pub fn get_folder_size(
    #[builder(start_fn)] path: &Path,
    timeout: Option<Duration>,
    parent_max_size: Option<&mut u64>,
    #[builder(default)] eager_get_links: bool,
) -> Result<u64, Error> {
    CLIENT.with(|cell| {
        let client = unsafe { &mut *cell.get() };
        client
            .get_folder_size(path)
            .maybe_timeout(timeout)
            .maybe_parent_max_size(parent_max_size)
            .eager_get_links(eager_get_links)
            .call()
    })
}

/// Folder size client with parent directory cache.
#[derive(Default)]
pub struct FolderSizeClient {
    everything: Option<Arc<EverythingClient>>,
    last_parent: PathBuf,
    /// `HashMap::new()` is not const.
    result_map: Option<HashMap<String, u64>>,
}

#[bon]
impl FolderSizeClient {
    pub const fn new() -> Self {
        Self {
            everything: None,
            last_parent: PathBuf::new(),
            result_map: None,
        }
    }

    /*
    /// Get or create the Everything client
    fn everything(&mut self) -> Result<&EverythingClient, wm::IpcError> {
        // TODO: get_or_try_insert_with()
        let everything = match self.everything.as_ref() {
            Some(everything) => everything,
            None => self.everything.insert(EverythingClient::shared()?),
        };
        Ok(everything)
    }
    */

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
    ///   You may also want to set `eager_get_links`.
    ///
    /// - `eager_get_links`: If `true`, for folders in the parent folder with size 0,
    ///   eagerly resolve symlinks/junctions and query the resolved path's size.
    ///
    /// ## Returns
    /// - `Ok(u64)`: The size in bytes
    /// - `Err(Error)`: If the path is invalid
    #[builder]
    pub fn get_folder_size(
        &mut self,
        #[builder(start_fn)] path: &Path,
        timeout: Option<Duration>,
        parent_max_size: Option<&mut u64>,
        #[builder(default)] eager_get_links: bool,
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

        // Get or create the Everything client
        // TODO: get_or_try_insert_with()
        let everything = match self.everything.as_ref() {
            Some(everything) => everything,
            None => self.everything.insert(EverythingClient::shared()?),
        };

        // Check if we need to query for a new parent
        let needs_query = self.last_parent != parent;

        if needs_query {
            // Clear and rebuild cache for new parent
            self.last_parent = parent.to_path_buf();
            self.result_map = None;

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
                if let (Some(filename), Some(mut file_size)) = (
                    item.get_str(RequestFlags::FileName),
                    item.get_size(RequestFlags::Size),
                ) {
                    let filename_str = filename.to_string_lossy();

                    // For folders with size 0, check if it's a symlink/junction when eager_get_links is true
                    if eager_get_links && file_size == 0 {
                        let path = parent.join(&filename_str);
                        match search::canonicalize_path_ev(&path) {
                            // Resolve path is different, query for the resolved path size
                            Ok(realpath) if realpath != path => {
                                debug!(dir = filename_str, ?realpath);
                                match everything
                                    .get_folder_size(&realpath)
                                    .maybe_timeout(timeout)
                                    .call()
                                {
                                    Ok(size) => {
                                        file_size = size;
                                    }
                                    e => warn!(?e, ?realpath, "query realpath failed"),
                                }
                            }
                            Ok(_) => (),
                            Err(e) => warn!(%e, ?path, "realpath failed"),
                        }
                    }

                    result_map.insert(filename_str, file_size);
                }
            }
            self.result_map = Some(result_map);
        }

        // Look up the file in the result map
        // Or PathFindFileNameW()
        let filename = path
            .file_name()
            .and_then(|f| f.to_str())
            .ok_or(Error::RelativePath)?
            .to_string();

        match self.result_map.as_ref().and_then(|m| {
            if let Some(max_size) = parent_max_size {
                *max_size = m.values().max().copied().unwrap_or_default();
            }

            m.get(&filename).copied()
        }) {
            // Empty folder
            Some(0) if eager_get_links => {
                return Ok(0);
            }
            // If size is 0, try with realpath
            Some(0) if !eager_get_links => {
                let realpath = search::canonicalize_path_ev(path)?;
                if realpath != path {
                    debug!(?realpath);
                    // TODO: pipe?
                    let size = everything
                        .get_folder_size(&realpath)
                        .maybe_timeout(timeout)
                        .call()?;

                    // Cache realpath size
                    // We got Some(0)
                    self.result_map.as_mut().unwrap().insert(filename, size);

                    return Ok(size);
                }
                // Empty folder
                return Ok(0);
            }
            Some(size) => return Ok(size),
            None => {
                debug!(filename, map = ?self.result_map);
            }
        }

        // TODO: May be new folder
        Err(Error::NotFound)
    }
}

#[cfg(not(feature = "drop-join-thread"))]
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
        let r1 = get_folder_size(Path::new(r"C:\Documents and Settings"))
            .call()
            .unwrap();
        dbg!(&r1);
        assert_eq!(r, r1);

        let r2 = get_folder_size(Path::new(r"C:\Users")).call().unwrap();
        dbg!(&r2);
        assert!(r2 > 0);
        assert_eq!(r, r2);
    }

    #[test_log::test]
    #[test_log(default_log_filter = "trace")]
    fn get_folder_size_ev_realpath_eager() {
        // Test realpath resolution: "C:\Documents and Settings" -> "C:\Users"
        let mut max_size: u64 = 0;
        let r = get_folder_size(Path::new(r"C:\Documents and Settings"))
            .parent_max_size(&mut max_size)
            .eager_get_links(true)
            .call()
            .unwrap();
        info!(r, max_size);
        assert!(r > 0);
        let r1 = get_folder_size(Path::new(r"C:\Documents and Settings"))
            .parent_max_size(&mut max_size)
            .eager_get_links(true)
            .call()
            .unwrap();
        info!(r1, max_size);
        assert_eq!(r, r1);

        let r2 = get_folder_size(Path::new(r"C:\Users"))
            .parent_max_size(&mut max_size)
            .eager_get_links(true)
            .call()
            .unwrap();
        info!(r2, max_size);
        assert!(r2 > 0);
        assert_eq!(r, r2);
    }
}
