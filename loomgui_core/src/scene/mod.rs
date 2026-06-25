//! Scene 层：持久 Node 树（场景图）。
//!
//! 见 `node` 模块。`build_scene` 是入口。

pub mod node;
pub mod transform; // v1d.3

pub use node::{Node, NodeId, NodeKind, Rect, Scene};
#[cfg(feature = "parse")]
pub use node::build_scene;
