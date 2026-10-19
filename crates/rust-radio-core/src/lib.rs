//! 云听接口客户端。
//!
//! 只做数据层：签名、列表、详情、节目单、选流、缓存、本地索引搜索。
//! 播放与 UI 在 `rust-radio-audio` / `rust-radio-app`。

pub mod cache;
pub mod client;
pub mod models;
pub mod search;
pub mod sign;

pub use client::{ApiError, Client, Freshness, GATEWAY, GATEWAY_BACKUP};
pub use models::{
    Line, Program, Province, Quality, Station, StationDetail, StreamKind, StreamTarget,
};
pub use search::StationIndex;
