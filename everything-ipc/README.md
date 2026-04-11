# everything-ipc
[![crates.io](https://img.shields.io/crates/v/everything-ipc.svg)](https://crates.io/crates/everything-ipc)
[![Documentation](https://docs.rs/everything-ipc/badge.svg)](https://docs.rs/everything-ipc)
[![License](https://img.shields.io/crates/l/everything-ipc.svg)](../LICENSE.txt)

Rust port of voidtools' Everything's IPC SDK.
Can be used to search user files quickly.

## Features
- Support both Everything v1.4 and v1.5, including Alpha version.
- Higher performance than Everything v1.4's official SDK:
  - Hot query time is about 30% shorter.
  - Sending blocking time is 60% shorter for async queries.
- Support both sync and async (Tokio) querying.
- Search text generating utilities.
- Folder-based batch IPC and cache.

See [documentation](https://docs.rs/everything-ipc) for details.

## Usage
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
