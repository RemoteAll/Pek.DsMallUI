//! 后台 Area 控制器（对应 C# `Pek.DsMallUI/Areas/Admin`）。
//!
//! | 路由 | 说明 |
//! |---|---|
//! | `GET /admin/config/index` | 站点设置页（渲染 `Views/Admin/Config/Index.cshtml`） |
//! | `POST /admin/config/updatebase` | 保存设置（表单字段名与 C# `ConfigController.UpdateBase` 一致） |
//! | `GET /admin/login/index` | 登录页（`Views/Admin/Login/Index.cshtml`） |
//! | `POST /admin/login/login` | 登录（成功 302 跳转，Ajax 请求返回 JSON 信封） |
//!
//! 「Area」在 `dhrust` 里只是**多段控制器前缀**（`Controller::new("admin/config")`），
//! 路由与参数绑定完全复用 `dhrust::net::controller`；视图解析另加一层 Area 目录约定
//! （见 [`crate::views`]）。

use std::sync::Arc;

use dhrust::net::controller::{json_result, ActionResult, Controller};
use dhrust::net::http::HttpResponse;
use dhrust::net::router::{Ctx, Router};
use serde_json::{json, Map, Value};

use crate::backend::{default_settings, AdminBackend, MemoryBackend};
use crate::views::Views;

/// 默认后台首页（登录成功后的跳转目标）。
pub const DEFAULT_ADMIN_ROUTE: &str = "/admin/config/index";

/// 后台 Area。
#[derive(Clone)]
pub struct AdminArea {
    backend: Arc<dyn AdminBackend>,
    views: Views,
    area: String,
    home: String,
}

impl AdminArea {
    /// 指定数据源与视图根（`views` 通常为 `<项目根>/Views`）。
    pub fn new(backend: Arc<dyn AdminBackend>, views: Views) -> AdminArea {
        AdminArea {
            backend,
            views,
            area: "Admin".to_string(),
            home: DEFAULT_ADMIN_ROUTE.to_string(),
        }
    }

    /// 使用内存数据源（示例/测试）。
    pub fn with_memory_backend(views: Views) -> AdminArea {
        AdminArea::new(Arc::new(MemoryBackend::new()), views)
    }

    /// 覆盖登录成功后的落地页。
    pub fn with_home(mut self, home: impl Into<String>) -> AdminArea {
        self.home = home.into();
        self
    }

    /// 视图根目录。
    pub fn views(&self) -> &Views {
        &self.views
    }

    /// 挂载 Area 的全部控制器。
    pub fn mount(&self, router: &mut Router) {
        let config = format!("{}/config", self.area.to_ascii_lowercase());
        let login = format!("{}/login", self.area.to_ascii_lowercase());

        let backend = self.backend.clone();
        let views = self.views.clone();
        let display_name = self.backend.display_name();
        Controller::new(&config)
            .get("index", {
                let backend = backend.clone();
                let views = views.clone();
                let display_name = display_name.clone();
                move |ctx: &Ctx| {
                    let model = config_model(&*backend, &display_name, Some(ctx), false);
                    render(&views, "Config", "Index", &model)
                }
            })
            .post("updatebase", {
                let backend = backend.clone();
                let views = views.clone();
                let display_name = display_name.clone();
                move |ctx: &Ctx| update_base(&*backend, &views, &display_name, ctx)
            })
            .mount(router);

        let backend = self.backend.clone();
        let views = self.views.clone();
        let display_name = self.backend.display_name();
        let home = self.home.clone();
        Controller::new(&login)
            .get("index", {
                let views = views.clone();
                let display_name = display_name.clone();
                move |ctx: &Ctx| {
                    let model = login_model(&display_name, Some(ctx), None);
                    render(&views, "Login", "Index", &model)
                }
            })
            .post("login", {
                let backend = backend.clone();
                let views = views.clone();
                let display_name = display_name.clone();
                let home = home.clone();
                move |ctx: &Ctx| do_login(&*backend, &views, &display_name, &home, ctx)
            })
            .mount(router);
    }

