/*!
Everything's window message IPC interface, supported by Everything v1.4+.

The main API is [`EverythingClient`].

- Support Everything v1.4 and v1.5 (including Alpha version).
- Higher performance than Everything v1.4's official SDK:
  - Hot query time is about 30% shorter.
  - Sending blocking time is 60% shorter for async queries.
- Support both sync and async (Tokio) querying.

## Examples
```no_run
// cargo add everything-ipc
use everything_ipc::wm::{EverythingClient, RequestFlags, Sort};

let everything = EverythingClient::new().expect("not available");

let list = everything
    .query_wait(r"C:\Windows\ *.exe")
    .request_flags(RequestFlags::FileName | RequestFlags::Size | RequestFlags::Path)
    .sort(Sort::SizeDescending)
    .max_results(5)
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

## References
- [everything-cpp (IbEverythingLib)](https://github.com/Chaoses-Ib/IbEverythingLib/tree/master/everything-cpp)
*/
use std::{
    mem,
    sync::{atomic, mpsc},
    time::Duration,
};

use bon::bon;
use tracing::{debug, error, instrument, trace, warn};
use windows::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    System::DataExchange::COPYDATASTRUCT,
    UI::{
        Controls::WC_STATIC,
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DispatchMessageW, GWL_USERDATA, GWLP_WNDPROC,
            GetMessageW, GetWindowLongPtrW, MSG, PostMessageW, ReplyMessage, SendMessageW,
            SetWindowLongPtrW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_COPYDATA, WM_QUIT,
        },
    },
};

use crate::IpcWindow;

mod types;
pub use types::*;
mod ext;

/// Errors that can occur when querying Everything
#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    /// No IPC window available
    #[error("IPC window not found")]
    NoIpcWindow,

    /// Failed to create reply window
    #[error("failed to create reply window")]
    CreateReplyWindow,

    /// Failed to send query to Everything
    #[error("failed to send query to Everything")]
    Send,

    /// Query timed out waiting for response
    #[error("query timed out")]
    Timeout,

    #[error("query: {0}")]
    Query(&'static str),
}

// ==================== Reply Window ====================

/*
/// Window class name for Everything IPC reply windows
const WINDOW_CLASS_NAME: &widestring::U16CStr = u16cstr!("everything_ipc::wm");

/// Global flag to track if the window class has been registered
static CLASS_REGISTERED: Once = Once::new();

/// Register the window class globally (once per process)
/// The class must be registered in the same thread that creates windows
fn register_window_class() {
    CLASS_REGISTERED.call_once(|| unsafe {
        let wnd_class = WNDCLASSW {
            lpfnWndProc: Some(reply_window_wndproc),
            hInstance: get_current_module_handle().into(),
            lpszClassName: PCWSTR(WINDOW_CLASS_NAME.as_ptr()),
            style: Default::default(),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hIcon: Default::default(),
            hCursor: Default::default(),
            hbrBackground: Default::default(),
            lpszMenuName: Default::default(),
        };

        let class_atom = RegisterClassW(&wnd_class);
        if class_atom == 0 {
            error!("Failed to register window class");
        } else {
            debug!(
                "Registered window class {}",
                WINDOW_CLASS_NAME.to_string_lossy()
            );
        }
    });
}
*/

/// A hidden reply window that receives query responses from Everything
#[derive(Debug)]
struct ReplyWindow {
    hwnd: HWND,
    // The thread handle is Send because we only use it to join the thread
    // This is safe because we only join the thread in Drop
    _thread: mem::MaybeUninit<std::thread::JoinHandle<()>>,
}

// SAFETY: The JoinHandle is only used to join the thread in Drop.
// The thread is created in ReplyWindow::new and only runs the message loop,
// which doesn't access any thread-local state. The HWND is stored as a raw
// pointer internally and is only accessed on the message loop thread.
// ReplyWindow is Send because we move the window creation into the thread.
unsafe impl Send for ReplyWindow {}
// ReplyWindow is Sync because the message loop is the only thread that
// accesses the HWND, and all access is through the single message loop thread.
unsafe impl Sync for ReplyWindow {}

/// Result from the message loop thread: the created window handle as usize
#[derive(Debug)]
struct MessageLoopResult {
    hwnd_usize: usize,
}

