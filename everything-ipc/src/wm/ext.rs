use std::{path::Path, time::Duration};

use bon::bon;
use tracing::warn;

use crate::wm::{EverythingClient, IpcError, RequestFlags};

#[bon]
impl EverythingClient {
    /// A convenient query wrapper that get the size of a folder.
    ///
    /// See also [`folder::size`](crate::folder::size).
    #[builder]
    pub fn get_folder_size(
        &self,
        #[builder(start_fn)] path: &Path,
        timeout: Option<Duration>,
    ) -> Result<u64, IpcError> {
        let search_query = format!(r#"wfn:"{}""#, path.display());

        let query_list = self
            .query_wait(&search_query)
            .request_flags(RequestFlags::Size)
            .maybe_timeout(timeout)
            .call()
            .inspect_err(|e| warn!(%e, ?path, "query failed"))?;

        if let Some(item) = query_list.get(0) {
            if let Some(size) = item.get_size(RequestFlags::Size) {
                return Ok(size);
            }
        }
        Err(IpcError::Query("folder not found"))
    }
}
