//! Scene 层：持久 Node 树（场景图）。
//!
//! 见 `node` 模块。`build_scene` 是入口。

pub mod node;

pub use node::{build_scene, Node, NodeId, NodeKind, Rect, Scene};