    /// 组装设置页模型（控制器与测试共用）。
    pub fn page_model(&self, saved: bool) -> Value {
        let display_name = self.backend.display_name();
        config_model(&*self.backend, &display_name, None, saved)
    }
}

/// 设置页模型：数据源设置 + 模板必备键（标题/用户/多语言等）。
fn config_model(
    backend: &dyn AdminBackend,
    display_name: &str,
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
fn login_model(display_name: &str, ctx: Option<&Ctx>, error: Option<&str>) -> Value {
    let return_url = ctx
        .and_then(|c| c.query_value("returnUrl"))
        .map(|v| v.to_string())
        .unwrap_or_default();
    json!({
        "title": format!("登录 - {display_name}"),
        "displayName": display_name,
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
fn render(views: &Views, controller: &str, view: &str, model: &Value) -> ActionResult {
    match views.render("Admin", controller, view, model) {
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
    });    match views.render_shared("Prompt", &model) {
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
    display_name: &str,
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
    // 与 C# 一致：保存后回到设置页并提示成功（这里直接渲染提示页 + 自动回跳）
    let _ = display_name;
    prompt(views, "保存成功", true, Some("/admin/config/index"))
}

/// 登录。
fn do_login(
    backend: &dyn AdminBackend,
    views: &Views,
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
            let model = login_model(display_name, Some(ctx), Some(&e));
            render(views, "Login", "Index", &model)
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
    use crate::views::materialize;
    use dhrust::net::http::HttpRequest;
    use std::path::PathBuf;

    fn temp_views(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dsmallui-admin-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        materialize(&dir).unwrap();
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

    #[test]
    fn config_page_renders_with_defaults() {
        let views = Views::new(temp_views("page"));
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
        let views = Views::new(temp_views("empty"));
        let backend = MemoryBackend::new();
        let ctx = ctx("POST", "/admin/config/updatebase", "", "SiteName=&SeoTitle=x");
        let (status, body) = body_of(update_base(&backend, &views, "后台", &ctx));
        assert_eq!(status, 200);
        let text = crate::decode_html_entities(&body);
        assert!(text.contains("网站名称不可为空"), "{text}");
    }

    #[test]
    fn update_base_ajax_returns_json() {
        let views = Views::new(temp_views("ajax"));
        let backend = MemoryBackend::new();
        let mut ctx = ctx(
            "POST",
            "/admin/config/updatebase",
            "",
            "SiteName=%E5%95%86%E5%9F%8E",
        );
        ctx.req.headers.push((
            "x-requested-with".to_string(),
            "XMLHttpRequest".to_string(),
        ));
        let (status, body) = body_of(update_base(&backend, &views, "后台", &ctx));
        assert_eq!(status, 200);
        assert!(body.contains("\"code\":0"), "{body}");
        assert_eq!(backend.settings()["siteName"], "商城");
    }

    #[test]
    fn login_success_redirects_and_failure_renders() {
        let views = Views::new(temp_views("login"));
        let backend = MemoryBackend::new();

        let bad = ctx("POST", "/admin/login/login", "", "UserName=admin&Password=wrong");
        let (status, body) = body_of(do_login(&backend, &views, "后台", DEFAULT_ADMIN_ROUTE, &bad));
        assert_eq!(status, 200);
        let text = crate::decode_html_entities(&body);
        assert!(text.contains("用户名或密码不正确"), "{text}");

        let ok = ctx(
            "POST",
            "/admin/login/login",
            "",
            "UserName=admin&Password=admin123&returnUrl=%2Fadmin%2Fconfig%2Findex",
        );
        let (status, body) = body_of(do_login(&backend, &views, "后台", DEFAULT_ADMIN_ROUTE, &ok));
        assert_eq!(status, 302, "{body}");
    }

    #[test]
    fn safe_redirect_blocks_open_redirect() {
        assert!(safe_redirect("//evil.com/x").is_none());
        assert!(safe_redirect("https://evil.com").is_none());
        assert_eq!(
            safe_redirect("/admin/config/index").as_deref(),
            Some("/admin/config/index")
        );
    }
}
