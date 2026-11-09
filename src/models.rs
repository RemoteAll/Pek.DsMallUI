//! 通用模型（对应 C# `Pek.DsMallUI/Models`）。
//!
//! 目前只有 [`Hierarchy`]（折叠实体）：C# 参考工程里用于把「分类 / 区域」等树形数据
//! 拍平成带层级的列表交给前端渲染。字段名保持 C# 原名（`gc_id` / `gc_name` …），
//! 便于与既有前端 JS 直接对接。

use serde::{Deserialize, Serialize};

/// 折叠实体（C# `Pek.DsMallUI.Models.Hierarchy`）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hierarchy {
    /// 当前层级 / 深度
    pub deep: i32,
    /// 编号
    pub gc_id: i64,
    /// 名称
    pub gc_name: Option<String>,
    /// 父编号
    pub gc_parent_id: i64,
    /// 类型
    #[serde(rename = "type")]
    pub kind: i32,
    /// 父区域编号
    #[serde(rename = "ParentCode")]
    pub parent_code: i64,
    /// 区域编号
    #[serde(rename = "AreaCode")]
    pub area_code: i64,
    /// 排序
    #[serde(rename = "Sort")]
    pub sort: i64,
    /// 是否显示
    pub gc_show: i32,
    /// 排序
    pub gc_sort: i32,
    /// 是否有子节点
    pub have_child: i32,
    /// 添加人
    #[serde(rename = "CreateUser")]
    pub create_user: String,
    /// 添加时间
    #[serde(rename = "CreateTime")]
    pub create_time: Option<String>,
    /// 是否系统
    #[serde(rename = "IsSystem")]
    pub is_system: bool,
}

/// 树节点的最小抽象（`id` / `parent_id` / `name`）。
pub trait TreeItem {
    /// 主键
    fn id(&self) -> i64;
    /// 父键（根为 0 或负数）
    fn parent_id(&self) -> i64;
    /// 显示名
    fn name(&self) -> &str;
}

impl TreeItem for Hierarchy {
    fn id(&self) -> i64 {
        self.gc_id
    }

    fn parent_id(&self) -> i64 {
        self.gc_parent_id
    }

    fn name(&self) -> &str {
        self.gc_name.as_deref().unwrap_or("")
    }
}

/// 把任意树节点拍平成 [`Hierarchy`] 列表（深度优先，`deep` 从 0 开始）。
///
/// 与 C# 侧 `HaveChild` / `Deep` 的填法一致：父在前、子在紧随其后，
/// 列表顺序即前端展示顺序；孤立节点（父不存在）按根处理，避免数据脏导致丢节点。
pub fn flatten<T: TreeItem>(items: &[T]) -> Vec<Hierarchy> {
    let ids: Vec<i64> = items.iter().map(TreeItem::id).collect();
    let roots: Vec<&T> = items
        .iter()
        .filter(|item| {
            let parent = item.parent_id();
            parent == item.id() || !ids.contains(&parent)
        })
        .collect();

    /// 深度优先把节点写进结果（父在前、子紧随）。
    fn walk<T: TreeItem>(
        items: &[T],
        node: &T,
        deep: i32,
        out: &mut Vec<Hierarchy>,
        guard: &mut usize,
    ) {
        if *guard > 100_000 {
            return;
        }
        *guard += 1;
        let children: Vec<&T> = items
            .iter()
            .filter(|candidate| candidate.parent_id() == node.id() && candidate.id() != node.id())
            .collect();
        out.push(Hierarchy {
            deep,
            gc_id: node.id(),
            gc_name: Some(node.name().to_string()),
            gc_parent_id: node.parent_id(),
            have_child: if children.is_empty() { 0 } else { 1 },
            ..Default::default()
        });
        for child in children {
            walk(items, child, deep + 1, out, guard);
        }
    }

    let mut out = Vec::with_capacity(items.len());
    let mut guard = 0usize;
    for root in roots {
        walk(items, root, 0, &mut out, &mut guard);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn serializes_csharp_field_names() {
        let node = Hierarchy {
            gc_id: 1,
            gc_name: Some("根".into()),
            ..Default::default()
        };
        let value = serde_json::to_value(&node).unwrap();
        assert_eq!(value["gc_id"], json!(1));
        assert_eq!(value["gc_name"], json!("根"));
        assert!(value.get("type").is_some());
        assert!(value.get("ParentCode").is_some());
        assert!(value.get("IsSystem").is_some());
    }

    struct Node(i64, i64, &'static str);
    impl TreeItem for Node {
        fn id(&self) -> i64 {
            self.0
        }
        fn parent_id(&self) -> i64 {
            self.1
        }
        fn name(&self) -> &str {
            self.2
        }
    }

    #[test]
    fn flatten_orders_parent_before_children() {
        let items = vec![
            Node(2, 1, "子"),
            Node(1, 0, "根"),
            Node(3, 1, "子二"),
        ];
        let flat = flatten(&items);
        let ids: Vec<i64> = flat.iter().map(|h| h.gc_id).collect();
        assert_eq!(ids.first().copied(), Some(1));
        assert_eq!(flat[0].deep, 0);
        assert_eq!(flat[0].have_child, 1);
        assert_eq!(flat.len(), 3);
    }

    #[test]
    fn orphan_is_treated_as_root() {
        let items = vec![Node(9, 404, "孤立")];
        let flat = flatten(&items);
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].deep, 0);
    }
}
