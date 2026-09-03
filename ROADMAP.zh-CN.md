# Roadmap（路线图）

本文档描述 `rolling-logger` 的规划方向。它是一份动态文档——随着项目演进，条目
可能被新增、调整或移除。

> English version: [ROADMAP.md](ROADMAP.md)

## 当前状态（v0.3.0）

- 框架无关的滚动文件写入器（`RollingFileWriter`），支持按日期/大小滚动、异步
  gzip 归档、可配置时区、fsync 持久化。
- 三个互斥门面：`tracing`（默认）、`log`、`slog`。
- 统一 `init()` 入口 + 底层原语。
- 门面无关宏层（`trace!` / `debug!` / `info!` / `warn!` / `error!`），跨三个门面
  通用。
- 三个门面均支持彩色控制台 + 纯文本文件双输出。

## 计划中

### 1. 可配置日志格式化

**现状**：日志行格式按门面硬编码（毫秒时间戳、级别、模块/target、消息）。

**目标**：让用户自定义每行的渲染方式，提供一个跨门面共享的配置面。

计划项：

- **时间戳格式**：毫秒（`%Y-%m-%d %H:%M:%S%.3f`）、秒、ISO 8601、RFC 3339，或
  自定义 `strftime` 模式。
- **字段开关**：是否显示 `target`、`module`、`file`、`line`、`thread` id/name。
- **颜色控制**：是否在控制台启用 ANSI 颜色。
- **消息模板**：可配置的布局字符串（如 `"{ts} [{level}] {target} - {msg}"`）。
- **结构化输出**：可选的 JSON / key-value 渲染，便于机器解析。
- **自定义 formatter**：提供一个 trait，让用户提供自己的行渲染器。

可能以 `LogFormat` 结构体落地，与 `LogConfig` 并列，三个门面层共享。

### 2. 桥接第三方 crate 的门面

**问题**：一个 Rust 应用通常依赖许多第三方 crate，它们未必使用同一种日志门面——
有的用 `log`，有的用 `tracing`。如果应用用 `rolling-logger` 只初始化了一个门面，
另一个门面发出的日志就没有 subscriber/logger，被静默丢弃。

具体例子：

- 应用使用 `tracing` 门面，但某个依赖发出 `log::info!` —— 这些行会消失。
- 应用使用 `log` 门面，但某个依赖发出 `tracing::info!` —— 这些事件会消失。

**目标**：提供便捷、可选的桥接，让两个门面的日志都流入同一个滚动写入器。

计划方案：

- **`log` → `tracing`**：接入 [`tracing-log`](https://docs.rs/tracing-log)
  （`LogTracer`），把 `log` 记录转发进 tracing subscriber。
- **`tracing` → `log`**：提供一个桥接（一个 `tracing_subscriber` layer，或内置
  helper），把 tracing 事件转发进 `log` 门面。
- **API**：在 `init()` 上增加配置开关（或独立 feature）自动安装对应桥接，例如
  `bridge_facades: true`，或独立的 `install_log_tracing_bridge()` /
  `install_tracing_log_bridge()` helper。

设计时需解决待定问题：

- 桥接应做成 Cargo feature（如 `bridge-log`、`bridge-tracing`）还是运行时配置字段。
- 两个门面和桥接同时启用时，如何避免双重记录。
- per-target 级别过滤跨桥接时如何生效。

## 待办 / 想法

- **`log` / `slog` 异步写**：tracing 门面通过 `tracing-appender` 异步写，考虑为
  `log`/`slog` 提供类似的非阻塞路径。
- **更多 sink**：滚动到文件的同时支持额外目标（syslog、网络等）。
- **监控指标**：更丰富的丢行 / 滚动 / 归档计数器，暴露给监控。
- **压缩选项**：可配置 gzip 级别，或支持其他压缩格式（如 zstd）。
- **docker**：虚拟化中运行时，处理调度和重启导致的问题。
