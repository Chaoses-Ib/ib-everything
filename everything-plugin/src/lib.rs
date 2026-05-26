//! Rust binding for [Everything](https://www.voidtools.com/)'s [plugin SDK](https://www.voidtools.com/forum/viewtopic.php?t=16535).
//!
//! Features:
//! - Load and save config with [Serde](https://github.com/serde-rs/serde)
//! - Make options pages GUI using [Winio](https://github.com/compio-rs/winio) in MVU (Elm) architecture
//! - Internationalization with [rust-i18n](https://github.com/longbridge/rust-i18n)
//! - Log with [tracing](https://github.com/tokio-rs/tracing)
//!
//! ## Example
//! ```rust
//! mod options;
//!
//! #[derive(Serialize, Deserialize, Debug, Default)]
//! pub struct Config {
//!     s: String,
//! }
//!
//! pub struct App {
//!     config: Config,
//! }
//!
//! impl PluginApp for App {
//!     type Config = Config;
//!
//!     fn new(config: Option<Self::Config>) -> Self {
//!         Self {
//!             config: config.unwrap_or_default(),
//!         }
//!     }
//!
//!     fn config(&self) -> &Self::Config {
//!         &self.config
//!     }
//!
//!     fn into_config(self) -> Self::Config {
//!         self.config
//!     }
//! }
//!
//! plugin_main!(App, {
//!     PluginHandler::builder()
//!         .name("Test Plugin")
//!         .description("A test plugin for Everything")
//!         .author("Chaoses-Ib")
//!         .version("0.1.0")
//!         .link("https://github.com/Chaoses-Ib/IbEverythingLib")
//!         .options_pages(vec![
//!             OptionsPage::builder()
//!                 .name("Test Plugin")
//!                 .load(ui::winio::spawn::<options::MainModel>)
//!                 .build(),
//!         ])
//!         .build()
//! });
//! ```
//!
//! ## Detachable design
//! The API is designed to allow the app to be easily detached from Everything and run independently. Either for standalone distribution or testing.
//!
//! This also means a standalone Winio app can be relatively easily integrated into Everything as a plugin.
//!
//! Components:
//! - tracing
//! - Winio
//! - Serde
//! - [`PluginApp`]
//! - [`PluginHandler`]
//!   - [`PluginHandler::init_start()`], [`PluginHandler::init_start_with_config()`]
//!   - [`PluginHandler::stop_kill()`]
//!   - [`PluginHandler::get_host()`]
//!
//! TODO:
//! - Tray icon and menu itmes / tabs
//! - Load & save config with file
//! - Unified host/IPC API
//!
//! ## Build
//! ### Static CRT
//! `.cargo/config.toml`:
//! ```toml
//! [target.'cfg(all(target_os = "windows", target_env = "msvc"))']
//! rustflags = ["-C", "target-feature=+crt-static"]
//! ```
//! Increase build size by ~100 KiB.
//!
//! ## Debugging
//! - `.\Everything64.exe -debug`
//!   
//!   Unlike `-debug`, `-debug-log` doesn't work with stdout/stderr outputs.
//!
//! ## Features
#![cfg_attr(docsrs, feature(doc_cfg))]
#![cfg_attr(feature = "doc", doc = document_features::document_features!())]

use core::str;
use std::{
    cell::{Cell, OnceCell, UnsafeCell},
    ffi::{CString, c_void},
    mem,
    ops::Deref,
    slice,
};

use bon::Builder;
use everything_ipc::IpcWindow;
use tracing::{debug, trace};

use crate::data::Config;

pub use everything_ipc as ipc;
pub use serde;

pub mod data;
#[cfg(feature = "tracing")]
pub mod log;
pub mod macros;
pub mod sys;
pub mod ui;

