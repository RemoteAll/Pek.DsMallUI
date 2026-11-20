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
use dhrust::net::http::{HttpOutcome, HttpResponse};
use dhrust::net::router::{route, Ctx, Router};
use serde_json::{json, Map, Value};

use crate::area;
use crate::backend::{
    default_settings, value_as_bool, value_as_i64, AdminBackend, AdminUser, MemoryBackend,
};
use crate::captcha;
use crate::menu::{self, AdminMenu};
use crate::views::Views;

/// 默认后台区域名（与 C# 默认 `Admin` 一致）。
pub const DEFAULT_ADMIN_AREA: &str = area::DEFAULT_AREA;

/// 指定区域的默认首页路由：`Biz` → `/biz/home/index`。
pub fn home_route(area_name: &str) -> String {
    format!("{}/home/index", area::route_prefix(area_name))
}

/// 登录成功钩子：返回要追加到响应上的头（如 `Set-Cookie` 会话）。
pub type LoginHook = Arc<dyn Fn(&AdminUser) -> Vec<(String, String)> + Send + Sync>;

/// 后台 Area。
#[derive(Clone)]
pub struct AdminArea {
    backend: Arc<dyn AdminBackend>,
    views: Views,
    area: String,
    home: String,
    upload_prefix: String,
    /// 登录页是否启用图形验证码（默认开启，对齐 C# `CaptChaUrl`）。
    captcha_enabled: bool,
    /// 登录成功后的扩展头（由消费方注入会话 Cookie 等）。
    login_hook: Option<LoginHook>,
    /// 左侧菜单（消费方可覆盖，默认只有本 crate 自己的页面）。
    menus: Vec<AdminMenu>,
    /// 退出登录地址（默认 `/{area}/logout`，由宿主提供路由）。
    logout_url: String,
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
            captcha_enabled: true,
            login_hook: None,
            menus: Vec::new(),
            logout_url: format!("{}/logout", area::route_prefix(area::DEFAULT_AREA)),
        }
    }

    /// 设置登录成功钩子（返回追加到响应的头，如 `Set-Cookie`）。
    pub fn with_login_hook(mut self, hook: LoginHook) -> AdminArea {
        self.login_hook = Some(hook);
        self
    }

    /// 开启/关闭登录验证码（默认开启；单元测试与脚本化登录可关闭）。
    pub fn with_captcha(mut self, enabled: bool) -> AdminArea {
        self.captcha_enabled = enabled;
        self
    }

    /// 登录验证码是否启用。
    pub fn captcha_enabled(&self) -> bool {
        self.captcha_enabled
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

    /// 设置左侧菜单（消费方追加自己的功能页；不设则用 [`menu::default_menus`]）。
    ///
    /// `url` 里可写 `{area}` 占位符，会按实际区域路由前缀替换。
    pub fn with_menus(mut self, menus: Vec<AdminMenu>) -> AdminArea {
        self.menus = menus;
        self
    }

    /// 设置退出登录地址（默认 `/{area}/logout`，宿主需提供该路由）。
    pub fn with_logout_url(mut self, url: impl Into<String>) -> AdminArea {
        self.logout_url = url.into();
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
        // 菜单原样传递给模型（`{area}` 占位符由模型按实际区域替换）
        let menus = self.menus.clone();
        let logout_url = self.logout_url.clone();

        // 登录图形验证码：`GET /captcha`（仅启用时挂载）
        if self.captcha_enabled {
            router.map_get(
                "/captcha",
                route(|_ctx: Ctx| async move {
                    HttpOutcome::Response(captcha::response())
                }),
            );
        }

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
                let menus = menus.clone();
                let logout_url = logout_url.clone();
                move |ctx: &Ctx| {
                    let model = home_model(
                        &*backend,
                        &display_name,
                        &area_name,
                        &upload_prefix,
                        &menus,
                        &logout_url,
                        Some(ctx),
                    );
                    render(&views, &area_name, "Home", "Index", &model)
                }
            })
            .get("main", {
                // 与 `/home/index` 用**同一个视图**：C# 里这是 iframe 的工作区，
                // 本移植版不自己造框架页，两者内容一致（`mainUrl` 仍可用）。
                let backend = backend.clone();
                let views = views.clone();
                let area_name = area_name.clone();
                let display_name = display_name.clone();
                let upload_prefix = upload_prefix.clone();
                let menus = menus.clone();
                let logout_url = logout_url.clone();
                move |ctx: &Ctx| {
                    let model = home_model(
                        &*backend,
                        &display_name,
                        &area_name,
                        &upload_prefix,
                        &menus,
                        &logout_url,
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
                let menus = menus.clone();
                let logout_url = logout_url.clone();
                move |ctx: &Ctx| {
                    let model = config_model(
                        &*backend,
                        &display_name,
                        &area_name,
                        &menus,
                        &logout_url,
                        Some(ctx),
                        false,
                    );
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
        let captcha_enabled = self.captcha_enabled;
        let login_hook = self.login_hook.clone();
        Controller::new(&format!("{segment}/login"))
            .get("index", {
                let views = views.clone();
                let area_name = area_name.clone();
                let display_name = display_name.clone();
                let home = home.clone();
                move |ctx: &Ctx| {
                    let model = login_model(
                        &display_name,
                        &area_name,
                        &home,
                        Some(ctx),
                        None,
                        captcha_enabled,
                    );
                    render(&views, &area_name, "Login", "Index", &model)
                }
            })
            .post("login", {
                let backend = backend.clone();
                let views = views.clone();
                let area_name = area_name.clone();
                let display_name = display_name.clone();
                let home = home.clone();
                let login_hook = login_hook.clone();
                move |ctx: &Ctx| {
                    do_login(
                        &*backend,
                        &views,
                        &area_name,
                        &display_name,
                        &home,
                        captcha_enabled,
                        login_hook.as_ref(),
                        ctx,
                    )
                }
            })
            .mount(router);
    }

    /// 组装设置页模型（控制器与测试共用）。
    pub fn page_model(&self, saved: bool) -> Value {
        let display_name = self.backend.display_name();
        config_model(
            &*self.backend,
            &display_name,
            &self.area,
            &self.menus,
            &self.logout_url,
            None,
            saved,
        )
    }
}

/// 后台首页模型（控制台卡片 + 菜单）。
#[allow(clippy::too_many_arguments)]
fn home_model(
    backend: &dyn AdminBackend,
    display_name: &str,
    area_name: &str,
    upload_prefix: &str,
    configured_menus: &[AdminMenu],
    logout_url: &str,
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
    let (left_menus, has_left_menus) = left_menus(configured_menus, area_name);
    let mut shortcuts = Vec::new();
    for menu in resolved_menus(configured_menus, area_name) {
        if menu.has_children() {
            for child in &menu.children {
                shortcuts.push(json!({ "name": child.title, "url": child.url, "desc": menu.title }));
            }
        } else {
            shortcuts.push(json!({ "name": menu.title, "url": menu.url, "desc": "" }));
        }
    }
    let has_menus = !shortcuts.is_empty();

    json!({
        "title": format!("管理后台 - {display_name}"),
        "displayName": display_name,
        "userName": current_user(ctx),
        "siteName": site_name,
        "areaName": area_name,
        "areaRoute": route,
        "uploadPrefix": upload_prefix,
        "version": crate::version(),
        "menus": shortcuts,
        "hasMenus": has_menus,
        "leftMenus": left_menus,
        "hasLeftMenus": has_left_menus,
        "currentPath": current_path(ctx),
        "home": home_route(area_name),
        "mainUrl": format!("{route}/home/main"),
        "logoutUrl": resolve_url(logout_url, area_name),
        "copyright": format!("@ {display_name}"),
    })
}

/// 设置页模型：数据源设置 + 模板必备键（标题/用户/多语言/区域等）。
fn config_model(
    backend: &dyn AdminBackend,
    display_name: &str,
    area_name: &str,
    configured_menus: &[AdminMenu],
    logout_url: &str,
    ctx: Option<&Ctx>,
    _saved: bool,
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
    let (left_menus, has_left_menus) = left_menus(configured_menus, area_name);
    base.insert("title".into(), json!(format!("站点设置 - {display_name}")));
    base.insert("displayName".into(), json!(display_name));
    base.insert("userName".into(), json!(current_user(ctx)));
    base.insert("saved".into(), json!(_saved));
    base.insert("areaName".into(), json!(area_name));
    base.insert("areaRoute".into(), json!(area::route_prefix(area_name)));
    base.insert("currentAction".into(), json!("Index"));
    base.insert("leftMenus".into(), left_menus);
    base.insert("hasLeftMenus".into(), json!(has_left_menus));
    base.insert("currentPath".into(), json!(current_path(ctx)));
    base.insert("home".into(), json!(home_route(area_name)));
    base.insert("logoutUrl".into(), json!(resolve_url(logout_url, area_name)));
    base.insert("copyright".into(), json!(format!("@ {display_name}")));

    // 模板里的单选/开关直接读 bool，整数读 Int32——后端可能存的是字符串，这里统一归一化。
    for key in [
        "allowLogin",
        "autoRegister",
        "useSsoRole",
        "logoutAll",
        "isEnableLanguage",
        "seoFriendlyUrlsForLanguagesEnabled",
        "isAllowUrlSuffix",
        "isHtmlStaticDevelopmentMode",
    ] {
        let value = base.get(key).cloned().unwrap_or(json!(false));
        base.insert(key.to_string(), json!(value_as_bool(&value)));
    }
    let seo_adjustment = base
        .get("pageTitleSeoAdjustment")
        .cloned()
        .unwrap_or(json!(1));
    base.insert(
        "pageTitleSeoAdjustment".into(),
        json!(value_as_i64(&seo_adjustment)),
    );
    // 输入框统一按字符串渲染（数字/布尔混存也不会报错）
    for key in [
        "sessionTimeout",
        "refreshUserPeriod",
        "htmlStaticExpireMinutes",
    ] {
        let value = base.get(key).cloned().unwrap_or(json!(""));
        let text = match value {
            Value::String(s) => s,
            Value::Null => String::new(),
            other => other.to_string(),
        };
        base.insert(key.to_string(), json!(text));
    }

    // 多语言：补齐每个语言项的全部字段（缺键会让模板直接渲染失败）。
    let languages = base
        .get("languages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let languages: Vec<Value> = languages
        .into_iter()
        .map(|item| {
            let mut map = match item {
                Value::Object(map) => map,
                _ => Map::new(),
            };
            if !map.contains_key("id") {
                map.insert("id".into(), json!(0));
            }
            for key in [
                "name",
                "siteName",
                "seoTitle",
                "seoKey",
                "seoDescribe",
                "registration",
                "copyright",
            ] {
                if !map.contains_key(key) {
                    map.insert(key.to_string(), json!(""));
                }
            }
            Value::Object(map)
        })
        .collect();
    let has_languages = !languages.is_empty();
    base.insert("languages".into(), Value::Array(languages));
    base.insert("hasLanguages".into(), json!(has_languages));
    Value::Object(base)
}

/// 登录页模型。
fn login_model(
    display_name: &str,
    area_name: &str,
    home: &str,
    ctx: Option<&Ctx>,
    error: Option<&str>,
    captcha_enabled: bool,
) -> Value {
    let return_url = ctx
        .and_then(|c| c.query_value("returnUrl"))
        .map(|v| v.to_string())
        .unwrap_or_default();
    // 非 Ajax 登录失败时，上报错误经 `?error=` 带回登录页。
    let error = error
        .map(|value| value.to_string())
        .or_else(|| {
            ctx.and_then(|c| c.query_value("error"))
                .map(|value| value.to_string())
        })
        .unwrap_or_default();
    // 背景图 bg_1..bg_8（C# `Rand.Next(1, 9)` 同口径）
    let bg_rnd = (dhrust::random::bytes(1)[0] % 8) as i32 + 1;
    json!({
        "title": format!("登录 - {display_name}"),
        "displayName": display_name,
        "areaName": area_name,
        "areaRoute": area::route_prefix(area_name),
        "userName": current_user(ctx),
        "returnUrl": return_url,
        "hasError": !error.is_empty(),
        "error": error,
        "bgRnd": bg_rnd,
        "captchaEnabled": captcha_enabled,
        "captchaUrl": "/captcha",
        "home": home,
    })
}

/// 当前登录用户（由宿主鉴权中间件写入 `Ctx::state` 的 `user` 键）。
fn current_user(ctx: Option<&Ctx>) -> String {
    ctx.and_then(|c| c.state_value("user"))
        .unwrap_or("")
        .to_string()
}

/// 当前请求路径（菜单高亮用）。
fn current_path(ctx: Option<&Ctx>) -> String {
    ctx.map(|c| c.req.path.clone()).unwrap_or_default()
}

/// 把 `{area}` 占位符换成实际路由前缀。
fn resolve_url(url: &str, area_name: &str) -> String {
    url.replace("{area}", &area::route_prefix(area_name))
}

/// 解析菜单：未配置时用库默认，再把 `{area}` 占位符换成实际路由前缀。
fn resolved_menus(configured: &[AdminMenu], area_name: &str) -> Vec<AdminMenu> {
    let base = if configured.is_empty() {
        menu::default_menus()
    } else {
        configured.to_vec()
    };
    base.into_iter()
        .map(|item| item.with_area_prefix(area_name))
        .collect()
}

/// 左侧菜单 → 模板 JSON（`(数组, 是否非空)`）。
fn left_menus(configured: &[AdminMenu], area_name: &str) -> (Value, bool) {
    let menus = resolved_menus(configured, area_name);
    let has = !menus.is_empty();
    (AdminMenu::list_to_value(&menus), has)
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
    let back_url = back_url.unwrap_or("javascript:history.back(-1);");
    let model = json!({
        "title": "跳转提示",
        "message": message,
        "isOk": is_ok,
        "hasBackUrl": true,
        "backUrl": back_url,
        "autoBack": true,
        "countdownTime": 3,
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
    let site_name = ctx.form_value_ci("SiteName").unwrap_or("").trim().to_string();
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
    captcha_enabled: bool,
    login_hook: Option<&LoginHook>,
    ctx: &Ctx,
) -> ActionResult {
    let user_name = form_value_any(ctx, &["UserName", "username"])
        .trim()
        .to_string();
    let password = form_value_any(ctx, &["Password", "password"]);
    let return_url = form_value_any(ctx, &["returnUrl", "returnurl"])
        .trim()
        .to_string();

    // 验证码（与 C# 登录页同一位置）：先校验，避免无效登录打穿口令校验。
    if captcha_enabled {
        let token = captcha::token_from_cookie(ctx.header("cookie"));
        let code = form_value_any(ctx, &["checkcode", "CheckCode", "captcha"]);
        if !captcha::check(&token, &code) {
            let message = "验证码不正确";
            dhrust::logs::warn!("[DsMallUI] 登录失败（{user_name}）：{message}");
            return login_failure(views, area_name, display_name, home, captcha_enabled, ctx, message);
        }
    }

    match backend.login(&user_name, &password) {
        Ok(user) => {
            // 消费方注入的扩展头（如会话 Cookie）。
            let extra_headers = login_hook.map(|hook| hook(&user)).unwrap_or_default();
            if is_ajax(ctx) {
                let body = json!({
                    "code": 0,
                    "message": "",
                    "data": {
                        "id": user.id,
                        "userName": user.user_name,
                        "displayName": user.display_name,
                        "isAdmin": user.is_admin,
                        "home": home,
                        "locate": home,
                        "success": true,
                    }
                })
                .to_string();
                let mut response = HttpResponse::json(200, body);
                for (name, value) in extra_headers {
                    response = response.with_header(&name, &value);
                }
                return ActionResult::Response(response);
            }
            let target = safe_redirect(&return_url).unwrap_or_else(|| home.to_string());
            let mut response = HttpResponse::empty(302).with_header("Location", &target);
            for (name, value) in extra_headers {
                response = response.with_header(&name, &value);
            }
            ActionResult::Response(response)
        }
        Err(e) => {
            dhrust::logs::warn!("[DsMallUI] 登录失败（{user_name}）：{e}");
            login_failure(views, area_name, display_name, home, captcha_enabled, ctx, &e)
        }
    }
}

/// 登录失败：Ajax 回 JSON，否则重渲染登录页并带上错误提示。
fn login_failure(
    views: &Views,
    area_name: &str,
    display_name: &str,
    home: &str,
    captcha_enabled: bool,
    ctx: &Ctx,
    message: &str,
) -> ActionResult {
    if is_ajax(ctx) {
        return json_result(401, message, None);
    }
    let model = login_model(
        display_name,
        area_name,
        home,
        Some(ctx),
        Some(message),
        captcha_enabled,
    );
    render(views, area_name, "Login", "Index", &model)
}

/// 取第一个命中的表单值（C# 视图字段大小写不一，这里兼容历史前端）。
fn form_value_any<'a>(ctx: &'a Ctx, names: &[&str]) -> String {
    for name in names {
        if let Some(value) = ctx.form_value_ci(name) {
            return value.to_string();
        }
    }
    String::new()
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
        let (status, body) = body_of(do_login(
            &backend, &views, "Biz", "后台", &home, false, None, &bad,
        ));
        assert_eq!(status, 200);
        let text = crate::decode_html_entities(&body);
        assert!(text.contains("用户名或密码不正确"), "{text}");

        let ok = ctx(
            "POST",
            "/biz/login/login",
            "",
            "UserName=admin&Password=admin123",
        );
        let (status, body) = body_of(do_login(
            &backend, &views, "Biz", "后台", &home, false, None, &ok,
        ));
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

    #[test]
    fn login_requires_captcha_when_enabled() {
        let views = Views::new(temp_views("captcha", "Biz"));
        let backend = MemoryBackend::new();
        let home = home_route("Biz");
        let request = ctx(
            "POST",
            "/biz/login/login",
            "",
            "UserName=admin&Password=admin123&checkcode=0000",
        );
        let (status, body) = body_of(do_login(
            &backend, &views, "Biz", "后台", &home, true, None, &request,
        ));
        assert_eq!(status, 200);
        let text = crate::decode_html_entities(&body);
        assert!(text.contains("验证码不正确"), "{text}");
    }

    #[test]
    fn login_hook_adds_response_headers() {
        let views = Views::new(temp_views("hook", "Biz"));
        let backend = MemoryBackend::new();
        let home = home_route("Biz");
        let ok = ctx(
            "POST",
            "/biz/login/login",
            "",
            "UserName=admin&Password=admin123",
        );
        let hook: LoginHook = Arc::new(|user| {
            vec![(
                "Set-Cookie".to_string(),
                format!("sid={}", user.user_name),
            )]
        });
        match do_login(
            &backend,
            &views,
            "Biz",
            "后台",
            &home,
            false,
            Some(&hook),
            &ok,
        ) {
            ActionResult::Response(response) => {
                assert_eq!(response.status, 302);
                assert!(response
                    .headers
                    .iter()
                    .any(|(k, v)| k.eq_ignore_ascii_case("set-cookie") && v == "sid=admin"));
            }
            ActionResult::View { .. } => panic!("登录成功应返回响应"),
        }
    }
}
