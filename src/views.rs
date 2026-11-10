//! 视图约定：`Views/{Area}/{Controller}/{Action}.cshtml`（Razor 子集，双端可渲染）。
//!
//! `dhrust::net::controller::ViewRenderer` 的约定是 `Views/{Controller}/{Action}.cshtml`，
//! 本 crate 在此之上补一层 **Area**（对齐 ASP.NET Core Areas：`/biz/config/index` →
//! `Views/Biz/Config/Index.cshtml`）。控制器前缀本身由 `dhrust` 的
//! [`Controller`](dhrust::net::controller::Controller) 提供，这里只负责视图解析与默认视图落地。
//!
//! **区域可配置**（默认 `Admin`，见 [`crate::area`]）：`Views/{Area}/…` 不存在时
//! 自动回退 `Views/Admin/…`，因此改区域名（如 `Biz`）不必复制默认视图；
//! 需要定制时再在 `Views/Biz/…` 放同名文件覆盖即可（对齐 NCube「子项目视图重写父项目视图」）。
//!
//! 视图引擎是 **Razor 子集**（见 DH.RustBase `Doc/Razor子集模板引擎架构.md` §3.2）：
//! 支持 `@expr`、`@if`、`@foreach`、`@{ var x = ... }`、`Layout`/`RenderBody`/`section`/
//! `Html.PartialAsync`；不支持 `@using`/`@inject`/`@helper`/自由方法调用。
//! 默认视图随 crate 内置，可用 [`materialize`] / [`materialize_for`] 落地后由项目覆盖。

use std::path::{Path, PathBuf};

use dhrust::net::controller::ViewRenderer;
use serde_json::{json, Map, Value};

use crate::area;

/// 内置默认视图（相对视图根的规范路径 → 模板源码）。
///
/// **规范位置固定在 `Admin`**（与 C# `Areas/Admin/Views` 对齐）；其它区域名通过
/// [`materialize_for`] 复制或直接走回退解析。
pub const DEFAULT_VIEWS: &[(&str, &str)] = &[
    ("Shared/_Layout.cshtml", include_str!("../Views/Shared/_Layout.cshtml")),
    ("Shared/Prompt.cshtml", include_str!("../Views/Shared/Prompt.cshtml")),
    (
        "Admin/Home/Index.cshtml",
        include_str!("../Views/Admin/Home/Index.cshtml"),
    ),
    (
        "Admin/Config/Index.cshtml",
        include_str!("../Views/Admin/Config/Index.cshtml"),
    ),
    (
        "Admin/Login/Index.cshtml",
        include_str!("../Views/Admin/Login/Index.cshtml"),
    ),
];

/// 视图根（`Views` 目录）。
#[derive(Debug, Clone)]
pub struct Views {
    root: PathBuf,
}

impl Views {
    /// 以视图根目录创建（默认 `Views`）。
    pub fn new(root: impl Into<PathBuf>) -> Views {
        Views { root: root.into() }
    }

    /// 视图根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 按 Area 约定渲染：`Views/{Area}/{Controller}/{Action}.cshtml`。
    pub fn render(
        &self,
        area_name: &str,
        controller: &str,
        action: &str,
        model: &Value,
    ) -> Result<String, String> {
        self.render_named(area_name, controller, action, model)
    }

    /// 按 Area 约定渲染指定视图（视图名 ≠ 动作名时使用）。
    ///
    /// 区域解析（逐段 Pascal、默认区域回退）由 `dhrust` 的
    /// [`ViewRenderer::with_fallback_area`] 提供，本 crate 不再重复实现。
    pub fn render_named(
        &self,
        area_name: &str,
        controller: &str,
        view: &str,
        model: &Value,
    ) -> Result<String, String> {
        let controller = controller.trim_matches('/');
        let area_name = area_name.trim().trim_matches('/');
        if area_name.is_empty() {
            return Err("区域名不可为空".to_string());
        }

        // Razor 子集对「取不存在的属性」是硬错误（`RenderError`），所以内置模板用到的键
        // 必须先补齐默认值，宿主只传业务字段也不会渲染失败。
        let model = with_template_defaults(model);
        let full_controller = format!("{area_name}/{controller}");
        let renderer = ViewRenderer::with_root(&self.root)
            .with_fallback_area(area::DEFAULT_AREA);
        renderer
            .render(&full_controller, view, view, &model)
            .map_err(|e| format!("渲染 {full_controller}/{view} 失败：{e}"))
    }

