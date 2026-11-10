//! 后台 Area 控制器（对应 C# `Pek.DsMallUI/Areas/Admin`）。
//!
//! **区域名可配置**（对齐 C# `DHSetting.Current.AdminArea` / `AdminArea.AreaName`）：
//! 默认 `Admin`，消费方在装配时改成自己的区域名（如 `Biz`）即可：
//!
//! ```no_run
//! # use pek_dsmallui::DsMallUi;
//! let ui = DsMallUi::builder().admin_area("Biz").build().unwrap();
//! // → /biz/home/index、/biz/config/index、/biz/login/index
//! ```
//!
//! | 路由（`{area}` 由配置决定） | 说明 |
//! |---|---|
//! | `GET /{area}/home/index` | 后台首页（控制台卡片 + 菜单） |
//! | `GET /{area}/config/index` | 站点设置页（渲染 `Views/{Area}/Config/Index.cshtml`） |
//! | `POST /{area}/config/updatebase` | 保存设置（字段名与 C# `ConfigController.UpdateBase` 一致） |
//! | `GET /{area}/login/index` | 登录页 |
//! | `POST /{area}/login/login` | 登录（成功 302 跳后台首页，Ajax 请求返回 JSON 信封） |
//!
//! 「Area」在 `dhrust` 里只是**多段控制器前缀**（`Controller::new("biz/config")`），
//! 路由与参数绑定完全复用 `dhrust::net::controller`；视图解析另加一层 Area 目录约定
//! （见 [`crate::views`]，非默认区域自动回退 `Admin` 模板）。

use std::sync::Arc;

use dhrust::net::controller::{json_result, ActionResult, Controller};
use dhrust::net::http::HttpResponse;
use dhrust::net::router::{Ctx, Router};
use serde_json::{json, Map, Value};

use crate::area;
use crate::backend::{default_settings, AdminBackend, MemoryBackend};
use crate::views::Views;

/// 默认后台区域名（与 C# 默认 `Admin` 一致）。
pub const DEFAULT_ADMIN_AREA: &str = area::DEFAULT_AREA;

/// 指定区域的默认首页路由：`Biz` → `/biz/home/index`。
pub fn home_route(area_name: &str) -> String {
    format!("{}/home/index", area::route_prefix(area_name))
}

/// 后台 Area。
#[derive(Clone)]
pub struct AdminArea {
    backend: Arc<dyn AdminBackend>,
    views: Views,
    area: String,
    home: String,
    upload_prefix: String,
}

impl AdminArea {
    /// 指定数据源与视图根（默认区域 `Admin`，首页 `/{area}/home/index`）。
    pub fn new(backend: Arc<dyn AdminBackend>, views: Views) -> AdminArea {
        let area = area::DEFAULT_AREA.to_string();
        AdminArea {
            backend,
            views,
            home: home_route(&area),
            area,
            upload_prefix: "/upload".to_string(),
        }
    }

    /// 使用内存数据源（示例/测试）。
    pub fn with_memory_backend(views: Views) -> AdminArea {
        AdminArea::new(Arc::new(MemoryBackend::new()), views)
    }

    /// 更换区域名（`Biz` / `biz` 等价；非法字符直接报错）。
    ///
    /// 首页路由随区域名重算（保持 `/{area}/home/index`）。
    pub fn with_area_name(mut self, name: impl AsRef<str>) -> Result<AdminArea, String> {
        let normalized = area::normalize(name.as_ref()).map_err(|e| e.to_string())?;
        self.home = home_route(&normalized);
        self.area = normalized;
        Ok(self)
    }

    /// 覆盖登录成功后的落地页（默认 `/{area}/home/index`）。
    pub fn with_home(mut self, home: impl Into<String>) -> AdminArea {
        self.home = home.into();
        self
    }

    /// 上传 URL 前缀（首页展示用；与 `DsMallUiBuilder::upload_path` 保持一致）。
    pub fn with_upload_prefix(mut self, prefix: impl Into<String>) -> AdminArea {
        let prefix = prefix.into();
        let prefix = prefix.trim().trim_end_matches('/');
        self.upload_prefix = if prefix.is_empty() {
            String::new()
        } else if prefix.starts_with('/') {
            prefix.to_string()
        } else {
            format!("/{prefix}")
        };
        self
    }

    /// 区域名（Pascal，如 `Biz`）。
    pub fn area_name(&self) -> &str {
        &self.area
    }

    /// 路由前缀（小写，如 `/biz`）。
    pub fn route_prefix(&self) -> String {
        area::route_prefix(&self.area)
    }