/// ## Example
/// ```ignore
/// #[derive(Serialize, Deserialize, Debug, Default)]
/// pub struct Config {
///     s: String,
/// }
///
/// pub struct App {
///     config: Config,
/// }
///
/// impl PluginApp for App {
///     type Config = Config;
///
///     fn new(config: Option<Self::Config>) -> Self {
///         Self {
///             config: config.unwrap_or_default(),
///         }
///     }
///
///     fn config(&self) -> &Self::Config {
///         &self.config
///     }
///
///     fn into_config(self) -> Self::Config {
///         self.config
///     }
/// }
/// ```
pub trait PluginApp: 'static {
    type Config: Config;

    fn new(config: Option<Self::Config>) -> Self;

    /// Can be used to start services requiring to access the [`PluginApp`] through [`PluginHandler::with_app()`] or [`PluginHandler::app()`].
    fn start(&self) {}

    fn config(&self) -> &Self::Config;

    fn into_config(self) -> Self::Config;
}

/// ## Example
/// ```ignore
/// PluginHandler::builder()
///     .name("Test Plugin")
///     .description("A test plugin for Everything")
///     .author("Chaoses-Ib")
///     .version("0.1.0")
///     .link("https://github.com/Chaoses-Ib/IbEverythingLib")
///     .options_pages(vec![
///         OptionsPage::builder()
///             .name("Test Plugin")
///             .load(ui::winio::spawn::<options::MainModel>)
///             .build(),
///     ])
///     .build()
/// ```
///
/// ## Design
/// - Config may be accessed from multiple threads, and options pages need to modify it. To avoid race conditions, either config is cloned when modifying, and then [`PluginApp`] is reloaded with it, i.e. [`arc_swap::ArcSwap`]; or [`PluginApp`] is shutdown before modifying and then restarted.
/// - User defined static to work around generic static limit.
///   - Interior mutability to make it easy to use with `static`. But `UnsafeCell` to avoid cost.
///
/// Config lifetime:
/// - May be set with [`PluginHandler::builder()`] (as default value)
/// - May be loaded when [`sys::EVERYTHING_PLUGIN_PM_START`]
/// - Be read when start
/// - Be read when loading (and rendering) options pages ([`sys::EVERYTHING_PLUGIN_PM_LOAD_OPTIONS_PAGE`])
/// - Be written/applied when [`sys::EVERYTHING_PLUGIN_PM_SAVE_OPTIONS_PAGE`], zero, one or multiple times
///   - TODO: Defer
/// - Be saved when [`sys::EVERYTHING_PLUGIN_PM_SAVE_SETTINGS`] (can occur without prior [`sys::EVERYTHING_PLUGIN_PM_SAVE_OPTIONS_PAGE`])
#[derive(Builder)]
pub struct PluginHandler<A: PluginApp> {
    #[builder(skip)]
    host: OnceCell<PluginHost>,

    #[builder(with = |x: impl Into<String>| CString::new(x.into()).unwrap())]
    name: Option<CString>,
    #[builder(with = |x: impl Into<String>| CString::new(x.into()).unwrap())]
    description: Option<CString>,
    #[builder(with = |x: impl Into<String>| CString::new(x.into()).unwrap())]
    author: Option<CString>,
    #[builder(with = |x: impl Into<String>| CString::new(x.into()).unwrap())]
    version: Option<CString>,
    #[builder(with = |x: impl Into<String>| CString::new(x.into()).unwrap())]
    link: Option<CString>,

    #[builder(skip)]
    app: UnsafeCell<Option<A>>,

    #[builder(default)]
    options_pages: Vec<ui::OptionsPage<A>>,
    #[builder(skip)]
    options_message: Cell<ui::OptionsMessage>,

    /// TODO: Feature cfg?
    #[builder(skip)]
    instance_name: UnsafeCell<Option<String>>,
}

unsafe impl<A: PluginApp> Send for PluginHandler<A> {}
unsafe impl<A: PluginApp> Sync for PluginHandler<A> {}

impl<A: PluginApp> PluginHandler<A> {
    /// Panics if already initialized.
    pub fn init_start(&self) {
        if self.host.get().is_some() {
            return;
        }
        self.handle(sys::EVERYTHING_PLUGIN_PM_INIT, 0 as _);
        self.handle(sys::EVERYTHING_PLUGIN_PM_START, 0 as _);
    }