    /// 渲染共享视图：`Views/Shared/{Name}.cshtml`（如 `Prompt`）。
    pub fn render_shared(&self, name: &str, model: &Value) -> Result<String, String> {
        let model = with_template_defaults(model);
        let renderer = ViewRenderer::with_root(&self.root);
        renderer
            .render("Shared", name, name, &model)
            .map_err(|e| format!("渲染 Shared/{name} 失败：{e}"))
    }

    /// 视图是否存在（用于部署自检；大小写按文件系统语义）。
    pub fn exists(&self, area_name: &str, controller: &str, action: &str) -> bool {
        self.root
            .join(area_name.trim_matches('/'))
            .join(controller.trim_matches('/'))
            .join(format!("{action}.cshtml"))
            .is_file()
    }

    /// 解析实际使用的区域目录（`{Area}` → 回退 `Admin`）。
    ///
    /// 仅供需要「探活」的调用方使用；真正的渲染解析已交给 `dhrust`。
    pub fn resolve_area(&self, area_name: &str, controller: &str, view: &str) -> Option<String> {
        let requested = area_name.trim().trim_matches('/');
        if requested.is_empty() {
            return None;
        }
        if self.exists(requested, controller, view) {
            return Some(requested.to_string());
        }
        if !area::is_default(requested) && self.exists(area::DEFAULT_AREA, controller, view) {
            return Some(area::DEFAULT_AREA.to_string());
        }
        None
    }
}

impl Default for Views {
    fn default() -> Self {
        Views::new("Views")
    }
}

/// 模板默认键：Razor 子集对「取不存在的属性」是硬错误，
/// 故渲染前先把内置模板会引用到的键补齐（宿主只传业务字段也不会失败）。
fn with_template_defaults(model: &Value) -> Value {
    let mut base = match model {
        Value::Object(map) => map.clone(),
        _ => Map::new(),
    };
    if let Value::Object(defaults) = crate::backend::default_settings() {
        for (key, value) in defaults {
            base.entry(key).or_insert(value);
        }
    }
    for (key, value) in [
        ("title", json!("")),
        ("displayName", json!("")),
        ("userName", json!("")),
        ("saved", json!(false)),
        ("hasLanguages", json!(false)),
        ("languages", json!([])),
        ("message", json!("")),
        ("isOk", json!(false)),
        ("hasBackUrl", json!(false)),
        ("backUrl", json!("")),
        ("autoBack", json!(false)),
        ("hasError", json!(false)),
        ("error", json!("")),
        ("returnUrl", json!("")),
        // 后台首页（Home/Index.cshtml）
        ("areaName", json!(area::DEFAULT_AREA)),
        ("areaRoute", json!(area::route_prefix(area::DEFAULT_AREA))),
        ("uploadPrefix", json!("/upload")),
        ("version", json!(crate::version())),
        ("menus", json!([])),
        ("hasMenus", json!(false)),
    ] {
        base.entry(key.to_string()).or_insert(value);
    }
    Value::Object(base)
}

/// 把内置默认视图落到**指定区域**目录（只补缺失，不覆盖已有文件）。
///
/// 默认视图的规范位置是 `Admin/…`；`area` 非默认时写到 `{Area}/…`
/// （便于项目在改区域名后仍有一份可编辑的副本）。
pub fn materialize_for(root: impl AsRef<Path>, area_name: &str) -> std::io::Result<usize> {
    let normalized = area::normalize(area_name)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.message()))?;
    let root = root.as_ref();
    let mut written = 0;
    for (relative, source) in DEFAULT_VIEWS {
        let target_relative = if area::is_default(&normalized) {
            relative.to_string()
        } else {
            match relative.strip_prefix("Admin/") {
                Some(tail) => format!("{normalized}/{tail}"),
                // `Shared/…` 不随区域变化
                None => relative.to_string(),
            }
        };
        let path = root.join(target_relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        if path.is_file() {
            continue;
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, source)?;
        written += 1;
    }
    Ok(written)
}

