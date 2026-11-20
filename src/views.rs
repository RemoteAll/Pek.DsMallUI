//! 视图约定：`Areas/{Area}/Views/{Controller}/{Action}.cshtml`（Razor 子集，双端可渲染）。
//!
//! 本 crate 在 `dhrust` 的 [`ViewRenderer`] 之上只做三件事：
//! 1. **Area 视图位置**由 [`ViewLayout`]（= `dhrust::net::controller::ViewConvention`）统一给出，
//!    查找顺序、项目覆盖库、`Shared` 回退、`_ViewStart` 约定布局全部复用 `dhrust`；
//! 2. **内置视图表**由 `build.rs` **编译期扫描 `Views/` 目录**生成（不手写清单），
//!    路径一律经约定推导，不在这里写死字符串路径；
//! 3. **页面级静态资源**（`{视图}.css` / `{视图}.js`）只登记磁盘目录与前缀，
//!    由 `dhrust` 按约定注入 —— **不读内容、不嵌入二进制**。
//!
//! 目录约定（`Areas`）：
//! ```text
//! Areas/Admin/Views/_ViewStart.cshtml      ← 区域默认布局（约定，非写死）
//! Areas/Admin/Views/Shared/_Layout.cshtml  ← 布局（项目可覆盖库同名文件）
//! Areas/Admin/Views/Config/Index.cshtml    ← 内容页（无需写 Layout）
//! ```
//!
//! **多份布局时怎么选（与 C# 同口径，优先级从高到低）**：
//! 1. 页面里 `Layout = "_LayoutX"` —— 显式指定，最高优先级；
//! 2. 页面里 `Layout = ""` —— 明确「不套布局」（独立整页，如登录页）；
//! 3. `_ViewStart.cshtml` 里的 `Layout = "_Layout"` —— 区域默认值（就近目录生效，
//!    `Areas/Admin/Views/Config/_ViewStart.cshtml` 优先于 `Areas/Admin/Views/_ViewStart.cshtml`）。
//!
//! 所以「同时存在多个布局」不会歧义：**页面显式 > 就近 `_ViewStart` > 更上层 `_ViewStart`**。
//!
//! 视图引擎是 **Razor 子集**（见 DH.RustBase `Doc/Razor子集模板引擎架构.md` §3.2）：
//! 支持 `@expr`、`@if`、`@foreach`、`@{ var x = ... }`、`Layout`/`RenderBody`/`section`/
//! `Html.PartialAsync`；不支持 `@using`/`@inject`/`@helper`/自由方法调用。

use std::path::{Path, PathBuf};

use dhrust::net::controller::{PageAssets, ViewRenderer};
use serde_json::{json, Map, Value};

use crate::area;
// 编译期清单（`build.rs` 扫描 `Areas/` 生成）：相对路径 → 模板源码。
// 相对路径与 `ViewConvention::Areas` 的查找路径逐字节一致 —— 目录结构即约定。
include!(concat!(env!("OUT_DIR"), "/embedded_views.rs"));

pub use dhrust::net::controller::ViewConvention as ViewLayout;

/// 内置视图目录（`Areas`，对齐 C#）；清单里的相对路径就是它下面的路径。
pub const VIEWS_ROOT_NAME: &str = "Areas";

/// 内置视图清单（编译期生成）：`(相对路径, 源码)`。
///
/// 例如 `("Admin/Views/Shared/_Layout.cshtml", "…")`；没有分类函数、没有区域常量：
/// 想加/改视图就往 `Areas/{区域}/Views/…` 动文件，重新编译即可。
pub fn embedded_view_files() -> &'static [(&'static str, &'static str)] {
    EMBEDDED_VIEWS
}

/// 清单里视图的「相对名」（不含 `.cshtml`，供 `ViewRenderer::with_embedded_views` 使用）。
pub fn embedded_names(layout: ViewLayout) -> Vec<(String, String)> {
    embedded_files_for(layout)
        .into_iter()
        .map(|(path, source)| {
            (
                path.trim_end_matches(".cshtml").to_string(),
                source.to_string(),
            )
        })
        .collect()
}

/// 清单 → `(相对路径（含 .cshtml）, 源码)`，路径按约定映射：
///
/// - `Areas`：与清单**逐字节一致**（C# 规范，目录即约定）；
/// - `Flat`：按同一套约定函数换算（`Shared/` → 根 `Shared/`，区域根文件 → `{Area}/`）。
pub fn embedded_files_for(layout: ViewLayout) -> Vec<(String, &'static str)> {
    EMBEDDED_VIEWS
        .iter()
        .map(|(path, source)| {
            let mapped = match layout {
                ViewLayout::Areas => (*path).to_string(),
                ViewLayout::Flat => flat_path(layout, path).unwrap_or_else(|| (*path).to_string()),
            };
            (mapped, *source)
        })
        .collect()
}

