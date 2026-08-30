# Roadmap

This document outlines the planned direction for `rolling-logger`. It is a
living document — items may be added, reordered, or removed as the project
evolves.

> 中文版见 [ROADMAP.zh-CN.md](ROADMAP.zh-CN.md)

## Current status (v0.3.0)

- A facade-agnostic rolling file writer (`RollingFileWriter`) with date/size
  rotation, async gzip archival, configurable timezone, and fsync durability.
- Three mutually exclusive facades: `tracing` (default), `log`, and `slog`.
- A unified `init()` entry plus low-level primitives.
- A facade-agnostic macro layer (`trace!` / `debug!` / `info!` / `warn!` /
  `error!`) that works across all three facades.
- Colored console + plain-text file output for all three facades.

## Planned

### 1. Configurable log formatting

**Current state**: the log line format is hard-coded per facade (timestamp with
milliseconds, level, module/target, and message).

**Goal**: let users customize how each line is rendered, with a shared
configuration surface that works across facades.

Planned options:

- **Timestamp format**: millisecond (`%Y-%m-%d %H:%M:%S%.3f`), seconds, ISO 8601,
  RFC 3339, or a custom `strftime` pattern.
- **Field toggles**: enable/disable `target`, `module`, `file`, `line`, and
  `thread` ids/names.
- **Color control**: enable/disable ANSI colors on the console.
- **Message templates**: a configurable layout string (e.g.
  `"{ts} [{level}] {target} - {msg}"`).
- **Structured output**: optional JSON / key-value rendering for machine parsing.
- **Custom formatter**: a trait so users can provide their own line renderer.

This likely lands as a `LogFormat` struct alongside `LogConfig`, shared by the
three facade layers.

### 2. Bridging third-party crate facades

**Problem**: a Rust application usually depends on many third-party crates, and
they don't all use the same logging facade — some use `log`, others use
`tracing`. If the application initializes `rolling-logger` with one facade, the
logs emitted through the *other* facade have no subscriber/logger and are
silently dropped.

Concrete examples:

- The app uses the `tracing` facade, but a dependency emits `log::info!` — those
  lines vanish.
- The app uses the `log` facade, but a dependency emits `tracing::info!` — those
  events vanish.

**Goal**: provide convenient, opt-in bridging so logs from both facades flow into
the same rolling writer.

Planned approach:

- **`log` → `tracing`**: integrate with [`tracing-log`](https://docs.rs/tracing-log)
  (`LogTracer`) so `log` records are forwarded into the tracing subscriber.
- **`tracing` → `log`**: provide a bridge (a `tracing_subscriber` layer, or a
  bundled helper) that forwards tracing events into the `log` facade.
- **API**: a config flag (or dedicated feature) on `init()` to install the
  relevant bridge automatically, e.g. `bridge_facades: true`, or standalone
  helpers like `install_log_tracing_bridge()` / `install_tracing_log_bridge()`.

Open questions to resolve during design:

- Whether bridging should be a Cargo feature (e.g. `bridge-log`, `bridge-tracing`)
  or a runtime config field.
- How to avoid double-logging when both facades and a bridge are active
  simultaneously.
- How per-target level filtering should behave across the bridge.

## Backlog / ideas

- **Async writes for `log` / `slog`**: the tracing facade writes asynchronously
  via `tracing-appender`; consider a similar non-blocking path for `log`/`slog`.
- **More sinks**: rotate to files *and* additional targets (syslog, network, etc.).
- **Metrics**: richer dropped-line / rotation / archival counters exposed for
  monitoring.
- **Compression options**: configurable gzip level, or support for other
  compression formats (e.g. zstd).