/// 把内置默认视图落到**默认区域**（`Admin`）目录（兼容入口）。
pub fn materialize(root: impl AsRef<Path>) -> std::io::Result<usize> {
    materialize_for(root, area::DEFAULT_AREA)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dsmallui-views-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn materialize_writes_defaults_then_skips_existing() {
        let dir = temp_dir("materialize");
        let first = materialize(&dir).unwrap();
        assert_eq!(first, DEFAULT_VIEWS.len());
        let second = materialize(&dir).unwrap();
        assert_eq!(second, 0, "二次落地必须全部跳过（幂等）");
        assert!(dir.join("Admin/Config/Index.cshtml").is_file());
        assert!(dir.join("Shared/_Layout.cshtml").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_for_writes_area_copy() {
        let dir = temp_dir("area");
        let written = materialize_for(&dir, "Biz").unwrap();
        assert_eq!(written, DEFAULT_VIEWS.len());
        assert!(dir.join("Biz/Config/Index.cshtml").is_file());
        assert!(dir.join("Biz/Home/Index.cshtml").is_file());
        // 共享视图不随区域复制
        assert!(dir.join("Shared/_Layout.cshtml").is_file());
        assert!(!dir.join("Biz/Shared/_Layout.cshtml").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_for_rejects_bad_area() {
        let dir = temp_dir("bad");
        let err = materialize_for(&dir, "a/b").unwrap_err();
        assert!(err.to_string().contains("只能包含"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_default_area_falls_back_to_admin_views() {
        let dir = temp_dir("fallback");
        materialize(&dir).unwrap();
        let views = Views::new(&dir);
        // 只存在 Admin 视图，Biz 也应能渲染（回退）
        let html = views
            .render("Biz", "Config", "Index", &json!({ "siteName": "示例站点" }))
            .unwrap();
        let text = crate::decode_html_entities(&html);
        assert!(text.contains("示例站点"), "{}", &text[..text.len().min(400)]);
        assert!(html.contains("</html>"), "应套用布局");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn area_specific_view_overrides_admin() {
        let dir = temp_dir("override");
        materialize(&dir).unwrap();
        std::fs::create_dir_all(dir.join("Biz/Config")).unwrap();
        std::fs::write(
            dir.join("Biz/Config/Index.cshtml"),
            "<p>BIZ-OVERRIDE</p>",
        )
        .unwrap();
        let views = Views::new(&dir);
        let html = views.render("Biz", "Config", "Index", &json!({})).unwrap();
        assert!(html.contains("BIZ-OVERRIDE"), "{html}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn renders_home_page() {
        let dir = temp_dir("home");
        materialize(&dir).unwrap();
        let views = Views::new(&dir);
        let html = views
            .render(
                "Admin",
                "Home",
                "Index",
                &json!({
                    "areaName": "Biz",
                    "areaRoute": "/biz",
                    "menus": [{ "name": "站点设置", "url": "/biz/config/index", "desc": "站点信息" }],
                    "hasMenus": true
                }),
            )
            .unwrap();
        let text = crate::decode_html_entities(&html);
        assert!(text.contains("Biz"), "{}", &text[..text.len().min(400)]);
        assert!(text.contains("站点设置"), "{}", &text[..text.len().min(600)]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn renders_shared_prompt() {
        let dir = temp_dir("prompt");
        materialize(&dir).unwrap();
        let views = Views::new(&dir);
        let html = views
            .render_shared(
                "Prompt",
                &json!({ "message": "保存成功", "isOk": true, "backUrl": "/biz/config/index" }),
            )
            .unwrap();
        let text = crate::decode_html_entities(&html);
        assert!(text.contains("保存成功"), "{}", &text[..text.len().min(400)]);
        assert!(html.contains("/biz/config/index"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_view_reports_path_and_fallback_area() {
        let dir = temp_dir("missing");
        materialize(&dir).unwrap();
        let views = Views::new(&dir);
        let err = views.render("Biz", "Nope", "Index", &Value::Null).unwrap_err();
        assert!(err.contains("Biz/Nope"), "{err}");
        assert!(err.contains("Admin"), "缺少回退区域提示：{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