impl ReplyWindow {
    /// Create a new reply window - creates the window in the message loop thread
    pub fn new(inner: Box<ClientInner>) -> Result<Self, IpcError> {
        /*
        // Register the window class (once per process)
        // This must be called before creating any windows
        register_window_class();
        */

        // Create a channel to receive the window handle from the message loop thread
        let (tx, rx) = mpsc::channel::<MessageLoopResult>();

        // Store the inner pointer for use in the message loop thread
        // let inner_ptr_usize = inner_ptr as usize;

        // Start the message loop in a separate thread
        // The window class must be registered in the same thread where windows are created
        let thread = std::thread::spawn(move || {
            // Create the window in THIS thread (the message loop thread)
            let hwnd = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    // PCWSTR(WINDOW_CLASS_NAME.as_ptr()),
                    WC_STATIC,
                    None,
                    WINDOW_STYLE(0),
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    // Some(HINSTANCE::default()),
                    None,
                    None,
                )
            };

            let hwnd = match hwnd.ok() {
                Some(h) => h,
                None => {
                    debug!("Failed to create window in message loop thread");
                    let _ = tx.send(MessageLoopResult { hwnd_usize: 0 });
                    return;
                }
            };

            // Send the window handle back to the caller as usize
            if let Err(_) = tx.send(MessageLoopResult {
                hwnd_usize: hwnd.0 as usize,
            }) {
                // _ = unsafe { DestroyWindow(hwnd) };
                return;
            }

            debug!(?hwnd, "Created reply window");

            // Set GWL_USERDATA to the EverythingInner pointer
            let inner_ptr = Box::into_raw(inner);
            unsafe { SetWindowLongPtrW(hwnd, GWL_USERDATA, inner_ptr as isize) };

            unsafe {
                SetWindowLongPtrW(
                    hwnd,
                    GWLP_WNDPROC,
                    reply_window_wndproc as *const () as isize,
                )
            };

            // Run the message loop
            run_message_loop(hwnd);
        });

        // Wait for the window handle from the message loop thread
        let result = rx.recv().map_err(|_| IpcError::CreateReplyWindow)?;
        let MessageLoopResult { hwnd_usize } = result;

        let hwnd = HWND(hwnd_usize as *mut _);
        if hwnd.is_invalid() {
            return Err(IpcError::CreateReplyWindow);
        }

        Ok(Self {
            hwnd,
            _thread: mem::MaybeUninit::new(thread),
        })
    }

    /// Get the window handle
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// Send a message to this window
    pub fn post_message(
        &self,
        msg: u32,
        w_param: WPARAM,
        l_param: LPARAM,
    ) -> Result<(), windows::core::Error> {
        unsafe { PostMessageW(Some(self.hwnd), msg, w_param, l_param) }
    }

    /// Post `WM_QUIT` to the reply window to signal the message loop to exit
    ///
    /// `WM_CLOSE` works too, but not `DestroyWindow()`.
    pub fn quit(&self) {
        let _ = self.post_message(WM_QUIT, WPARAM(0), LPARAM(0));
    }
}

impl Drop for ReplyWindow {
    fn drop(&mut self) {
        // This must be done before waiting for the thread to ensure the thread
        // can process the quit message
        self.quit();

        let _thread = unsafe { self._thread.assume_init_read() };
        // Join the message loop thread if it exists
        // if let Some(handle) = self.thread.take() {
        //     let _ = handle.join();
        // }
        #[cfg(feature = "drop-join-thread")]
        let _ = _thread.join();
    }
}

/// A query response received from Everything
#[derive(Debug)]
pub struct QueryResponse {
    pub id: u32,
    pub data: Vec<u8>,
}