    /// Panics if already initialized.
    ///
    /// ## Note
    /// Plugin may be already inited before hijacker is loaded on Everything v1.5.0.1409+.
    /// Double init and start may cause buggy behavior.
    ///
    /// Refs:
    /// - [在最新的everything上崩溃 - Issue #110 - IbEverythingExt](https://github.com/Chaoses-Ib/IbEverythingExt/issues/110)
    /// - [在最新的1.5.0.1411b上无效了 - Issue #114 - IbEverythingExt](https://github.com/Chaoses-Ib/IbEverythingExt/issues/114)
    /// - [关闭显示文件夹大小功能之后还是会显示大小 - Issue #115 - IbEverythingExt](https://github.com/Chaoses-Ib/IbEverythingExt/issues/115)
    /// - [发现config.yaml必须手动更新 - Issue #116 - IbEverythingExt](https://github.com/Chaoses-Ib/IbEverythingExt/issues/116)
    pub fn init_start_with_config(&self, config: A::Config) {
        if self.host.get().is_some() {
            return;
        }
        self.handle(sys::EVERYTHING_PLUGIN_PM_INIT, 0 as _);
        self.handle(
            sys::EVERYTHING_PLUGIN_PM_START,
            Box::into_raw(Box::new(config)) as _,
        );
    }

    /// Panics if not initialized or already stopped.
    pub fn stop_kill(&self) {
        self.handle(sys::EVERYTHING_PLUGIN_PM_STOP, 0 as _);
        self.handle(sys::EVERYTHING_PLUGIN_PM_KILL, 0 as _);
    }

    /// `None` before handling `EVERYTHING_PLUGIN_PM_INIT`
    pub fn get_host(&self) -> Option<&PluginHost> {
        self.host.get()
    }

    /// Not available before handling `EVERYTHING_PLUGIN_PM_INIT`
    pub fn host(&self) -> &PluginHost {
        debug_assert!(self.get_host().is_some(), "Plugin host not inited");
        unsafe { self.get_host().unwrap_unchecked() }
    }

    #[cfg(feature = "rust-i18n")]
    fn init_i18n(data: *mut c_void) {
        let language = if !data.is_null() {
            let host = unsafe { PluginHost::from_data(data) };
            host.config_get_language_name()
        } else {
            PluginHost::get_thread_language_name()
        };

        rust_i18n::set_locale(&language);
    }

    /// Should be called before [`PluginHandler`] is created, as some fields may depend on the locale.
    ///
    /// Already called in the [`plugin_main!`] macro. (Requiring manually calling is a footgun: [IbEverythingExt #100](https://github.com/Chaoses-Ib/IbEverythingExt/issues/100))
    pub fn handle_init_i18n(_msg: u32, _data: *mut c_void) {
        #[cfg(feature = "rust-i18n")]
        if _msg == sys::EVERYTHING_PLUGIN_PM_INIT {
            use std::sync::Once;
            static INIT: Once = Once::new();

            if INIT.is_completed() && !_data.is_null() {
                // If i18n is inited, tracing should also be inited.
                debug!("i18n reinit");
                // Allow reinit (DLL hijacking and plugin)
                Self::init_i18n(_data);
            }
            INIT.call_once(|| {
                Self::init_i18n(_data);
            });
        }
    }

