# rolling-logger

A production-grade rolling file logger. Its core rolling writer is a
facade-agnostic `io::Write` implementation that can target the three mainstream
logging facades: [`tracing`](https://docs.rs/tracing), [`log`](https://docs.rs/log)
and [`slog`](https://docs.rs/slog).

> 中文文档见 [README.zh-CN.md](README.zh-CN.md) · See [README.zh-CN.md](README.zh-CN.md) for the Chinese version.

## Features

- **Date + size dual rotation**: filenames like `{prefix}.{YYYY-MM-DD}.log`, with
  a `{seq}` suffix when the size limit is exceeded.
- **Automatic gzip archival** of historical logs into `history/`: async,
  concurrency-limited, atomic, crash-safe.
- **Configurable archive delay in days**: negative = archive on rotation; `0` =
  archive yesterday and earlier; `1` = archive the day before yesterday and earlier.
- **Configurable log timezone** (IANA name, e.g. `"UTC"` / `"Asia/Shanghai"`),
  for cross-timezone deployment.
- **Optional fsync durability**: force flush to disk on `flush`, no log loss on crash.
- **Dropped-line monitoring**: query the number of lines dropped when the channel is full.
- **Graceful shutdown**: `LoggerGuard` flushes buffers and waits for archiver
  threads on drop.
- **Console + file output**: colored console, plain-text file.
- **Facade-agnostic macros**: `trace!` / `debug!` / `info!` / `warn!` / `error!`
  proxy to the enabled facade automatically — code doesn't care whether it's
  `tracing` or `log` underneath.

## Installation

Add to `Cargo.toml`:

```toml
[dependencies]
rolling-logger = "0.1"
```

## Facade support (features)

| feature | default | underlying facade |
| --- | --- | --- |
| `tracing` | ✅ | [`tracing`](https://docs.rs/tracing) (`tracing-subscriber`) |
| `log-backend` | ❌ | [`log`](https://docs.rs/log) |
| `slog-backend` | ❌ | [`slog`](https://docs.rs/slog) |

The three facades are **mutually exclusive** (enabling more than one fails
compilation). The core writer `RollingFileWriter` is a facade-agnostic
`io::Write`, shared by all three facades for the same rolling/archival capability.

Regardless of the facade, initialization always uses the single `init` entry:

```rust
use rolling_logger::{init, LogConfig};

let config = LogConfig { /* ... */ };
let _guard = init(&config)?;
```

## Quick start

```rust
use rolling_logger::{init, LogConfig};

fn main() -> anyhow::Result<()> {
    let config = LogConfig {
        dir: "./logs".into(),
        level: "info".into(),
        file_prefix: "app".into(),
        max_file_size_mb: 10,
        max_files: 30,
        archive_delay_days: 0,
        archive_batch_size: 100,
        fsync_on_flush: false,
        timezone: "UTC".into(),
    };

    // Unified init entry; the facade is decided by the feature (tracing by default).
    let _guard = init(&config)?;

    tracing::info!("hello, rolling-logger!");
    tracing::warn!("this goes to both console and file");

    Ok(())
}
```

Targeting the `log` facade (disable the default tracing):

```toml
[dependencies]
rolling-logger = { version = "0.1", default-features = false, features = ["log-backend"] }
```

```rust
use rolling_logger::{init, LogConfig};

let config = LogConfig { /* ... */ };
let _guard = init(&config)?;
log::info!("hello via log facade");
```

Targeting the `slog` facade (disable the default tracing):

```toml
[dependencies]
rolling-logger = { version = "0.1", default-features = false, features = ["slog-backend"] }
```

```rust
use rolling_logger::{init, LogConfig};

let config = LogConfig { /* ... */ };
let guard = init(&config)?;
let log = guard.logger();   // slog macros need an explicit logger
slog::info!(log, "hello via slog"; "user_id" => 42);
```

Runnable examples: [`examples/tracing.rs`](examples/tracing.rs) (tracing facade),
[`examples/log.rs`](examples/log.rs) (log facade) and
[`examples/slog.rs`](examples/slog.rs) (slog facade).

## Facade-agnostic macros

This crate provides five facade-agnostic macros that proxy to the enabled facade
at compile time, so business code doesn't care whether it's `tracing` or `log`:

```rust
use rolling_logger::{debug, error, info, trace, warn};

trace!("trace level");
debug!("debug level, value {}", x);
info!("info level");
warn!("warn level");
error!("error level");

// The target syntax is also supported (common subset of the two facades).
info!(target: "my_component", "a log line with a target");
```

Proxy rules:

| enabled feature | macros proxy to |
| --- | --- |
| `tracing` (default) | `tracing::trace!` etc. |
| `log-backend` | `log::trace!` etc. |
| `slog-backend` / none | no-op (args not evaluated, zero cost) |

> **Boundary 1**: the facade-agnostic macros only support the **common subset**
> syntax of `tracing` / `log` (`info!("msg {}", x)` and `info!(target: "...", ...)`).
> For `tracing`-specific structured fields (e.g. `info!(field = v, "msg")`), use
> `tracing::info!` directly.
>
> **Boundary 2**: the `slog` facade is **not** part of the facade-agnostic macro
> layer. slog macros need an explicit logger and use `; key => value` structured
> syntax (no `{}` positional args), which is incompatible with `tracing`/`log`.
> Use slog's native macros directly under the slog facade.

## Configuration

| field | type | default | description |
| --- | --- | --- | --- |
| `dir` | `String` | — | Directory where log files are stored. |
| `level` | `String` | — | Log level filter rule, e.g. `"info,my_crate=debug"`. |
| `file_prefix` | `String` | — | Log filename prefix. |
| `max_file_size_mb` | `u64` | — | Max size of a single log file (MB). |
| `max_files` | `usize` | — | Max archived files to retain (`0` = unlimited). |
| `archive_delay_days` | `i64` | `0` | Archive delay in days; negative = archive on rotation. |
| `archive_batch_size` | `usize` | `100` | Max files to archive per pass. |
| `fsync_on_flush` | `bool` | `false` | Whether to force fsync on flush. |
| `timezone` | `String` | `"UTC"` | Timezone for log timestamps (IANA name). |

## Directory layout

```
logs/
├── app.2026-08-28.log          # historical log (moved to history/ after archiving)
├── app.2026-08-29.log          # current log
└── history/
    ├── app.2026-08-28.log.gz   # compressed archive
    └── ...
```

## Advanced usage

### Custom level filtering

Under the `tracing` facade, `level` follows `tracing_subscriber::EnvFilter` syntax:

```rust
let config = LogConfig {
    level: "info,my_app=debug,hyper=warn".into(),
    // ...other fields
};
```

> Under the `log` / `slog` facades, only a single global level is supported; the
> first token of `level` is taken (e.g. `"info"` above), and per-target rules are
> ignored.

### Monitor dropped lines

```rust
let guard = init(&config)?;
// ...after running for a while
eprintln!("dropped file lines: {}", guard.dropped_file_lines());
```

> Only the `tracing` facade provides `dropped_file_lines()` (the `log` / `slog`
> facades write synchronously and have no drop channel).

### Parse a timezone manually

```rust
use rolling_logger::parse_timezone;

let tz = parse_timezone("Asia/Shanghai"); // falls back to UTC on failure
```

## License

[MIT](LICENSE)
