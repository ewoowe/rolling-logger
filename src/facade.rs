//! 门面无关的日志宏抽象层。
//!
//! 提供 [`trace!`] / [`debug!`] / [`info!`] / [`warn!`] / [`error!`] 五个等级宏，
//! 根据编译期启用的 feature 代理到对应门面（`tracing` 或 `log`）。
//!
//! 这些宏既可供下游调用方直接使用（`use rolling_logger::info;`），也用于本
//! crate 内部（如滚动/归档的自诊断，见 `writer.rs`）。
//!
//! # 代理规则
//!
//! - 启用 `tracing`（默认）→ 代理到 [`tracing`] 的对应宏
//! - 启用 `log-backend` → 代理到 [`log`] 的对应宏
//! - 未启用任何门面 → 降级为 no-op（完全惰性：参数不求值、零开销）
//!
//! 仅支持两个门面的**公共子集**语法：`info!("msg {}", x)` 与
//! `info!(target: "...", "msg {}", x)`。tracing 特有的结构化字段语法
//! （如 `info!(field = v, "msg")`）不在门面无关范围内，需要时请直接使用
//! `tracing::info!`。

// ─────────────────────────────────────────────────────────────────────────────
// 门面宏 re-export：把当前门面的 5 个等级宏以同名 re-export 到本模块，
// 供下面的对外宏通过 `$crate::facade::<name>!` 转发（保证宏卫生）。
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "tracing")]
#[doc(hidden)]
pub use tracing::{debug, error, info, trace, warn};

#[cfg(all(feature = "log-backend", not(feature = "tracing")))]
#[doc(hidden)]
pub use log::{debug, error, info, trace, warn};

// ─────────────────────────────────────────────────────────────────────────────
// 对外宏（门面无关）
//
// 有门面时转发到上面的 re-export；无门面时降级为 no-op。
// 注意：`#[cfg]` 不能出现在 `macro_rules!` 宏体内部，因此每个等级写「有门面」/
// 「无门面」两个版本，用 `#[cfg]` 控制哪一个生效。
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! trace {
    ($($arg:tt)+) => { $crate::facade::trace!($($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend")))]
#[macro_export]
macro_rules! trace {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => { $crate::facade::debug!($($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend")))]
#[macro_export]
macro_rules! debug {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => { $crate::facade::info!($($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend")))]
#[macro_export]
macro_rules! info {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => { $crate::facade::warn!($($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend")))]
#[macro_export]
macro_rules! warn {
    ($($arg:tt)+) => {};
}

#[cfg(any(feature = "tracing", feature = "log-backend"))]
#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => { $crate::facade::error!($($arg)+) };
}
#[cfg(not(any(feature = "tracing", feature = "log-backend")))]
#[macro_export]
macro_rules! error {
    ($($arg:tt)+) => {};
}