/// Reply window procedure for handling WM_APP and WM_COPYDATA
#[instrument(skip_all, fields(hwnd))]
unsafe extern "system" fn reply_window_wndproc(
    hwnd: HWND,
    msg: u32,
    w_param: WPARAM,
    l_param: LPARAM,
) -> LRESULT {
    // dbg!(hwnd, msg, w_param, l_param);
    match msg {
        // Forward the request data to the IPC window
        WM_APP => {
            // WPARAM contains pointer to Box<Vec<u8>> (from PostMessageW)
            // We take ownership of the Box, create a COPYDATASTRUCT, and forward it
            let request_ptr = w_param.0 as *mut Vec<u8>;
            let request = unsafe { Box::from_raw(request_ptr) };

            // Get the EverythingInner pointer from GWL_USERDATA
            let inner_ptr = unsafe { GetWindowLongPtrW(hwnd, GWL_USERDATA) };
            if inner_ptr != 0 {
                let inner = unsafe { &*(inner_ptr as *const ClientInner) };
                let ipc_hwnd = inner.ipc_window.hwnd();

                // Create COPYDATASTRUCT from the request data
                let cds = COPYDATASTRUCT {
                    dwData: EVERYTHING_IPC_COPYDATA_QUERY2W as usize,
                    cbData: request.len() as u32,
                    lpData: request.as_ptr() as *mut _,
                };

                // Get the raw pointer to the COPYDATASTRUCT
                let cds_ptr = &cds as *const COPYDATASTRUCT;

                // Send to IPC window synchronously
                let r = unsafe {
                    SendMessageW(
                        ipc_hwnd,
                        WM_COPYDATA,
                        Some(WPARAM(hwnd.0 as usize)),
                        Some(LPARAM(cds_ptr as isize)),
                    )
                };
                if r.0 == 1 {
                    trace!(?ipc_hwnd, ?r);
                } else {
                    warn!(?ipc_hwnd, ?r);
                    // Drop the current query sender since the message failed to send
                    // This will cause the query to Err on the client side
                    drop(inner.take_current_query_sender());
                }
            }

            // Request Vec is dropped here after SendMessageW returns

            LRESULT(0)
        }
        // Response from Everything IPC window
        WM_COPYDATA => {
            let copydata = unsafe { &*(l_param.0 as *const COPYDATASTRUCT) };
            // Do not assert that copydata->dwData == _EVERYTHING_COPYDATA_QUERYREPLY(0)
            // The code in Everything's SDK is wrong. copydata->dwData is replyid and can be any value.
            let id = copydata.dwData as u32;

            // Get the EverythingInner pointer from GWL_USERDATA
            let inner_ptr = unsafe { GetWindowLongPtrW(hwnd, GWL_USERDATA) } as *const ClientInner;
            if inner_ptr.is_null() {
                error!("No object found");
                return LRESULT(0);
            }

            // Get the sender from the inner struct
            let inner = unsafe { &*inner_ptr };
            if let Some(sender) = inner.take_current_query_sender() {
                if match &sender {
                    QuerySender::Sync(_sender) => {
                        // TODO: https://github.com/rust-lang/rust/issues/153668
                        /*
                        if sender.is_disconnected() {
                            return LRESULT(1);
                        }
                        */
                        false
                    }
                    #[cfg(feature = "tokio")]
                    QuerySender::Tokio(sender) => sender.is_closed(),
                } {
                    return LRESULT(1);
                }

                // Convert to QueryList and send
                // TODO: Callback for one less copy
                let data = unsafe {
                    std::slice::from_raw_parts(
                        copydata.lpData as *const u8,
                        copydata.cbData as usize,
                    )
                }
                .into();
                // Reply to Everything
                _ = unsafe { ReplyMessage(LRESULT(1)) };
                trace!(id, cbData = copydata.cbData, "WM_COPYDATA received");

                let results = QueryList::new(id, data);
                if match sender {
                    QuerySender::Sync(sender) => sender.send(results).is_ok(),
                    #[cfg(feature = "tokio")]
                    QuerySender::Tokio(sender) => sender.send(results).is_ok(),
                } {
                    debug!(id, "Sent query response");
                } else {
                    warn!(id, "Failed to send query response");
                }
            } else {
                warn!(id, "No pending query");
            }

            LRESULT(1)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, w_param, l_param) },
    }
}

/// Message loop runner - runs in a separate thread
/// Takes an HWND which is passed as usize for Send compatibility
fn run_message_loop(hwnd: HWND) {
    unsafe {
        let mut msg: MSG = mem::zeroed();
        let mut ret;
        loop {
            ret = GetMessageW(&mut msg, Some(hwnd), 0, 0);
            if ret.0 <= 0 {
                break;
            }
            DispatchMessageW(&mut msg);
        }
    }

    // Cleanup
    let inner_ptr = unsafe { GetWindowLongPtrW(hwnd, GWL_USERDATA) };
    if inner_ptr != 0 {
        drop(unsafe { Box::from_raw(inner_ptr as *mut ClientInner) });
    }
}

