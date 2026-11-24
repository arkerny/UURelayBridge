//! SNI 相关纯逻辑：TLS ClientHello 解析 / 分片、DNS 查询、hosts 管理。
//!
//! 这里不碰平台 API，也不做网络监听，方便单测；
//! 真正的本地转发服务在 app crate 的 `hostmode` 里。

pub mod dns;
pub mod doh;
pub mod hosts;
pub mod tls;