    /// 登录成功后的落地页。
    pub fn home(&self) -> &str {
        &self.home
    }

    /// 视图根目录。
    pub fn views(&self) -> &Views {
        &self.views
    }

    /// 挂载 Area 的全部控制器（`/{area}/home|config|login/{action}`）。
    pub fn mount(&self, router: &mut Router) {
        let segment = area::route_segment(&self.area);
        let display_name = self.backend.display_name();
        let upload_prefix = self.upload_prefix.clone();

        let backend = self.backend.clone();
        let views = self.views.clone();
        let area_name = self.area.clone();
        Controller::new(&format!("{segment}/home"))
            .get("index", {
                let backend = backend.clone();
                let views = views.clone();
                let area_name = area_name.clone();
                let display_name = display_name.clone();
                let upload_prefix = upload_prefix.clone();
                move |ctx: &Ctx| {
                    let model = home_model(
                        &*backend,
                        &display_name,
                        &area_name,
                        &upload_prefix,
                        Some(ctx),
                    );
                    render(&views, &area_name, "Home", "Index", &model)
                }
            })
            .mount(router);

        let backend = self.backend.clone();
        let views = self.views.clone();
        let area_name = self.area.clone();
        Controller::new(&format!("{segment}/config"))
            .get("index", {
                let backend = backend.clone();
                let views = views.clone();
                let area_name = area_name.clone();
                let display_name = display_name.clone();
                move |ctx: &Ctx| {
                    let model = config_model(&*backend, &display_name, &area_name, Some(ctx), false);
                    render(&views, &area_name, "Config", "Index", &model)
                }
            })
            .post("updatebase", {
                let backend = backend.clone();
                let views = views.clone();
                let area_name = area_name.clone();
                move |ctx: &Ctx| update_base(&*backend, &views, &area_name, ctx)
            })
            .mount(router);

        let backend = self.backend.clone();
        let views = self.views.clone();
        let area_name = self.area.clone();
        let home = self.home.clone();
        Controller::new(&format!("{segment}/login"))
            .get("index", {
                let views = views.clone();
                let area_name = area_name.clone();
                let display_name = display_name.clone();
                move |ctx: &Ctx| {
                    let model = login_model(&display_name, &area_name, Some(ctx), None);
                    render(&views, &area_name, "Login", "Index", &model)
                }
            })
            .post("login", {
                let backend = backend.clone();
                let views = views.clone();
                let area_name = area_name.clone();
                let display_name = display_name.clone();
                let home = home.clone();
                move |ctx: &Ctx| {
                    do_login(&*backend, &views, &area_name, &display_name, &home, ctx)
                }
            })
            .mount(router);
    }

    /// 组装设置页模型（控制器与测试共用）。
    pub fn page_model(&self, saved: bool) -> Value {
        let display_name = self.backend.display_name();
        config_model(&*self.backend, &display_name, &self.area, None, saved)
    }
}

/// 后台首页模型（控制台卡片 + 菜单）。
fn home_model(
    backend: &dyn AdminBackend,
    display_name: &str,
    area_name: &str,
    upload_prefix: &str,
    ctx: Option<&Ctx>,
) -> Value {
    let settings = backend.settings();
    let site_name = settings
        .get("siteName")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    let route = area::route_prefix(area_name);
    let menus = json!([
        {
            "name": "站点设置",
            "url": format!("{route}/config/index"),
            "desc": "站点名称、SEO、客服、页脚与自定义 HTML"
        },
        {
            "name": "登录页",
            "url": format!("{route}/login/index"),
            "desc": "后台登录入口（账号由 AdminBackend 提供）"
        },
        {
            "name": "UEditor 配置",
            "url": "/fileuploader?action=config",
            "desc": "富文本编辑器后端配置（JSON，前端初始化用）"
        }
    ]);

    json!({
        "title": format!("管理后台 - {display_name}"),
        "displayName": display_name,
        "userName": current_user(ctx),
        "siteName": site_name,
        "areaName": area_name,
        "areaRoute": route,
        "uploadPrefix": upload_prefix,
        "version": crate::version(),
        "menus": menus,
        "hasMenus": true,
    })
}