/// Wrapper for query senders (both sync and async)
enum QuerySender {
    Sync(mpsc::Sender<QueryList>),
    #[cfg(feature = "tokio")]
    Tokio(tokio::sync::oneshot::Sender<QueryList>),
}

/// Inner state shared by Everything
struct ClientInner {
    ipc_window: IpcWindow,
    /// The sender for the current (last) query
    /// Using Mutex for thread-safe mutable access
    current_query_sender: std::sync::Mutex<Option<QuerySender>>,
}

impl ClientInner {
    /// Safe guard to mitigate possible `PoisonError` panics.
    pub fn take_current_query_sender(&self) -> Option<QuerySender> {
        match self.current_query_sender.lock() {
            Ok(mut sender) => sender.take(),
            #[cfg(debug_assertions)]
            Err(e) => Err(e).unwrap(),
            #[cfg(not(debug_assertions))]
            Err(e) => {
                error!("poison");
                // self.current_query_sender.clear_poison();
                // e.into_inner()
                None
            }
        }
    }
}

/**
Everything IPC client

See [`wm`](super::wm) for details.
*/
pub struct EverythingClient {
    /// Owned by [`ReplyWindow`] to avoid possible UAF of [`ClientInner`] after drop.
    inner: &'static ClientInner,
    reply_window: ReplyWindow,
}

impl IpcWindow {
    pub fn wm_client(&self) -> Result<EverythingClient, IpcError> {
        // Create the inner state
        let inner = Box::new(ClientInner {
            ipc_window: self.clone(),
            current_query_sender: std::sync::Mutex::new(None),
        });
        let inner_ref: &ClientInner = inner.as_ref();
        let inner_ref: &'static ClientInner = unsafe { mem::transmute(inner_ref) };

        // Create the reply window with a pointer to the inner state
        // let inner_ptr = Arc::as_ptr(&inner) as *mut ClientInner;
        let reply_window = ReplyWindow::new(inner)?;

        let inner = inner_ref;
        Ok(EverythingClient {
            inner,
            reply_window,
        })
    }
}

impl std::ops::Deref for EverythingClient {
    type Target = IpcWindow;

    fn deref(&self) -> &Self::Target {
        self.ipc_window()
    }
}

impl EverythingClient {
    /// Create a new Everything client
    pub fn new() -> Result<Self, IpcError> {
        IpcWindow::new().ok_or(IpcError::NoIpcWindow)?.wm_client()
    }

    /// Create a new Everything client with instance name
    pub fn with_instance(instance_name: Option<&str>) -> Result<Self, IpcError> {
        IpcWindow::with_instance(instance_name)
            .ok_or(IpcError::NoIpcWindow)?
            .wm_client()
    }

    /// Get the IPC window for sending messages
    fn ipc_window(&self) -> &IpcWindow {
        &self.inner.ipc_window
    }

    /// Get the next query ID
    fn next_id(&self) -> u32 {
        static NEXT_ID: atomic::AtomicU32 = atomic::AtomicU32::new(0);
        NEXT_ID.fetch_add(1, atomic::Ordering::SeqCst)
    }

    /// Send a query to Everything
    fn query_send(
        &self,
        search: &str,
        search_flags: SearchFlags,
        request_flags: RequestFlags,
        sort: Sort,
        id: u32,
        offset: u32,
        max_results: Option<u32>,
    ) -> bool {
        let msg_hwnd = self.reply_window.hwnd();

        // Build the query request using EverythingIpcQuery2 struct
        let request = EverythingIpcQuery2::create(
            msg_hwnd.0 as u32,
            id,
            search_flags.bits(),
            offset,
            max_results.unwrap_or(u32::MAX),
            request_flags.bits(),
            sort as u32,
            search,
        );

        // Box the request Vec to keep it alive until the message is processed
        let request_box = Box::new(request);
        let request_ptr = Box::into_raw(request_box);

        // available: SendMessageW (blocked), SendMessageTimeoutW (unstable)
        // unavailable: PostMessageW, SendNotifyMessageW
        // not tested: SendMessageCallbackW

        // Use ReplyWindow::post_message to send WM_APP
        // The reply window's wndproc will forward this to the IPC window
        // WPARAM contains pointer to Box<Vec<u8>>, which owns the request data
        match self
            .reply_window
            .post_message(WM_APP, WPARAM(request_ptr as usize), LPARAM(0))
        {
            Ok(_) => true,
            Err(_) => {
                // PostMessageW failed, free the request to avoid leak
                let _ = unsafe { Box::from_raw(request_ptr) };
                false
            }
        }
    }
}