    /// You shouldn't and unlikely need to call this function from multiple threads.
    pub fn handle(&self, msg: u32, data: *mut c_void) -> *mut c_void {
        match msg {
            sys::EVERYTHING_PLUGIN_PM_INIT => {
                if !self.app_is_some() {
                    #[cfg(feature = "tracing")]
                    let _ = log::tracing_try_init();
                    debug!("Plugin init");
                } else {
                    // Allow reinit (DLL hijacking and plugin)
                    debug!("Plugin reinit");
                    self.app_into_config();
                }

                if !data.is_null() {
                    _ = self.host.set(unsafe { PluginHost::from_data(data) });
                }

                *unsafe { &mut *self.instance_name.get() } =
                    PluginHost::instance_name_from_main_thread();
                debug!(instance_name = ?self.instance_name());

                // #[cfg(feature = "rust-i18n")]
                // rust_i18n::set_locale(&self.get_language_name());

                1 as _
            }
            sys::EVERYTHING_PLUGIN_PM_GET_PLUGIN_VERSION => sys::EVERYTHING_PLUGIN_VERSION as _,
            sys::EVERYTHING_PLUGIN_PM_GET_NAME => {
                debug!("Plugin get name");
                match &self.name {
                    Some(name) => name.as_ptr() as _,
                    None => 0 as _,
                }
            }
            sys::EVERYTHING_PLUGIN_PM_GET_DESCRIPTION => {
                debug!("Plugin get description");
                match &self.description {
                    Some(description) => description.as_ptr() as _,
                    None => 0 as _,
                }
            }
            sys::EVERYTHING_PLUGIN_PM_GET_AUTHOR => {
                debug!("Plugin get author");
                match &self.author {
                    Some(author) => author.as_ptr() as _,
                    None => 0 as _,
                }
            }
            sys::EVERYTHING_PLUGIN_PM_GET_VERSION => {
                debug!("Plugin get version");
                match &self.version {
                    Some(version) => version.as_ptr() as _,
                    None => 0 as _,
                }
            }
            sys::EVERYTHING_PLUGIN_PM_GET_LINK => {
                debug!("Plugin get link");
                match &self.link {
                    Some(link) => link.as_ptr() as _,
                    None => 0 as _,
                }
            }
            sys::EVERYTHING_PLUGIN_PM_START => {
                debug!("Plugin start");

                self.app_new(self.load_settings(data));

                1 as _
            }
            sys::EVERYTHING_PLUGIN_PM_STOP => {
                debug!("Plugin stop");

                // TODO

                1 as _
            }
            sys::EVERYTHING_PLUGIN_PM_UNINSTALL => {
                debug!("Plugin uninstall");

                // TODO

                1 as _
            }
            // Always the last message sent to the plugin
            sys::EVERYTHING_PLUGIN_PM_KILL => {
                debug!("Plugin kill");

                self.app_into_config();

                1 as _
            }
            sys::EVERYTHING_PLUGIN_PM_ADD_OPTIONS_PAGES => self.add_options_pages(data),
            sys::EVERYTHING_PLUGIN_PM_LOAD_OPTIONS_PAGE => self.load_options_page(data),
            sys::EVERYTHING_PLUGIN_PM_SAVE_OPTIONS_PAGE => self.save_options_page(data),
            sys::EVERYTHING_PLUGIN_PM_GET_OPTIONS_PAGE_MINMAX => self.get_options_page_minmax(data),
            sys::EVERYTHING_PLUGIN_PM_SIZE_OPTIONS_PAGE => self.size_options_page(data),
            sys::EVERYTHING_PLUGIN_PM_OPTIONS_PAGE_PROC => self.options_page_proc(data),
            sys::EVERYTHING_PLUGIN_PM_KILL_OPTIONS_PAGE => self.kill_options_page(data),
            sys::EVERYTHING_PLUGIN_PM_SAVE_SETTINGS => self.save_settings(data),
            _ => {
                debug!(msg, ?data, "Plugin message");
                0 as _
            }
        }
    }

    pub fn instance_name(&self) -> Option<&str> {
        unsafe { &*self.instance_name.get() }.as_deref()
    }

    fn app_is_some(&self) -> bool {
        let app = unsafe { &*self.app.get() };
        app.is_some()
    }

    fn app_new(&self, config: Option<A::Config>) {
        let app = unsafe { &mut *self.app.get() };
        debug_assert!(app.is_none(), "App already inited");
        *app = Some(A::new(config));
        unsafe { app.as_ref().unwrap_unchecked() }.start();
    }

    fn app_into_config(&self) -> A::Config {
        let app = unsafe { &mut *self.app.get() };
        match app.take() {
            Some(app) => app.into_config(),
            None => unreachable!("App not inited"),
        }
    }

    /// Not available during saving config and recreated afterwards. Use [`Self::with_app`] instead when possible.
    pub unsafe fn app(&self) -> &A {
        unsafe { &*self.app.get() }
            .as_ref()
            .expect("App not inited")
    }

    /// Not available during saving config.
    pub fn with_app<T>(&self, f: impl FnOnce(&A) -> T) -> T {
        f(unsafe { self.app() })
    }
}