/// `Areas` 相对路径 → `Flat` 相对路径（全部经 [`ViewLayout`] 的约定函数推导）。
fn flat_path(layout: ViewLayout, relative: &str) -> Option<String> {
    let without_ext = relative.strip_suffix(".cshtml")?;
    let (area, rest) = without_ext.split_once("/Views/")?;
    if let Some(name) = rest.strip_prefix("Shared/") {
        return Some(layout.shared_path(area, name));
    }
    if !rest.contains('/') {
        // 区域根约定文件（`_ViewStart` 等）
        return Some(layout.area_root_path(area, rest));
    }
    let (controller, view) = rest.rsplit_once('/')?;
    Some(layout.view_path(area, controller, view))
}
/// 视图根（`Views` / `Areas` 目录）。
#[derive(Debug, Clone)]
pub struct Views {
    root: PathBuf,
    layout: ViewLayout,
    /// 页面级静态资源约定（`None` = 不注入）。
    assets: Option<PageAssets>,
    /// 显式登记的资源（对齐 C# `PekHtml.AddCssFileParts` / `AddScriptParts`）。
    extra_assets: crate::assets::Assets,
    /// 解析缓存 TTL（毫秒；`0` = 改文件立即生效）。
    meta_ttl_ms: u64,
}

impl Views {
    /// 以视图根目录创建（默认 `Views`，平铺布局）。
    pub fn new(root: impl Into<PathBuf>) -> Views {
        Views {
            root: root.into(),
            layout: ViewLayout::Flat,
            assets: None,
            extra_assets: crate::assets::Assets::new(),
            meta_ttl_ms: dhrust::net::controller::DEFAULT_META_TTL_MS,
        }
    }

    /// 以 C# Areas 根目录创建（通常传 `Areas`）。
    pub fn areas(root: impl Into<PathBuf>) -> Views {
        Views {
            root: root.into(),
            layout: ViewLayout::Areas,
            assets: None,
            extra_assets: crate::assets::Assets::new(),
            meta_ttl_ms: dhrust::net::controller::DEFAULT_META_TTL_MS,
        }
    }

    /// 启用页面级静态资源约定（`{视图}.css` / `{视图}.js` 自动注入，磁盘直读）。
    pub fn with_page_assets(mut self, assets: Option<PageAssets>) -> Views {
        self.assets = assets;
        self
    }

    /// 显式登记页面资源（对齐 C# `PekHtml.AddCssFileParts` / `AddScriptParts`）；
    /// 渲染时以 `Model.assets` 交给布局遍历输出标签。
    pub fn with_extra_assets(mut self, assets: crate::assets::Assets) -> Views {
        self.extra_assets = assets;
        self
    }

    /// 设置解析缓存 TTL（毫秒；调测期传 `0` = 保存即生效）。
    pub fn with_meta_ttl_ms(mut self, ms: u64) -> Views {
        self.meta_ttl_ms = ms;
        self
    }

    /// 当前页面资源约定。
    pub fn page_assets(&self) -> Option<&PageAssets> {
        self.assets.as_ref()
    }

    /// 构造渲染器：约定 + 回退区域 + 内置模板 + 页面资源。
    fn renderer(&self) -> ViewRenderer {
        ViewRenderer::with_root(&self.root)
            .with_convention(self.layout)
            .with_fallback_area(area::DEFAULT_AREA)
            .with_embedded_views(self.embedded_views())
            .with_page_assets(self.assets.clone())
            .with_meta_ttl_ms(self.meta_ttl_ms)
    }

    /// 内置模板表：`相对名（不含 .cshtml）→ 源码`（由目录清单 + 约定推导）。
    ///
    /// 只含 `.cshtml` 模板（文本），**不含任何 css/js/图片等静态资源**。
    pub fn embedded_views(&self) -> Vec<(String, String)> {
        embedded_names(self.layout)
    }

    /// 当前目录布局。
    pub fn layout(&self) -> ViewLayout {
        self.layout
    }

