use std::sync::{Arc, Mutex, Weak};

use crate::wm::{EverythingClient, IpcError};

static SHARED: Mutex<Weak<EverythingClient>> = Mutex::new(Weak::new());

impl EverythingClient {
    /// Get a globally-shared, lazily-created [`EverythingClient`].
    ///
    /// The first call creates and returns an [`Arc`]-wrapped client. Subsequent
    /// calls return a clone of the same [`Arc`]. When all returned [`Arc`]s are
    /// dropped, the underlying client (and its reply window thread) is dropped.
    /// A new client is automatically created on the next call.
    ///
    /// ## Note
    /// [`EverythingClient::query()`] will cancel the previous query.
    /// Use with caution.
    pub fn shared() -> Result<Arc<Self>, IpcError> {
        let mut guard = SHARED.lock().unwrap();
        if let Some(client) = guard.upgrade() {
            return Ok(client);
        }
        let client = Arc::new(Self::new()?);
        *guard = Arc::downgrade(&client);
        Ok(client)
    }

    /// Quit and join the message thread of the last [`shared`](Self::shared) client instance.
    ///
    /// This function is meant to manually make sure the thread is exited before DLL unloading.
    /// While `drop-join-thread` feature also works for `static`s,
    /// it will break `thread_local`s.
    ///
    /// ## Safety
    /// This function is `!Sync`. It's unlikely that you need `Sync` though.
    pub unsafe fn shared_quit_join_thread() {
        let mut guard = SHARED.lock().unwrap();
        if let Some(client) = guard.upgrade() {
            *guard = Default::default();
            drop(guard);

            // TODO: Arc::get_mut_unchecked()
            let client = unsafe { &mut *Arc::as_ptr(&client).cast_mut() };
            client.reply_window.quit_join_thread();
        }
    }
}