#[bon]
impl EverythingClient {
    /// Send a query to Everything
    ///
    /// # Important Note
    /// Everything only handles one query per window at a time. Sending another query
    /// when a query has not completed will cancel the old query.
    ///
    /// This method serializes queries by replacing the previous sender.
    /// Callers should wait for the receiver before calling again.
    ///
    /// # Example
    /// ```no_run
    /// use std::time::Duration;
    /// use everything_ipc::wm::{EverythingClient, RequestFlags};
    ///
    /// let everything = EverythingClient::new().unwrap();
    ///
    /// // These queries will be serialized (not sent concurrently)
    /// let receiver1 = everything
    ///     .query("search1")
    ///     .request_flags(RequestFlags::FileName)
    ///     .call()
    ///     .unwrap();
    /// let result1 = receiver1.recv_timeout(Duration::from_secs(5)).unwrap();
    ///
    /// // Now safe to send next query
    /// let receiver2 = everything
    ///     .query("search2")
    ///     .request_flags(RequestFlags::FileName)
    ///     .call()
    ///     .unwrap();
    /// let result2 = receiver2.recv_timeout(Duration::from_secs(5)).unwrap();
    /// ```
    #[instrument(skip_all)]
    #[builder]
    pub fn query(
        &self,
        #[builder(start_fn)] search: &str,
        #[builder(default)] search_flags: SearchFlags,
        request_flags: RequestFlags,
        #[builder(default)] sort: Sort,
        #[builder(default)] offset: u32,
        max_results: Option<u32>,
    ) -> Result<mpsc::Receiver<QueryList>, IpcError> {
        let id = self.next_id();
        debug!("generating query ID {}", id);

        // Create a channel for the response
        let (sender, receiver) = mpsc::channel::<QueryList>();

        // Send the query first
        let sent = self.query_send(
            search,
            search_flags,
            request_flags,
            sort,
            id,
            offset,
            max_results,
        );

        if !sent {
            warn!("failed to send query ID {}", id);
            return Err(IpcError::Send);
        }
        debug!("query ID {} sent successfully", id);

        // Store the sender (only one query at a time per Everything instance)
        // Using Mutex for thread-safe mutable access
        let old_sender = self
            .inner
            .current_query_sender
            .lock()
            .unwrap()
            .replace(QuerySender::Sync(sender));
        // Drop any previous sender that wasn't used - its receiver will fail
        drop(old_sender);

        Ok(receiver)
    }

    /// Send a query to Everything and wait for the result
    ///
    /// This method serializes queries to work around Everything's single-query-per-window limitation.
    /// Only one query can be sent at a time per reply window.
    #[instrument(skip_all)]
    #[builder]
    pub fn query_wait(
        &self,
        #[builder(start_fn)] search: &str,
        #[builder(default)] search_flags: SearchFlags,
        request_flags: RequestFlags,
        #[builder(default)] sort: Sort,
        #[builder(default)] offset: u32,
        max_results: Option<u32>,
        #[builder(default = Duration::from_millis(3000))] timeout: Duration,
    ) -> Result<QueryList, IpcError> {
        // Reuse query to send the query, then wait for the result
        let receiver = self
            .query(search)
            .search_flags(search_flags)
            .request_flags(request_flags)
            .sort(sort)
            .offset(offset)
            .maybe_max_results(max_results)
            .call()?;

        // Wait for the response with timeout
        match receiver.recv_timeout(timeout) {
            Ok(results) => Ok(results),
            Err(_) => {
                warn!("query timed out");
                Err(IpcError::Timeout)
            }
        }
    }
}