    /// 视图根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 按 Area 约定渲染：
    /// - `Flat`：`Views/{Area}/{Controller}/{Action}.cshtml`
    /// - `Areas`：`Areas/{Area}/Views/{Controller}/{Action}.cshtml`
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
        let mut model = with_template_defaults(model);
        if let Some(map) = model.as_object_mut() {
            // C# `PekHtml` 那套登记的资源，统一以 `assets` 交给布局 foreach 输出
            map.insert("assets".into(), self.extra_assets.to_value());
        }
        let full_controller = self.layout.controller(area_name, controller);
        self.renderer()
            .render(&full_controller, view, view, &model)
            .map_err(|e| format!("渲染 {full_controller}/{view} 失败：{e}"))
    }

    /// 渲染共享片段视图（**不套** `_ViewStart` 布局，对齐 C# Partial）。
    pub fn render_shared(&self, name: &str, model: &Value) -> Result<String, String> {
        let model = with_template_defaults(model);
        let controller = self.layout.shared_controller(area::DEFAULT_AREA);
        self.renderer()
            .render_isolated(&controller, name, name, &model)
            .map_err(|e| format!("渲染 Shared/{name} 失败：{e}"))
    }

    /// 视图是否存在（用于部署自检；大小写按文件系统语义）。
    pub fn exists(&self, area_name: &str, controller: &str, action: &str) -> bool {
        let area_name = area_name.trim_matches('/');
        let base = match self.layout {
            ViewLayout::Flat => self.root.join(area_name).join(controller.trim_matches('/')),
            ViewLayout::Areas => self
                .root
                .join(area_name)
                .join("Views")
                .join(controller.trim_matches('/')),
        };
        base.join(format!("{action}.cshtml")).is_file()
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

    /// 按当前布局落地默认视图（只补缺失，不覆盖已有文件）。
    pub fn materialize(&self, area_name: &str) -> std::io::Result<usize> {
        self.materialize_inner(area_name, false)
    }

    /// 强制刷新内置视图（覆盖已有文件；用于升级模板，调用方需自行备份定制）。
    pub fn refresh(&self, area_name: &str) -> std::io::Result<usize> {
        self.materialize_inner(area_name, true)
    }

    /// 落地实现：**清单里的相对路径原样落盘**（`Areas` 约定）；
    /// `Flat` 约定下按同一条机械变换去掉 `Views/` 段。
    fn materialize_inner(&self, area_name: &str, overwrite: bool) -> std::io::Result<usize> {
        let _ = area::normalize(area_name)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e.message()))?;
        let mut written = 0;
        for (target, source) in embedded_files_for(self.layout) {
            let path = self.root.join(target.replace('/', std::path::MAIN_SEPARATOR_STR));
            if path.is_file() && !overwrite {
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
    // `Model.assets`：布局里的资源清单（对齐 C# `PekHtml`），未登记时给空清单
    base.entry("assets".to_string()).or_insert_with(|| {
        crate::assets::Assets::new().to_value()
    });
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
        ("countdownTime", json!(3)),
        ("currentAction", json!("Index")),
        ("bgRnd", json!(1)),
        ("captchaEnabled", json!(true)),
        ("captchaUrl", json!("/captcha")),
        ("home", json!("")),
        ("mainUrl", json!("")),
        // 后台首页（Home/Index.cshtml）
        ("areaName", json!(area::DEFAULT_AREA)),
        ("areaRoute", json!(area::route_prefix(area::DEFAULT_AREA))),
        ("uploadPrefix", json!("/upload")),
        ("version", json!(crate::version())),
        ("menus", json!([])),
        ("hasMenus", json!(false)),
        ("leftMenus", json!([])),
        ("hasLeftMenus", json!(false)),
        ("currentPath", json!("")),
        ("logoutUrl", json!("")),
        ("copyright", json!("")),
    ] {
        base.entry(key.to_string()).or_insert(value);
    }
    Value::Object(base)
}

/// 把内置视图按约定落到**指定视图根**（只补缺失，不覆盖已有文件）。
///
/// 落盘位置就是库内目录结构（`Areas/{区域}/Views/…` 或 `Flat` 等价路径），
/// **不按调用方区域复制**：改区域名靠回退解析，若要定制就在对应位置放同名文件。
pub fn materialize_for(root: impl AsRef<Path>, area_name: &str) -> std::io::Result<usize> {
    Views::new(root.as_ref()).materialize(area_name)
}