/// 设置页模型：数据源设置 + 模板必备键（标题/用户/多语言/区域等）。
fn config_model(
    backend: &dyn AdminBackend,
    display_name: &str,
    area_name: &str,
    ctx: Option<&Ctx>,
    saved: bool,
) -> Value {
    let mut base = match backend.settings() {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    if let Value::Object(defaults) = default_settings() {
        for (key, value) in defaults {
            base.entry(key).or_insert(value);
        }
    }
    base.insert("title".into(), json!(format!("站点设置 - {display_name}")));
    base.insert("displayName".into(), json!(display_name));
    base.insert("userName".into(), json!(current_user(ctx)));
    base.insert("saved".into(), json!(saved));
    base.insert("areaName".into(), json!(area_name));
    base.insert("areaRoute".into(), json!(area::route_prefix(area_name)));
    // 多语言列表（C# 从 Language.FindByStatus() 取；Rust 侧由数据源提供，缺省为空）
    let languages = base
        .get("languages")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let has_languages = languages
        .as_array()
        .map(|list| !list.is_empty())
        .unwrap_or(false);
    base.insert("languages".into(), languages);
    base.insert("hasLanguages".into(), json!(has_languages));
    Value::Object(base)
}

/// 登录页模型。
fn login_model(
    display_name: &str,
    area_name: &str,
    ctx: Option<&Ctx>,
    error: Option<&str>,
) -> Value {
    let return_url = ctx
        .and_then(|c| c.query_value("returnUrl"))
        .map(|v| v.to_string())
        .unwrap_or_default();
    json!({
        "title": format!("登录 - {display_name}"),
        "displayName": display_name,
        "areaName": area_name,
        "areaRoute": area::route_prefix(area_name),
        "userName": current_user(ctx),
        "returnUrl": return_url,
        "hasError": error.is_some(),
        "error": error.unwrap_or(""),
    })
}

/// 当前登录用户（由宿主鉴权中间件写入 `Ctx::state` 的 `user` 键）。
fn current_user(ctx: Option<&Ctx>) -> String {
    ctx.and_then(|c| c.state_value("user"))
        .unwrap_or("")
        .to_string()
}

/// 渲染视图；失败返回 JSON 500（含具体路径，便于定位）。
fn render(
    views: &Views,
    area_name: &str,
    controller: &str,
    view: &str,
    model: &Value,
) -> ActionResult {
    match views.render(area_name, controller, view, model) {
        Ok(html) => ActionResult::Response(HttpResponse::bytes(
            200,
            "text/html; charset=utf-8",
            html.into_bytes(),
        )),
        Err(e) => {
            dhrust::logs::error!("[DsMallUI] {e}");
            ActionResult::Response(HttpResponse::json(
                500,
                json!({ "code": 500, "message": e }).to_string(),
            ))
        }
    }
}

/// 渲染共享提示页（对应 C# `Prompt(new PromptModel{...})`）。
fn prompt(
    views: &Views,
    message: &str,
    is_ok: bool,
    back_url: Option<&str>,
) -> ActionResult {
    let model = json!({
        "title": if is_ok { "操作成功" } else { "操作提示" },
        "message": message,
        "isOk": is_ok,
        "hasBackUrl": back_url.is_some(),
        "backUrl": back_url.unwrap_or(""),
        "autoBack": is_ok && back_url.is_some(),
    });
    match views.render_shared("Prompt", &model) {
        Ok(html) => ActionResult::Response(HttpResponse::bytes(
            200,
            "text/html; charset=utf-8",
            html.into_bytes(),
        )),
        Err(e) => {
            dhrust::logs::error!("[DsMallUI] {e}");
            ActionResult::Response(HttpResponse::json(
                500,
                json!({ "code": 500, "message": e }).to_string(),
            ))
        }
    }
}

/// 保存站点设置（字段名与 C# `ConfigController.UpdateBase` 参数一致）。
fn update_base(
    backend: &dyn AdminBackend,
    views: &Views,
    area_name: &str,
    ctx: &Ctx,
) -> ActionResult {
    let site_name = ctx.form_value("SiteName").unwrap_or("").trim().to_string();
    if site_name.is_empty() {
        if is_ajax(ctx) {
            return json_result(400, "网站名称不可为空", None);
        }
        return prompt(views, "网站名称不可为空", false, None);
    }

    if let Err(e) = backend.save_settings(&ctx.form) {
        dhrust::logs::warn!("[DsMallUI] 保存站点设置失败：{e}");
        if is_ajax(ctx) {
            return json_result(500, &e, None);
        }
        return prompt(views, &e, false, None);
    }

    if is_ajax(ctx) {
        return json_result(0, "保存成功", None);
    }
    // 与 C# 一致：保存后回到设置页并提示成功（提示页自动回跳）
    prompt(
        views,
        "保存成功",
        true,
        Some(&format!("{}/config/index", area::route_prefix(area_name))),
    )
}

/// 登录。
fn do_login(
    backend: &dyn AdminBackend,
    views: &Views,
    area_name: &str,
    display_name: &str,
    home: &str,
    ctx: &Ctx,
) -> ActionResult {
    let user_name = ctx.form_value("UserName").unwrap_or("").trim().to_string();
    let password = ctx.form_value("Password").unwrap_or("").to_string();
    let return_url = ctx
        .form_value("returnUrl")
        .unwrap_or("")
        .trim()
        .to_string();

    match backend.login(&user_name, &password) {
        Ok(user) => {
            if is_ajax(ctx) {
                return json_result(
                    0,
                    "",
                    Some(json!({
                        "id": user.id,
                        "userName": user.user_name,
                        "displayName": user.display_name,
                        "isAdmin": user.is_admin,
                        "home": home,
                    })),
                );
            }
            let target = safe_redirect(&return_url).unwrap_or_else(|| home.to_string());
            ActionResult::Response(
                HttpResponse::empty(302).with_header("Location", &target),
            )
        }
        Err(e) => {
            dhrust::logs::warn!("[DsMallUI] 登录失败（{user_name}）：{e}");
            if is_ajax(ctx) {
                return json_result(401, &e, None);
            }
            let model = login_model(display_name, area_name, Some(ctx), Some(&e));
            render(views, area_name, "Login", "Index", &model)
        }
    }
}

/// 是否 Ajax 请求（与 C# `Request.IsAjaxRequest()` 同口径）。
fn is_ajax(ctx: &Ctx) -> bool {
    if let Some(value) = ctx.header("x-requested-with") {
        if value.eq_ignore_ascii_case("xmlhttprequest") {
            return true;
        }
    }
    ctx.header("accept")
        .map(|value| value.to_ascii_lowercase().contains("application/json"))
        .unwrap_or(false)
}

/// 站内跳转目标校验：只允许以单个 `/` 开头的相对路径（防开放重定向）。
fn safe_redirect(target: &str) -> Option<String> {
    let target = target.trim();
    if !target.starts_with('/') || target.starts_with("//") || target.contains('\\') {
        return None;
    }
    Some(target.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::materialize_for;
    use dhrust::net::http::{HttpOutcome, HttpRequest};
    use std::path::PathBuf;

    fn temp_views(tag: &str, area_name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsmallui-admin-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        materialize_for(&dir, area_name).unwrap();
        dir
    }

    fn ctx(method: &str, path: &str, query: &str, form: &str) -> Ctx {
        Ctx::build(HttpRequest {
            method: method.into(),
            path: path.into(),
            query: query.into(),
            headers: vec![(
                "content-type".to_string(),
                "application/x-www-form-urlencoded".to_string(),
            )],
            body: form.as_bytes().to_vec().into(),
            remote_addr: None,
        })
    }

    fn body_of(action: ActionResult) -> (u16, String) {
        match action {
            ActionResult::Response(response) => (
                response.status,
                String::from_utf8_lossy(&response.body).to_string(),
            ),
            ActionResult::View { .. } => (0, String::from("<view>")),
        }
    }

    fn call(handler: &dhrust::net::http::HttpHandler, path: &str) -> (u16, String) {
        let request = HttpRequest {
            method: "GET".into(),
            path: path.into(),
            query: String::new(),
            headers: Vec::new(),
            body: Vec::new().into(),
            remote_addr: None,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        match runtime.block_on(handler(request)) {
            HttpOutcome::Response(response) => (
                response.status,
                String::from_utf8_lossy(&response.body).to_string(),
            ),
            _ => (0, String::new()),
        }
    }

    #[test]
    fn default_area_is_admin() {
        let views = Views::new(temp_views("default", "Admin"));
        let area = AdminArea::with_memory_backend(views);
        assert_eq!(area.area_name(), DEFAULT_ADMIN_AREA);
        assert_eq!(area.route_prefix(), "/admin");
        assert_eq!(area.home(), "/admin/home/index");
    }

    #[test]
    fn area_name_is_configurable() {
        let views = Views::new(temp_views("biz", "Biz"));
        let area = AdminArea::with_memory_backend(views)
            .with_area_name("biz")
            .unwrap();
        assert_eq!(area.area_name(), "Biz");
        assert_eq!(area.route_prefix(), "/biz");
        assert_eq!(area.home(), "/biz/home/index");
    }

    #[test]
    fn invalid_area_is_rejected() {
        let views = Views::new(temp_views("bad", "Admin"));
        let err = AdminArea::with_memory_backend(views)
            .with_area_name("a/b")
            .err()
            .expect("非法区域名必须报错");
        assert!(err.contains("只能包含"), "{err}");
    }

    #[test]
    fn routes_are_mounted_under_configured_area() {
        let views = Views::new(temp_views("routes", "Biz"));
        let area = AdminArea::with_memory_backend(views)
            .with_area_name("Biz")
            .unwrap();
        let mut router = Router::new();
        area.mount(&mut router);
        let handler = router.into_handler();

        let (status, body) = call(&handler, "/biz/home/index");
        assert_eq!(status, 200, "{body}");
        let text = crate::decode_html_entities(&body);
        assert!(text.contains("管理后台"), "{}", &text[..text.len().min(400)]);

        let (status, body) = call(&handler, "/biz/config/index");
        assert_eq!(status, 200, "{body}");

        let (status, _) = call(&handler, "/admin/home/index");
        assert_eq!(status, 404, "默认区域不应再挂载");
    }

    #[test]
    fn home_page_lists_area_menus() {
        let views = Views::new(temp_views("home", "Biz"));
        let area = AdminArea::with_memory_backend(views)
            .with_area_name("Biz")
            .unwrap();
        let mut router = Router::new();
        area.mount(&mut router);
        let (status, body) = call(&router.into_handler(), "/biz/home/index");
        assert_eq!(status, 200);
        let text = crate::decode_html_entities(&body);
        assert!(text.contains("站点设置"), "{}", &text[..text.len().min(600)]);
        assert!(text.contains("/biz/config/index"), "{}", &body[..body.len().min(600)]);
    }

    #[test]
    fn config_page_renders_with_defaults() {
        let views = Views::new(temp_views("page", "Admin"));
        let area = AdminArea::with_memory_backend(views);
        let model = area.page_model(false);
        assert_eq!(model["hasLanguages"], false);
        assert!(model["languages"].is_array());
        assert!(model.get("siteName").is_some());
        let html = area
            .views()
            .render("Admin", "Config", "Index", &model)
            .unwrap();
        assert!(html.contains("站点设置"), "{}", &html[..html.len().min(300)]);
    }

    #[test]
    fn update_base_requires_site_name() {
        let views = Views::new(temp_views("empty", "Biz"));
        let backend = MemoryBackend::new();
        let ctx = ctx("POST", "/biz/config/updatebase", "", "SiteName=&SeoTitle=x");
        let (status, body) = body_of(update_base(&backend, &views, "Biz", &ctx));
        assert_eq!(status, 200);
        let text = crate::decode_html_entities(&body);
        assert!(text.contains("网站名称不可为空"), "{text}");
    }

    #[test]
    fn update_base_ajax_returns_json() {
        let views = Views::new(temp_views("ajax", "Biz"));
        let backend = MemoryBackend::new();
        let mut ctx = ctx(
            "POST",
            "/biz/config/updatebase",
            "",
            "SiteName=%E5%95%86%E5%9F%8E",
        );
        ctx.req.headers.push((
            "x-requested-with".to_string(),
            "XMLHttpRequest".to_string(),
        ));
        let (status, body) = body_of(update_base(&backend, &views, "Biz", &ctx));
        assert_eq!(status, 200);
        assert!(body.contains("\"code\":0"), "{body}");
        assert_eq!(backend.settings()["siteName"], "商城");
    }

    #[test]
    fn login_success_redirects_to_area_home() {
        let views = Views::new(temp_views("login", "Biz"));
        let backend = MemoryBackend::new();
        let home = home_route("Biz");

        let bad = ctx("POST", "/biz/login/login", "", "UserName=admin&Password=wrong");
        let (status, body) = body_of(do_login(&backend, &views, "Biz", "后台", &home, &bad));
        assert_eq!(status, 200);
        let text = crate::decode_html_entities(&body);
        assert!(text.contains("用户名或密码不正确"), "{text}");

        let ok = ctx(
            "POST",
            "/biz/login/login",
            "",
            "UserName=admin&Password=admin123",
        );
        let (status, body) = body_of(do_login(&backend, &views, "Biz", "后台", &home, &ok));
        assert_eq!(status, 302, "{body}");
    }

    #[test]
    fn safe_redirect_blocks_open_redirect() {
        assert!(safe_redirect("//evil.com/x").is_none());
        assert!(safe_redirect("https://evil.com").is_none());
        assert_eq!(
            safe_redirect("/biz/config/index").as_deref(),
            Some("/biz/config/index")
        );
    }
}