#[cfg(feature = "tokio")]
#[bon]
impl EverythingClient {
    /// Send a query to Everything asynchronously
    ///
    /// # Important Note
    /// Everything only handles one query per window at a time. Sending another query
    /// when a query has not completed will cancel the old query.
    ///
    /// This method serializes queries by replacing the previous sender.
    /// Callers should await the receiver before calling again.
    ///
    /// # Example
    /// ```no_run
    /// use everything_ipc::wm::{EverythingClient, RequestFlags};
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let everything = EverythingClient::new().unwrap();
    ///
    /// // These queries will be serialized (not sent concurrently)
    /// let receiver1 = everything
    ///     .query_tokio("search1")
    ///     .request_flags(RequestFlags::FileName)
    ///     .call()?;
    /// let result1 = receiver1.await?;
    ///
    /// // Now safe to send next query
    /// let receiver2 = everything
    ///     .query_tokio("search2")
    ///     .request_flags(RequestFlags::FileName)
    ///     .call()?;
    /// let result2 = receiver2.await?;
    /// # Ok(())
    /// # }
    /// ```
    #[instrument(skip_all)]
    #[builder]
    pub fn query_tokio(
        &self,
        #[builder(start_fn)] search: &str,
        #[builder(default)] search_flags: SearchFlags,
        request_flags: RequestFlags,
        #[builder(default)] sort: Sort,
        #[builder(default)] offset: u32,
        max_results: Option<u32>,
    ) -> Result<tokio::sync::oneshot::Receiver<QueryList>, IpcError> {
        let id = self.next_id();
        debug!("generating query ID {}", id);

        // Create a channel for the response
        let (sender, receiver) = tokio::sync::oneshot::channel::<QueryList>();

        // Send the query first
        let sent = self.query_send(
            search,
            search_flags,
            request_flags,
            sort,
            id,
            offset,
            max_results,
        );

        if !sent {
            warn!("failed to send query ID {}", id);
            return Err(IpcError::Send);
        }
        debug!("query ID {} sent successfully", id);

        // Store the sender (only one query at a time per Everything instance)
        // Using Mutex for thread-safe mutable access
        let old_sender = self
            .inner
            .current_query_sender
            .lock()
            .unwrap()
            .replace(QuerySender::Tokio(sender));
        // Drop any previous sender that wasn't used - its receiver will fail
        drop(old_sender);

        Ok(receiver)
    }