/// 把内置默认视图按 **C# Areas 目录规范**落地（`Areas/{Area}/Views/...`）。
pub fn materialize_areas_for(root: impl AsRef<Path>, area_name: &str) -> std::io::Result<usize> {
    Views::areas(root.as_ref()).materialize(area_name)
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
        assert_eq!(first, EMBEDDED_VIEWS.len());
        let second = materialize(&dir).unwrap();
        assert_eq!(second, 0, "二次落地必须全部跳过（幂等）");
        assert!(dir.join("Admin/Config/Index.cshtml").is_file());
        assert!(dir.join("Shared/_AdminLayout.cshtml").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn materialize_for_writes_library_views() {
        let dir = temp_dir("area");
        let written = materialize_for(&dir, "Biz").unwrap();
        assert_eq!(written, EMBEDDED_VIEWS.len());
        // 库视图位置 = C# 约定位置（不按调用方区域复制；改名区域走回退解析）
        assert!(dir.join("Admin/Config/Index.cshtml").is_file());
        assert!(dir.join("Shared/_AdminLayout.cshtml").is_file());
        // 二次落地幂等
        assert_eq!(materialize_for(&dir, "Biz").unwrap(), 0);
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
    fn areas_layout_materializes_and_renders() {
        let dir = temp_dir("areas");
        let views = Views::areas(&dir);
        assert_eq!(views.materialize("Admin").unwrap(), EMBEDDED_VIEWS.len());
        assert!(dir.join("Admin/Views/Config/Index.cshtml").is_file());
        assert!(dir.join("Admin/Views/Shared/_AdminLayout.cshtml").is_file());
        assert!(dir.join("Admin/Views/_ViewStart.cshtml").is_file());
        assert!(dir.join("Admin/Views/Shared/Prompt.cshtml").is_file());

        let html = views
            .render("Admin", "Config", "Index", &json!({ "siteName": "示例站点" }))
            .unwrap();
        let text = crate::decode_html_entities(&html);
        assert!(text.contains("站点设置"), "{}", &text[..text.len().min(400)]);
        assert!(text.contains("示例站点"), "{}", &text[..text.len().min(800)]);
        assert!(html.contains("</html>"), "应套用 Areas 布局");
        // 只有 `_AdminLayout`（与 C# 同名同路径）；顶栏/侧栏由框架包提供，本项目不自造
        assert!(html.contains("/static/admin/css/admin.css"), "缺少内容页布局样式");
    }

    #[test]
    fn library_ships_only_csharp_skin_views() {
        // 分层与 C# `Pek.DsMallUI` 一致：库只带 `Config` / `Login` / `_AdminLayout` 这类皮肤视图；
        // `Home/*`（后台骨架与工作区）属于站点项目，不随库下发。
        let dir = temp_dir("skin-curated");
        let views = Views::areas(&dir);
        views.materialize("Admin").unwrap();
        assert!(dir.join("Admin/Views/Config/Index.cshtml").is_file());
        assert!(dir.join("Admin/Views/Login/Index.cshtml").is_file());
        assert!(dir.join("Admin/Views/Shared/_AdminLayout.cshtml").is_file());
        assert!(
            !dir.join("Admin/Views/Home/DashBoard.cshtml").exists(),
            "Home 视图应由项目提供，库不应下发"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn login_page_opts_out_of_layout_by_convention() {
        let dir = temp_dir("login-nolayout");
        let views = Views::areas(&dir);
        views.materialize("Admin").unwrap();
        let html = views
            .render("Admin", "Login", "Index", &json!({ "displayName": "站点" }))
            .unwrap();
        assert!(html.contains("login_body"), "应是登录整页");
        assert!(!html.contains("layui-layout-admin"), "登录页不套后台框架");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn library_views_render_without_materialize() {
        // 不落地任何文件，全部走库内置模板 + 回退区域
        let dir = temp_dir("embedded");
        let views = Views::areas(&dir);
        let html = views
            .render("Admin", "Config", "Index", &json!({ "siteName": "内置" }))
            .unwrap();
        let text = crate::decode_html_entities(&html);
        assert!(text.contains("内置"), "{}", &text[..text.len().min(400)]);
        assert!(
            html.contains("/static/admin/css/admin.css"),
            "布局应来自库内置模板"
        );
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
    fn project_provided_home_views_use_admin_layout() {
        // `Home/*` 由项目提供（库不下发）；写了项目视图后应能正常套 `_AdminLayout`
        let dir = temp_dir("home");
        materialize(&dir).unwrap();
        std::fs::create_dir_all(dir.join("Admin/Home")).unwrap();
        std::fs::write(
            dir.join("Admin/Home/Main.cshtml"),
            "<a href=\"/biz/config/index\">站点设置</a>",
        )
        .unwrap();
        let views = Views::new(&dir);
        let html = views
            .render(
                "Admin",
                "Home",
                "Main",
                &json!({ "displayName": "站点" }),
            )
            .unwrap();
        let text = crate::decode_html_entities(&html);
        assert!(text.contains("站点设置"), "{}", &text[..text.len().min(600)]);
        assert!(html.contains("/static/admin/css/admin.css"), "应套用 _AdminLayout");
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
