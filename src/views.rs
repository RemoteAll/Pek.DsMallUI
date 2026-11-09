//! 视图约定：`Views/{Area}/{Controller}/{Action}.cshtml`（Razor 子集，双端可渲染）。
//!
//! `dhrust::net::controller::ViewRenderer` 的约定是 `Views/{Controller}/{Action}.cshtml`，
//! 本 crate 在此之上补一层 **Area**（对齐 ASP.NET Core Areas：`/admin/config/index` →
//! `Views/Admin/Config/Index.cshtml`）。控制器前缀本身由 `dhrust` 的
//! [`Controller`](dhrust::net::controller::Controller) 提供，这里只负责视图解析与默认视图落地。
//!
//! 视图引擎是 **Razor 子集**（见 DH.RustBase `Doc/Razor子集模板引擎架构.md` §3.2）：
//! 支持 `@expr`、`@if`、`@foreach`、`@{ var x = ... }`、`Layout`/`RenderBody`/`section`/
//! `Html.PartialAsync`；不支持 `@using`/`@inject`/`@helper`/自由方法调用。
//! 因此从 C# 参考工程迁过来的页面必须**改写成子集**（C# 版大量 `@PekHtml.*`/`@T()` 不可用），
//! 改写后的默认视图随 crate 内置，可用 [`materialize`] 落地后由项目自行覆盖。

use std::path::{Path, PathBuf};

use dhrust::net::controller::ViewRenderer;
use serde_json::{json, Map, Value};

/// 内置默认视图（相对视图根目录的路径 → 模板源码）。
///
/// 与 C# 参考工程 `Pek.DsMallUI/Areas/Admin/Views` 的页面一一对应，但改写成 Razor 子集。
pub const DEFAULT_VIEWS: &[(&str, &str)] = &[
    ("Shared/_Layout.cshtml", include_str!("../Views/Shared/_Layout.cshtml")),
    ("Shared/Prompt.cshtml", include_str!("../Views/Shared/Prompt.cshtml")),
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
        area: &str,
        controller: &str,
        action: &str,
        model: &Value,
    ) -> Result<String, String> {
        self.render_named(area, controller, action, model)
    }

    /// 按 Area 约定渲染指定视图（视图名 ≠ 动作名时使用）。
    pub fn render_named(
        &self,
        area: &str,
        controller: &str,
        view: &str,
        model: &Value,
    ) -> Result<String, String> {
        let controller = format!(
            "{}/{}",
            area.trim_matches('/'),
            controller.trim_matches('/')
        );
        // Razor 子集对「取不存在的属性」是硬错误（`RenderError`），所以内置模板用到的键
        // 必须先补齐默认值，宿主只传业务字段也不会渲染失败。
        let model = with_template_defaults(model);
        let renderer = ViewRenderer::with_root(&self.root);
        renderer
            .render(&controller, view, view, &model)
            .map_err(|e| format!("渲染 {controller}/{view} 失败：{e}"))
    }

    /// 渲染共享视图：`Views/Shared/{Name}.cshtml`（如 `Prompt`）。
    pub fn render_shared(&self, name: &str, model: &Value) -> Result<String, String> {
        let model = with_template_defaults(model);
        let renderer = ViewRenderer::with_root(&self.root);
        renderer
            .render("Shared", name, name, &model)
            .map_err(|e| format!("渲染 Shared/{name} 失败：{e}"))
    }

    /// 视图是否存在（用于部署自检）。
    pub fn exists(&self, area: &str, controller: &str, action: &str) -> bool {
        self.root
            .join(area)
            .join(controller)
            .join(format!("{action}.cshtml"))
            .is_file()
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
    ] {
        base.entry(key.to_string()).or_insert(value);
    }
    Value::Object(base)
}

/// 把内置默认视图落到磁盘（**只补缺失，不覆盖已有文件**，便于项目二次定制）。
///
/// 返回新写入的文件数；已存在（含被项目改过的）一律跳过。
pub fn materialize(root: impl AsRef<Path>) -> std::io::Result<usize> {
    let root = root.as_ref();
    let mut written = 0;
    for (relative, source) in DEFAULT_VIEWS {
        let path = root.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
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
    fn renders_config_page_with_layout() {
        let dir = temp_dir("render");
        materialize(&dir).unwrap();
        let views = Views::new(&dir);
        let model = serde_json::json!({
            "siteName": "示例站点",
            "seoTitle": "标题",
            "saved": false,
        });
        let html = views.render("Admin", "Config", "Index", &model).unwrap();
        let text = crate::decode_html_entities(&html);
        assert!(text.contains("示例站点"), "{}", &text[..text.len().min(400)]);
        assert!(html.contains("<form"), "应渲染出表单");
        assert!(html.contains("</html>"), "应套用布局");
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
                &serde_json::json!({ "message": "保存成功", "isOk": true, "backUrl": "/admin/config/index" }),
            )
            .unwrap();
        let text = crate::decode_html_entities(&html);
        assert!(text.contains("保存成功"), "{}", &text[..text.len().min(400)]);
        assert!(html.contains("/admin/config/index"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_view_reports_path() {
        let dir = temp_dir("missing");
        materialize(&dir).unwrap();
        let views = Views::new(&dir);
        let err = views.render("Admin", "Nope", "Index", &Value::Null).unwrap_err();
        assert!(err.contains("Admin/Nope"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