/// - [x] `instance_name` (non-official)
/// - [x] `config_*`
/// - [ ] `db_*`
/// - [ ] `debug_*` (tracing)
/// - [ ] `localization_get_*`
/// - [x] `os_enable_or_disable_dlg_item`
/// - [x] `os_get_(local_)?app_data_path_cat_filename`
/// - [x] `plugin_?et_setting_string`
/// - [ ] `property_*`
/// - [x] `ui_options_add_plugin_page`
/// - [x] `utf8_buf_(init|kill)`
/// - [ ] `version_get_*`, `plugin_get_version`
pub struct PluginHost {
    get_proc_address: sys::everything_plugin_get_proc_address_t,
}

impl PluginHost {
    pub fn new(get_proc_address: sys::everything_plugin_get_proc_address_t) -> Self {
        Self { get_proc_address }
    }

    pub unsafe fn from_data(data: *mut c_void) -> Self {
        Self::new(unsafe { mem::transmute(data) })
    }

    fn get_proc_address(
        &self,
    ) -> unsafe extern "system" fn(
        name: *const sys::everything_plugin_utf8_t,
    ) -> *mut ::std::os::raw::c_void {
        unsafe { self.get_proc_address.unwrap_unchecked() }
    }

    /// You can `unwrap_unchecked()` if the API exists in all versions of Everything.
    pub unsafe fn get<T: Copy>(&self, name: &str) -> Option<T> {
        assert_eq!(mem::size_of::<T>(), mem::size_of::<fn()>());

        trace!(name, "Plugin host get proc address");
        let name = CString::new(name).unwrap();
        let ptr = unsafe { (self.get_proc_address())(name.as_ptr() as _) };
        if ptr.is_null() {
            None
        } else {
            // let f: fn() = unsafe { mem::transmute(ptr) };
            Some(unsafe { mem::transmute_copy(&ptr) })
        }
    }

    /// Initialize a cbuf with an empty string.
    ///
    /// The cbuf must be killed with [`Self::utf8_buf_kill`]
    ///
    /// See also [`Self::utf8_buf_kill`]
    ///
    /// ## Note
    /// Usage:
    /// ```ignore
    /// let mut cbuf = MaybeUninit::uninit();
    /// host.utf8_buf_init(cbuf.as_mut_ptr());
    ///
    /// unsafe { os_get_app_data_path_cat_filename(filename.as_ptr() as _, cbuf.as_mut_ptr()) };
    ///
    /// // Or `utf8_buf_kill()`
    /// self.utf8_buf_into_string(cbuf.as_mut_ptr())
    /// ```
    /// Do not move [`sys::everything_plugin_utf8_buf_t`].
    pub fn utf8_buf_init(&self, cbuf: *mut sys::everything_plugin_utf8_buf_t) {
        let utf8_buf_init: unsafe extern "system" fn(cbuf: *mut sys::everything_plugin_utf8_buf_t) =
            unsafe { self.get("utf8_buf_init").unwrap_unchecked() };
        unsafe { utf8_buf_init(cbuf) };
    }

    /// Kill a cbuf initialized with [`Self::utf8_buf_init`].
    ///
    /// Any allocated memory is returned to the system.
    ///
    /// See also [`Self::utf8_buf_init`]
    pub fn utf8_buf_kill(&self, cbuf: *mut sys::everything_plugin_utf8_buf_t) {
        let utf8_buf_kill: unsafe extern "system" fn(cbuf: *mut sys::everything_plugin_utf8_buf_t) =
            unsafe { self.get("utf8_buf_kill").unwrap_unchecked() };
        unsafe { utf8_buf_kill(cbuf) };
    }

    pub fn utf8_buf_into_string(&self, cbuf: *mut sys::everything_plugin_utf8_buf_t) -> String {
        let s = unsafe { (*cbuf).to_string() };
        self.utf8_buf_kill(cbuf);
        s
    }

    pub fn ipc_window_from_main_thread() -> Option<IpcWindow> {
        IpcWindow::from_current_thread()
    }

    pub fn instance_name_from_main_thread() -> Option<String> {
        let ipc_window = Self::ipc_window_from_main_thread();
        ipc_window.and_then(|w| w.instance_name().map(|s| s.to_string()))
    }
}

impl Deref for sys::everything_plugin_utf8_buf_t {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        unsafe {
            // str::from_raw_parts(self.buf, self.len)
            str::from_utf8_unchecked(slice::from_raw_parts(self.buf, self.len))
        }
    }
}
