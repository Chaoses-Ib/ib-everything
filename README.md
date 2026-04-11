# ib-everything
Rust/C++ port of voidtools' [Everything](https://www.voidtools.com/)'s IPC/plugin SDK.

Can be used to search user files quickly.

## [everything-ipc](everything-ipc/README.md)
[![crates.io](https://img.shields.io/crates/v/everything-ipc.svg)](https://crates.io/crates/everything-ipc)
[![Documentation](https://docs.rs/everything-ipc/badge.svg)](https://docs.rs/everything-ipc)
[![License](https://img.shields.io/crates/l/everything-ipc.svg)](LICENSE.txt)

Rust port of Everything's IPC SDK.

Features:
- Support both Everything v1.4 and v1.5, including Alpha version.
- Higher performance than Everything v1.4's official SDK:
  - Hot query time is about 30% shorter.
  - Sending blocking time is 60% shorter for async queries.
- Support both sync and async (Tokio) querying.
- Search text generating utilities.
- Folder-based batch IPC and cache.

See [documentation](https://docs.rs/everything-ipc) for details.

### Usage
```rust
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

## [everything-plugin](everything-plugin/README.md)
[![crates.io](https://img.shields.io/crates/v/everything-plugin.svg)](https://crates.io/crates/everything-plugin)
[![Documentation](https://docs.rs/everything-plugin/badge.svg)](https://docs.rs/everything-plugin)
[![License](https://img.shields.io/crates/l/everything-plugin.svg)](LICENSE.txt)

Rust binding for [Everything](https://www.voidtools.com/)'s [plugin SDK](https://www.voidtools.com/forum/viewtopic.php?t=16535).

Features:
- Load and save config with [Serde](https://github.com/serde-rs/serde)
- Make options pages GUI using [Winio](https://github.com/compio-rs/winio) in MVU (Elm) architecture
- Internationalization with [rust-i18n](https://github.com/longbridge/rust-i18n)
- Log with [tracing](https://github.com/tokio-rs/tracing)

Example:
```rust
mod options;

#[derive(Serialize, Deserialize, Debug, Default)]
pub struct Config {
    s: String,
}

pub struct App {
    config: Config,
}

impl PluginApp for App {
    type Config = Config;

    fn new(config: Option<Self::Config>) -> Self {
        Self {
            config: config.unwrap_or_default(),
        }
    }

    fn config(&self) -> &Self::Config {
        &self.config
    }

    fn into_config(self) -> Self::Config {
        self.config
    }
}

plugin_main!(App, {
    PluginHandler::builder()
        .name("Test Plugin")
        .description("A test plugin for Everything")
        .author("Chaoses-Ib")
        .version("0.1.0")
        .link("https://github.com/Chaoses-Ib/IbEverythingLib")
        .options_pages(vec![
            OptionsPage::builder()
                .name("Test Plugin")
                .load(ui::winio::spawn::<options::MainModel>)
                .build(),
        ])
        .build()
});
```

## [everything-cpp](everything-cpp)
A C++17 implementation of [Everything](https://www.voidtools.com/)'s (IPC) SDK.

### Features
- Higher performance. Compared with [the official SDK](https://www.voidtools.com/support/everything/sdk/), it reduces the query time by about 30%.
- Better asynchronous. Its sending blocking time is only 40% of the SDK. And it is based on [`std::future`](https://en.cppreference.com/w/cpp/thread/future.html), which gives you more features about asynchronous.
- Support [named instances](https://www.voidtools.com/en-us/support/everything/multiple_instances/#named_instances).
- Header-only and does not depend on the official DLL.

## See also
### Projects using this library
- [ib-shell: Some desktop environment libraries, mainly for Windows Shell](https://github.com/Chaoses-Ib/ib-shell)
- [IbDOpusExt: An extension for Directory Opus.](https://github.com/Chaoses-Ib/IbDOpusExt)

### Everything plugins using this library
- [IbEverythingExt: Everything 拼音搜索, ローマ字検索, wildcard, quick select, Shell extension](https://github.com/Chaoses-Ib/IbEverythingExt)

### Other bindings
Rust bindings (depending on the official DLL) for Everything's (IPC) SDK:
- [reedHam/everything-wrapper: Everything sdk wrapper for rust using bindgen.](https://github.com/reedHam/everything-wrapper)  
  [Rust SDK Wrapper - voidtools forum](https://www.voidtools.com/forum/viewtopic.php?t=13256)
- [owtotwo/everything-sdk-rs: An ergonomic Everything(voidtools) SDK wrapper in Rust. (Supports async and raw sdk functions)](https://github.com/owtotwo/everything-sdk-rs)
  - License: GPLv3
- [Ciantic/everything-sys-rs: VoidTools' Everything library as Rust crate](https://github.com/Ciantic/everything-sys-rs/)
