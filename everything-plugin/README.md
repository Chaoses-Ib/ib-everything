# everything-plugin
[![crates.io](https://img.shields.io/crates/v/everything-plugin.svg)](https://crates.io/crates/everything-plugin)
[![Documentation](https://docs.rs/everything-plugin/badge.svg)](https://docs.rs/everything-plugin)
[![License](https://img.shields.io/crates/l/everything-plugin.svg)](../LICENSE.txt)

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

## Build
### Static CRT
`.cargo/config.toml`:
```toml
[target.'cfg(all(target_os = "windows", target_env = "msvc"))']
rustflags = ["-C", "target-feature=+crt-static"]
```
Increase build size by ~100 KiB.

## Debugging
- `.\Everything64.exe -debug`
  
  Unlike `-debug`, `-debug-log` doesn't work with stdout/stderr outputs.

## Plugins using this library
- [IbEverythingExt: Everything 拼音搜索, ローマ字検索, wildcard, quick select, Shell extension](https://github.com/Chaoses-Ib/IbEverythingExt)
