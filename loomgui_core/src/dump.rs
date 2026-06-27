//! v1d.3：整树 JSON dump（调试用，spec §3.8）。
use crate::scene::node::{NodeKind, Scene};

/// JSON 字符串转义：处理 `"` → `\"`、`\` → `\\`、控制字符 → `\uXXXX`。
pub fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < ' ' => out.push_str(&format!("\\u{:04X}", c as u32)),
            _ => out.push(ch),
        }
    }
    out
}

/// 整树 JSON：每节点 {node_id, parent, tag, id, classes, kind, layout, world_matrix, visible}。
pub fn dump_scene_json(scene: &Scene) -> String {
    let mut s = String::from("[");
    for (i, n) in scene.nodes.iter().enumerate() {
        if i > 0 { s.push(','); }
        let (tag, kind_str) = match &n.kind {
            NodeKind::Container => ("div", "Container"),
            NodeKind::Button => ("button", "Button"),
            NodeKind::Text { .. } => ("span", "Text"),
            NodeKind::Image { src: _src } => ("img", "Image"),
        };
        let id = json_escape(n.id_attr.as_deref().unwrap_or(""));
        let classes = n.classes.iter().map(|c| json_escape(c)).collect::<Vec<_>>().join(" ");
        let wm = if n.id.0 < scene.world_transforms.len() { &scene.world_transforms[n.id.0] } else { &crate::transform::IDENTITY };
        // v1d.4 诊断：附 anim.transform 是否 Some + opacity 值，定位 tween 是否真写进 anim。
        let (anim_tr, anim_op) = match scene.anim.0.get(n.id.0) {
            Some(a) => (a.transform.is_some(), a.opacity),
            None => (false, None),
        };
        let op_str = anim_op.map(|v| format!("{:.3}", v)).unwrap_or_else(|| "null".into());
        s.push_str(&format!(
            r#"{{"node_id":{},"parent":{},"tag":"{}","id":"{}","classes":"{}","kind":"{}","layout":{{"x":{},"y":{},"w":{},"h":{}}},"world_matrix":[{},{},{},{},{},{}],"anim_tr":{},"anim_op":{},"visible":{}}}"#,
            n.id.0, n.parent.map(|p| p.0.to_string()).unwrap_or("-1".into()),
            tag, id, classes, kind_str,
            n.layout_rect.x, n.layout_rect.y, n.layout_rect.w, n.layout_rect.h,
            wm[0], wm[1], wm[2], wm[3], wm[4], wm[5],
            anim_tr, op_str,
            true, // visible：v1 无独立 visible 字段，恒 true（clip/touchable 另列）
        ));
    }
    s.push(']');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::node::{Node, NodeId, Rect, Scene};

    #[test]
    fn dump_has_node_fields() {
        let mut n = Node::default();
        n.id = NodeId(0);
        n.id_attr = Some("root".into());
        n.classes = vec!["main".into()];
        n.layout_rect = Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0 };
        let s = Scene { roots: vec![NodeId(0)], nodes: vec![n], dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new() };
        let json = dump_scene_json(&s);
        assert!(json.contains(r#""id":"root""#), "含 id");
        assert!(json.contains(r#""classes":"main""#), "含 classes");
        assert!(json.contains(r#""x":1"#), "含 layout.x");
        assert!(json.contains(r#""y":2"#), "含 layout.y");
        assert!(json.contains(r#""w":3"#), "含 layout.w");
        assert!(json.contains(r#""world_matrix":[1,0,0,1,0,0]"#), "identity world_matrix");
    }

    #[test]
    fn dump_escapes_quotes_in_id() {
        let mut n = Node::default();
        n.id = NodeId(0);
        n.id_attr = Some("a\"b".into());
        let s = Scene { roots: vec![NodeId(0)], nodes: vec![n], dynamic_rules: Default::default(), focused_node: None, world_transforms: Vec::new(), anim: Default::default(), scroll: Default::default(), text_layouts: Vec::new() };
        let json = dump_scene_json(&s);
        assert!(json.contains(r#""id":"a\"b""#), "id 中的引号被转义：{}", json);
    }
}
