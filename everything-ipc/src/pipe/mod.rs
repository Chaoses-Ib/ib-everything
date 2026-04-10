/*!
Everything's named pipe IPC interface, supported by Everything v1.5+.

[Everything 1.5 SDK - voidtools forum](https://www.voidtools.com/forum/viewtopic.php?t=15853)

TODO: Batch IPC
*/

use bon::bon;
use std::{io, mem, sync::Mutex, time::Duration};
use thiserror::Error;
use windows::{
    Win32::{
        Foundation::{
            ERROR_IO_PENDING, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE, GetLastError, HANDLE,
        },
        Storage::FileSystem::{
            CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, ReadFile, WriteFile,
        },
        System::{
            IO::GetOverlappedResult,
            Threading::{CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects},
        },
    },
    core::PCWSTR,
};

use crate::windows::Handle;

/// Error type for Everything IPC operations
#[derive(Error, Debug)]
pub enum IpcError {
    #[error("out of memory")]
    OutOfMemory,
    #[error("IPC pipe not found")]
    PipeNotFound,
    #[error("pipe is busy")]
    PipeBusy,
    #[error("connection disconnected")]
    Disconnected,
    #[error("invalid parameter")]
    InvalidParameter,
    #[error("bad request")]
    BadRequest,
    #[error("cancelled")]
    Cancelled,
    #[error("property not found")]
    PropertyNotFound,
    #[error("server error")]
    Server,
    #[error("invalid command")]
    InvalidCommand,
    #[error("bad response")]
    BadResponse,
    #[error("insufficient buffer")]
    InsufficientBuffer,
    #[error("shutdown")]
    Shutdown,
    #[error("io error: {0}")]
    Io(#[from] io::Error),
}

/// IPC pipe command codes
const COMMAND_GET_FOLDER_SIZE: u32 = 18;

/// IPC pipe response codes
const RESPONSE_OK: u32 = 200;
const RESPONSE_OK_MORE_DATA: u32 = 100;
const RESPONSE_ERROR_BAD_REQUEST: u32 = 400;
const RESPONSE_ERROR_CANCELLED: u32 = 401;
const RESPONSE_ERROR_NOT_FOUND: u32 = 404;
const RESPONSE_ERROR_OUT_OF_MEMORY: u32 = 500;
const RESPONSE_ERROR_INVALID_COMMAND: u32 = 501;

/// Chunk size for overlapped I/O operations
const CHUNK_SIZE: usize = 65536;

/// Message header for IPC communication
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct Message {
    pub code: u32,
    pub size: u32,
}

/// Client structure for IPC communication
///
/// There can only be one client, you should drop this as early as possible.
#[derive(Debug)]
pub struct EverythingClient {
    mutex: Mutex<()>,
    pipe_handle: Option<Handle>,
    send_event: Handle,
    recv_event: Handle,
    shutdown_event: Handle,
}

#[bon]
impl EverythingClient {
    /// Connect to the Everything IPC pipe, trying common instance names.
    ///
    /// This will first try to connect without an instance name (default),
    /// and if that fails, try "1.5a".
    #[builder]
    pub async fn new(
        instance_name: Option<&str>,
        /// Automatic retry until timeout if the pipe is busy.
        #[builder(default = Duration::from_secs(10))]
        timeout: Duration,
    ) -> Result<Self, IpcError> {
        match Self::with_instance_timeout(instance_name, timeout).await {
            Ok(client) => Ok(client),
            Err(IpcError::PipeNotFound) if instance_name.is_none() => {
                Self::with_instance_timeout(Some("1.5a"), timeout).await
            }
            Err(e) => Err(e),
        }
    }

