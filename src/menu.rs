//! 后台左侧菜单模型（对齐 C# 站点菜单树 `MenuTree` 的最小形态）。
//!
//! 菜单由**消费方**提供（`AdminArea::with_menus`），本 crate 只负责：
//! 拼 URL 前缀、序列化成模板可直接遍历的 JSON、给出默认菜单。
//! 这样库升级不会覆盖项目的菜单结构，项目也不必改模板。

use serde_json::{json, Value};

use crate::area;

/// 一个菜单节点：叶子（有 `url`）或分组（有 `children`）。
#[derive(Debug, Clone, Default)]
pub struct AdminMenu {
    /// 显示名
    pub title: String,
    /// 图标（layui 图标类名，可空）
    pub icon: String,
    /// 叶子地址（分组为空）
    pub url: String,
    /// 子菜单
    pub children: Vec<AdminMenu>,
}

impl AdminMenu {
    /// 叶子菜单。
    pub fn item(title: impl Into<String>, url: impl Into<String>) -> AdminMenu {
        AdminMenu {
            title: title.into(),
            url: url.into(),
            ..Default::default()
        }
    }

    /// 带图标的叶子菜单。
    pub fn item_icon(
        title: impl Into<String>,
        url: impl Into<String>,
        icon: impl Into<String>,
    ) -> AdminMenu {
        AdminMenu {
            icon: icon.into(),
            ..Self::item(title, url)
        }
    }

    /// 分组菜单（可折叠）。
    pub fn group(
        title: impl Into<String>,
        icon: impl Into<String>,
        children: Vec<AdminMenu>,
    ) -> AdminMenu {
        AdminMenu {
            title: title.into(),
            icon: icon.into(),
            url: String::new(),
            children,
        }
    }

    /// 是否分组（有子项）。
    pub fn has_children(&self) -> bool {
        !self.children.is_empty()
    }

    /// 把 `{area}` 占位符替换成实际路由前缀（`/admin`），方便消费方写相对菜单。
    pub fn with_area_prefix(mut self, area_name: &str) -> AdminMenu {
        let prefix = area::route_prefix(area_name);
        self.url = self.url.replace("{area}", &prefix);
        for child in &mut self.children {
            child.url = child.url.replace("{area}", &prefix);
        }
        self
    }

    /// 序列化为模板可遍历的对象：
    /// `{ title, icon, url, hasChildren, children: [...] }`。
    pub fn to_value(&self) -> Value {
        let children: Vec<Value> = self.children.iter().map(AdminMenu::to_value).collect();
        json!({
            "title": self.title,
            "icon": self.icon,
            "url": self.url,
            "hasChildren": !children.is_empty(),
            "children": children,
        })
    }

    /// 菜单列表 → JSON 数组。
    pub fn list_to_value(menus: &[AdminMenu]) -> Value {
        Value::Array(menus.iter().map(AdminMenu::to_value).collect())
    }
}

/// 默认菜单（库内置，消费方可用 [`crate::controllers::admin::AdminArea::with_menus`] 覆盖）。
///
/// 只放 **本 crate 自己提供的页面**（首页 / 站点设置），项目自己的功能由项目追加。
pub fn default_menus() -> Vec<AdminMenu> {
    vec![
        AdminMenu::item_icon("后台首页", "{area}/home/index", "layui-icon-home"),
        AdminMenu::item_icon("站点设置", "{area}/config/index", "layui-icon-set"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_serializes_children_and_flags() {
        let menus = vec![AdminMenu::group(
            "内容管理",
            "layui-icon-template-1",
            vec![
                AdminMenu::item("分类管理", "/admin/content/categories"),
                AdminMenu::item("文章管理", "/admin/content/articles"),
            ],
        )];
        let value = AdminMenu::list_to_value(&menus);
        let first = &value[0];
        assert_eq!(first["title"], "内容管理");
        assert_eq!(first["hasChildren"], true);
        assert_eq!(first["children"][1]["url"], "/admin/content/articles");
    }

    #[test]
    fn area_prefix_is_replaced() {
        let menu = AdminMenu::item("首页", "{area}/home/index").with_area_prefix("Biz");
        assert_eq!(menu.url, "/biz/home/index");
    }

    #[test]
    fn default_menus_point_to_own_pages() {
        let menus = default_menus()
            .into_iter()
            .map(|m| m.with_area_prefix("Admin"))
            .collect::<Vec<_>>();
        assert_eq!(menus[0].url, "/admin/home/index");
        assert_eq!(menus[1].url, "/admin/config/index");
    }
}