    /// Send a query to Everything asynchronously and wait for the result
    ///
    /// This method serializes queries to work around Everything's single-query-per-window limitation.
    /// Only one query can be sent at a time per reply window.
    #[instrument(skip_all)]
    #[builder]
    pub async fn query_wait_tokio(
        &self,
        #[builder(start_fn)] search: &str,
        #[builder(default)] search_flags: SearchFlags,
        request_flags: RequestFlags,
        #[builder(default)] sort: Sort,
        #[builder(default)] offset: u32,
        max_results: Option<u32>,
        #[builder(default = Duration::from_millis(3000))] timeout: Duration,
    ) -> Result<QueryList, IpcError> {
        // Reuse query_async to send the query, then wait for the result
        let receiver = self
            .query_tokio(search)
            .search_flags(search_flags)
            .request_flags(request_flags)
            .sort(sort)
            .offset(offset)
            .maybe_max_results(max_results)
            .call()?;

        // Wait for the response with timeout
        match tokio::time::timeout(timeout, receiver).await {
            Ok(Ok(results)) => Ok(results),
            Ok(Err(_)) => {
                warn!("query receiver error");
                Err(IpcError::Send)
            }
            Err(_) => {
                warn!("query timed out");
                Err(IpcError::Timeout)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_log::test]
    #[test_log(default_log_filter = "trace")]
    fn doc() {
        let everything = EverythingClient::new().expect("not available");

        let list = everything
            .query_wait(r"C:\Windows\ *.exe")
            .request_flags(RequestFlags::FileName | RequestFlags::Size | RequestFlags::Path)
            .sort(Sort::SizeDescending)
            .max_results(5)
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
    }

    #[test_log::test]
    #[test_log(default_log_filter = "trace")]
    fn query_empty_search() {
        let everything = EverythingClient::new().unwrap();
        let search = "";
        let search_flags = SearchFlags::MatchCase;
        let request_flags = RequestFlags::FileName | RequestFlags::Path;
        let sort = Sort::NameAscending;

        // Send query for first 5 items
        let result =
            everything.query_send(search, search_flags, request_flags, sort, 1000, 0, Some(5));

        assert!(result, "Query should be sent successfully");
    }

    #[test_log::test]
    #[test_log(default_log_filter = "trace")]
    fn query_with_pattern() {
        let everything = EverythingClient::new().unwrap();
        let search = "test";
        let search_flags = SearchFlags::MatchCase;
        let request_flags = RequestFlags::FileName;
        let sort = Sort::NameAscending;

        let result =
            everything.query_send(search, search_flags, request_flags, sort, 1001, 0, Some(10));

        assert!(result, "Query should be sent successfully");
    }

    #[test]
    fn query_with_full_path() {
        let everything = EverythingClient::new().unwrap();
        let search = "";
        let search_flags = SearchFlags::MatchCase;
        let request_flags =
            RequestFlags::FullPathAndFileName | RequestFlags::Size | RequestFlags::DateModified;
        let sort = Sort::NameAscending;

        let result =
            everything.query_send(search, search_flags, request_flags, sort, 1002, 0, Some(3));

        assert!(result, "Query should be sent successfully");
    }

    #[test]
    fn query_sort_by_size() {
        let everything = EverythingClient::new().unwrap();
        let search = "";
        let search_flags = SearchFlags::MatchCase;
        let request_flags = RequestFlags::FileName | RequestFlags::Size;
        let sort = Sort::SizeAscending;

        let result =
            everything.query_send(search, search_flags, request_flags, sort, 1003, 0, Some(5));

        assert!(result, "Query should be sent successfully");
    }

    #[test]
    fn query_with_offset() {
        let everything = EverythingClient::new().unwrap();
        let search = "";
        let search_flags = SearchFlags::MatchCase;
        let request_flags = RequestFlags::FileName;
        let sort = Sort::NameAscending;

        // First query without offset
        let result1 =
            everything.query_send(search, search_flags, request_flags, sort, 1005, 0, Some(2));

        assert!(result1, "First query should be sent successfully");

        // Second query with offset
        let result2 =
            everything.query_send(search, search_flags, request_flags, sort, 1006, 2, Some(2));

        assert!(
            result2,
            "Second query with offset should be sent successfully"
        );
    }

    #[test]
    fn query_everything() {
        let everything = EverythingClient::new().unwrap();
        let search = "test";
        let search_flags = SearchFlags::MatchCase;
        let request_flags = RequestFlags::FileName;
        let sort = Sort::NameAscending;

        let result = everything.query_send(
            search,
            search_flags,
            request_flags,
            sort,
            everything.next_id(),
            0,
            Some(5),
        );

        assert!(result, "Query should be sent successfully");
    }

    #[test]
    fn query_multiple_requests() {
        let everything = EverythingClient::new().unwrap();
        let search = "";
        let search_flags = SearchFlags::MatchCase;
        let request_flags = RequestFlags::FileName
            | RequestFlags::Path
            | RequestFlags::Size
            | RequestFlags::DateModified
            | RequestFlags::DateCreated;
        let sort = Sort::NameAscending;

        let result =
            everything.query_send(search, search_flags, request_flags, sort, 1004, 0, Some(5));

        assert!(result, "Query should be sent successfully");
    }

    #[test]
    fn query_wait_empty() {
        let everything = EverythingClient::new().unwrap();
        let search = "";
        let search_flags = SearchFlags::MatchCase;
        let request_flags = RequestFlags::FileName;
        let sort = Sort::NameAscending;

        // Check if IPC is available
        assert!(everything.is_ipc_available(), "IPC should be available");

        let result = everything
            .query_wait(search)
            .search_flags(search_flags)
            .request_flags(request_flags)
            .sort(sort)
            .offset(0)
            .max_results(10)
            .call();
        assert!(
            result.is_ok(),
            "query_wait should return Ok when Everything is available"
        );
    }

    #[test_log::test]
    #[test_log(default_log_filter = "trace")]
    fn query_wait() {
        let everything = EverythingClient::new().unwrap();
        // Use empty string to get all files, which is more reliable than searching for "test"
        // which may not exist on the system
        let search = "test";
        // MATCH_ACCENTS is marked as abandoned in the C++ code, use MATCH_CASE instead
        let search_flags = SearchFlags::MatchCase;
        let request_flags = RequestFlags::FileName;
        let sort = Sort::NameAscending;

        // Check if IPC is available
        assert!(everything.is_ipc_available(), "IPC should be available");

        let result = everything
            .query_wait(search)
            .search_flags(search_flags)
            .request_flags(request_flags)
            .sort(sort)
            .offset(0)
            .max_results(10)
            .call();
        dbg!(&result);
        assert!(
            result.is_ok(),
            "query_wait should return Ok when Everything is available"
        );
        assert!(
            result.as_ref().is_ok_and(|r| r.total_len() > 0),
            "Expected found_num > 0, got: {:?}",
            result
        );
    }

    #[test_log::test]
    #[test_log(default_log_filter = "trace")]
    fn query_wait_cancel() {
        let everything = EverythingClient::new().unwrap();

        // Check if IPC is available
        assert!(everything.is_ipc_available(), "IPC should be available");

        // Send multiple queries at once
        // Note: These will be serialized, so only the last one succeeds
        // The first two queries will be disconnected when the next query replaces their sender
        let searches = ["", "test", "rust"];
        let mut receivers = Vec::new();

        for search in &searches {
            let search_flags = SearchFlags::MatchCase;
            let request_flags = RequestFlags::FileName;
            let sort = Sort::NameAscending;
            let receiver = everything
                .query(search)
                .search_flags(search_flags)
                .request_flags(request_flags)
                .sort(sort)
                .offset(0)
                .max_results(10)
                .call()
                .expect("query should succeed");
            receivers.push(receiver);
        }

        // First query should fail (response rejected because sender was replaced)
        // When query 2 is sent, it replaces the sender for query 0
        // When the response comes back for query 0, the old sender is gone
        let result = receivers[0].recv_timeout(std::time::Duration::from_millis(3000));
        assert!(
            result.is_err(),
            "Query 0 should fail because sender was replaced (got: {:?})",
            result
        );

        // Second query should succeed (it's the current sender when response arrives)
        let result = receivers[1].recv_timeout(std::time::Duration::from_millis(3000));
        assert!(
            result.is_err(),
            "Query 1 should fail because sender was replaced (got: {:?})",
            result
        );

        // Last query should succeed
        let result = receivers[2].recv_timeout(std::time::Duration::from_millis(3000));
        let result = result.expect("Last query should succeed");
        dbg!(&result);
        assert!(
            result.total_len() > 0,
            "Last query should return valid results"
        );
    }

    #[test_log::test]
    #[test_log(default_log_filter = "trace")]
    fn query_wait_parallel() {
        // Check if IPC is available
        let everything1 = EverythingClient::new().unwrap();
        let everything2 = EverythingClient::new().unwrap();
        let everything3 = EverythingClient::new().unwrap();

        assert!(everything1.is_ipc_available(), "IPC should be available");

        // Send multiple queries at once using separate Everything instances
        // Note: These won't cancel each other
        let receiver1 = everything1
            .query("")
            .search_flags(SearchFlags::MatchCase)
            .request_flags(RequestFlags::FileName)
            .sort(Sort::NameAscending)
            .offset(0)
            .max_results(10)
            .call()
            .expect("query should succeed");
        let receiver2 = everything2
            .query("test")
            .search_flags(SearchFlags::MatchCase)
            .request_flags(RequestFlags::FileName)
            .sort(Sort::NameAscending)
            .offset(0)
            .max_results(10)
            .call()
            .expect("query should succeed");
        let receiver3 = everything3
            .query("rust")
            .search_flags(SearchFlags::MatchCase)
            .request_flags(RequestFlags::FileName)
            .sort(Sort::NameAscending)
            .offset(0)
            .max_results(10)
            .call()
            .expect("query should succeed");

        // Wait for all queries to complete
        for (i, receiver) in [receiver1, receiver2, receiver3].into_iter().enumerate() {
            let result = receiver.recv_timeout(std::time::Duration::from_millis(5000));
            let result = result.expect(&format!("Query {} timed out", i));
            dbg!(&result);
            assert!(result.len() > 0, "Query {} should return valid results", i);
        }
    }
}