    /// Connect to the Everything IPC pipe with automatic retry until timeout.
    ///
    /// This wraps [new_auto] and retries on [PipeBusy] until the timeout.
    ///
    /// # Arguments
    /// * `timeout` - The maximum time to wait for the pipe to become available.
    async fn with_instance_timeout(
        instance_name: Option<&str>,
        timeout: Duration,
    ) -> Result<Self, IpcError> {
        let start = std::time::Instant::now();
        loop {
            match Self::try_new(instance_name) {
                Ok(client) => return Ok(client),
                Err(IpcError::PipeBusy) => {
                    if start.elapsed() >= timeout {
                        return Err(IpcError::PipeBusy);
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// Connect to the Everything IPC pipe
    /// `instance_name` can be None or empty to connect to the unnamed instance.
    pub fn try_new(instance_name: Option<&str>) -> Result<Self, IpcError> {
        unsafe {
            let mut pipe_name = Vec::<u16>::new();
            if !get_pipe_name(&mut pipe_name, instance_name.unwrap_or("")) {
                return Err(IpcError::OutOfMemory);
            }

            let pipe_name_ptr = pipe_name.as_ptr();
            let pipe_handle = match CreateFileW(
                PCWSTR(pipe_name_ptr),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                Default::default(),
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            ) {
                Ok(handle) => handle,
                Err(_) => {
                    let last_error = GetLastError();
                    if last_error.0 == ERROR_PIPE_BUSY.0 {
                        return Err(IpcError::PipeBusy);
                    }
                    return Err(IpcError::PipeNotFound);
                }
            };

            let mut client = Self {
                mutex: Mutex::new(()),
                pipe_handle: None,
                send_event: Handle::new(HANDLE::default()),
                recv_event: Handle::new(HANDLE::default()),
                shutdown_event: Handle::new(HANDLE::default()),
            };

            client.pipe_handle = Some(Handle::new(pipe_handle));

            client.shutdown_event = Handle::new(match CreateEventW(None, true, false, None) {
                Ok(handle) => handle,
                Err(_) => return Err(IpcError::OutOfMemory),
            });
            if client.shutdown_event.is_null() {
                return Err(IpcError::OutOfMemory);
            }

            client.send_event = Handle::new(match CreateEventW(None, true, false, None) {
                Ok(handle) => handle,
                Err(_) => return Err(IpcError::OutOfMemory),
            });
            if client.send_event.is_null() {
                return Err(IpcError::OutOfMemory);
            }

            client.recv_event = Handle::new(match CreateEventW(None, true, false, None) {
                Ok(handle) => handle,
                Err(_) => return Err(IpcError::OutOfMemory),
            });
            if client.recv_event.is_null() {
                return Err(IpcError::OutOfMemory);
            }

            Ok(client)
        }
    }

    /// Shutdown this client
    pub fn shutdown(&self) -> Result<(), IpcError> {
        unsafe {
            if !self.shutdown_event.is_null() {
                let _ = SetEvent(self.shutdown_event.get());
            }
            Ok(())
        }
    }

    /// IPC control operation
    fn ioctrl(
        &self,
        code: u32,
        in_data: *const u8,
        in_size: usize,
        out_data: *mut u8,
        out_size: usize,
        out_num_read: *mut usize,
    ) -> Result<bool, IpcError> {
        let _guard = self.mutex.lock();

        self._send(code, in_data, in_size)?;

        let mut recv_header = Message { code: 0, size: 0 };

        let header_ok = self._recv_header(&mut recv_header)?;
        if !header_ok {
            return Ok(false);
        }

        let data_size = recv_header.size as usize;
        if data_size > out_size {
            if self._recv_skip(data_size).is_err() {
                return Err(IpcError::InsufficientBuffer);
            }
            return Ok(false);
        }

        if self._recv_data(out_data, data_size).is_ok() {
            if !out_num_read.is_null() {
                unsafe {
                    *out_num_read = data_size;
                }
            }
            return Ok(true);
        }

        Ok(false)
    }

    /// Send data through IPC pipe
    fn _send(&self, code: u32, in_data: *const u8, in_size: usize) -> Result<(), IpcError> {
        if in_size as u64 > u32::MAX as u64 {
            return Err(IpcError::OutOfMemory);
        }

        let send_message = Message {
            code,
            size: in_size as u32,
        };

        if !self.write_overlapped(
            &send_message as *const _ as *const u8,
            mem::size_of::<Message>(),
        )? {
            return Err(IpcError::Disconnected);
        }

        if !self.write_overlapped(in_data, in_size)? {
            return Err(IpcError::Disconnected);
        }

        Ok(())
    }

    /// Write data with overlapped I/O
    fn write_overlapped(&self, buf: *const u8, size: usize) -> Result<bool, IpcError> {
        let mut overlapped: windows::Win32::System::IO::OVERLAPPED =
            unsafe { mem::MaybeUninit::zeroed().assume_init() };
        overlapped.hEvent = self.send_event.get();
        overlapped.Anonymous.Anonymous.Offset = 0;
        overlapped.Anonymous.Anonymous.OffsetHigh = 0;

        let mut buf_ptr = buf;
        let mut remaining = size;

        while remaining > 0 {
            let mut num_written: u32 = 0;
            unsafe {
                let chunk_size = std::cmp::min(remaining, CHUNK_SIZE) as u32;

                let result = WriteFile(
                    self.pipe_handle.as_ref().unwrap().get(),
                    Some(std::slice::from_raw_parts(buf_ptr, chunk_size as usize)),
                    Some(&mut num_written),
                    Some(&mut overlapped),
                );

                match result {
                    Ok(()) => {
                        if num_written > 0 {
                            buf_ptr = buf_ptr.add(num_written as usize);
                            remaining -= num_written as usize;
                            continue;
                        } else {
                            return Ok(false);
                        }
                    }
                    Err(_) => {
                        let last_error = GetLastError();

                        if last_error.0 == ERROR_IO_PENDING.0 {
                            let handles: [HANDLE; 2] =
                                [self.shutdown_event.get(), self.send_event.get()];
                            let wait_result = WaitForMultipleObjects(&handles, false, INFINITE);

                            if wait_result.0 == windows::Win32::Foundation::WAIT_OBJECT_0.0 {
                                return Err(IpcError::Shutdown);
                            }

                            match GetOverlappedResult(
                                self.pipe_handle.as_ref().unwrap().get(),
                                &overlapped,
                                &mut num_written,
                                false,
                            ) {
                                Ok(()) => {
                                    if num_written > 0 {
                                        buf_ptr = buf_ptr.add(num_written as usize);
                                        remaining -= num_written as usize;
                                        continue;
                                    } else {
                                        return Ok(false);
                                    }
                                }
                                Err(_) => {
                                    return Ok(false);
                                }
                            }
                        } else {
                            return Ok(false);
                        }
                    }
                }
            }
        }

        Ok(true)
    }

    /// Receive header from IPC pipe
    fn _recv_header(&self, recv_header: &mut Message) -> Result<bool, IpcError> {
        let data_size = mem::size_of::<Message>();
        let recv_header_ptr = recv_header as *mut _ as *mut u8;

        if self._recv_data(recv_header_ptr, data_size).is_err() {
            return Ok(false);
        }

        if recv_header.code == RESPONSE_OK || recv_header.code == RESPONSE_OK_MORE_DATA {
            return Ok(true);
        }

        // For error responses, skip the data payload and map to error type
        if self._recv_skip(recv_header.size as usize).is_ok() {
            match recv_header.code {
                RESPONSE_ERROR_BAD_REQUEST => return Err(IpcError::BadRequest),
                RESPONSE_ERROR_CANCELLED => return Err(IpcError::Cancelled),
                RESPONSE_ERROR_NOT_FOUND => return Err(IpcError::PipeNotFound),
                RESPONSE_ERROR_OUT_OF_MEMORY => return Err(IpcError::Server),
                RESPONSE_ERROR_INVALID_COMMAND => return Err(IpcError::InvalidCommand),
                _ => return Err(IpcError::BadResponse),
            }
        }

        Ok(false)
    }

    /// Receive data from IPC pipe
    fn _recv_data(&self, buf: *mut u8, buf_size: usize) -> Result<(), IpcError> {
        let mut overlapped: windows::Win32::System::IO::OVERLAPPED =
            unsafe { mem::MaybeUninit::zeroed().assume_init() };
        overlapped.hEvent = self.recv_event.get();
        overlapped.Anonymous.Anonymous.Offset = 0;
        overlapped.Anonymous.Anonymous.OffsetHigh = 0;

        let mut buf_ptr = buf;
        let mut remaining = buf_size;

        loop {
            let chunk_size = std::cmp::min(remaining, CHUNK_SIZE);

            unsafe {
                if remaining == 0 {
                    return Ok(());
                }

                let mut num_read: u32 = 0;
                let result = ReadFile(
                    self.pipe_handle.as_ref().unwrap().get(),
                    Some(std::slice::from_raw_parts_mut(buf_ptr, chunk_size)),
                    Some(&mut num_read),
                    Some(&mut overlapped),
                );

                match result {
                    Ok(()) => {
                        if num_read > 0 {
                            buf_ptr = buf_ptr.add(num_read as usize);
                            remaining -= num_read as usize;
                        } else {
                            return Err(IpcError::Disconnected);
                        }
                    }
                    Err(_) => {
                        let last_error = GetLastError();

                        if last_error.0 == ERROR_IO_PENDING.0 {
                            let handles: [HANDLE; 2] =
                                [self.shutdown_event.get(), self.recv_event.get()];
                            let wait_result = WaitForMultipleObjects(&handles, false, INFINITE);

                            if wait_result.0 == windows::Win32::Foundation::WAIT_OBJECT_0.0 {
                                return Err(IpcError::Shutdown);
                            }

                            match GetOverlappedResult(
                                self.pipe_handle.as_ref().unwrap().get(),
                                &overlapped,
                                &mut num_read,
                                false,
                            ) {
                                Ok(()) => {
                                    if num_read > 0 {
                                        buf_ptr = buf_ptr.add(num_read as usize);
                                        remaining -= num_read as usize;
                                    } else {
                                        return Err(IpcError::Disconnected);
                                    }
                                }
                                Err(_) => {
                                    return Err(IpcError::Disconnected);
                                }
                            }
                        } else {
                            return Err(IpcError::Disconnected);
                        }
                    }
                }
            }
        }
    }

    /// Skip data from IPC pipe
    fn _recv_skip(&self, size: usize) -> Result<(), IpcError> {
        let mut buf = [0u8; 256];
        let mut remaining = size;

        loop {
            if remaining == 0 {
                return Ok(());
            }

            let recv_size = if remaining > 256 { 256 } else { remaining };

            if self._recv_data(buf.as_mut_ptr(), recv_size).is_err() {
                break;
            }

            remaining -= recv_size;
        }

        Err(IpcError::Disconnected)
    }
}

impl EverythingClient {
    /// Retrieves the indexed folder size.
    ///
    /// ## Remarks
    /// Everything 1.5 will index folder sizes by default.
    /// This can be disabled under Tools -> Options -> Indexes -> Index folder sizes.
    ///
    /// A case sensensitive search is performed first.
    /// If no folder is found a case insensitive search is performed.
    #[doc(alias = "get_folder_size_from_filename")]
    pub fn get_folder_size(&self, path: &str) -> Result<u64, IpcError> {
        let mut value: u64 = u64::MAX;
        let mut num_read: usize = 0;

        let result = self.ioctrl(
            COMMAND_GET_FOLDER_SIZE,
            path.as_ptr(),
            path.len(),
            &mut value as *mut u64 as *mut u8,
            mem::size_of::<u64>(),
            &mut num_read,
        )?;

        if result && num_read == mem::size_of::<u64>() && value != u64::MAX {
            Ok(value)
        } else {
            Err(IpcError::BadResponse)
        }
    }
}

/// Get pipe name with instance suffix if provided
fn get_pipe_name(buf: &mut Vec<u16>, instance_name: &str) -> bool {
    let base = r"\\.\PIPE\Everything IPC";

    // Push base string as UTF-16
    for ch in base.encode_utf16() {
        buf.push(ch);
    }

    if !instance_name.is_empty() {
        buf.push('(' as u16);
        buf.push(' ' as u16);

        // Push instance name as UTF-16
        for ch in instance_name.encode_utf16() {
            buf.push(ch);
        }

        buf.push(')' as u16);
    }

    // Null terminator
    buf.push(0);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn new() {
        // Test that we can connect to the Everything IPC
        let client = EverythingClient::builder().build().await.unwrap();
        let _ = client;
    }

    #[tokio::test]
    async fn get_folder_size() {
        let client = EverythingClient::builder().build().await.expect("connect");

        let result = client.get_folder_size(r"C:\Windows");

        match result {
            Ok(size) => {
                dbg!(size);
                assert!(size > 0, "should have a non-zero size");

                assert!(
                    size > 1024 * 1024, // at least 1MB
                    "should be at least 1MB, got {} bytes",
                    size
                );
            }
            Err(e) => {
                // The command may not be supported by all versions of Everything
                panic!(
                    "Note: get_folder_size_from_filename_w returned error: {:?}",
                    e
                );
            }
        }
    }
}
