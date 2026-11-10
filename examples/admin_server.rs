//! 最小可运行示例：一个端口上同时提供 DsMallUI 的后台页面、UEditor 接口与静态资源。
//!
//! 运行：
//!
//! ```text
//! cargo run --example admin_server                 # 默认 127.0.0.1:8080，区域 Biz
//! cargo run --example admin_server -- 8099         # 指定端口
//! cargo run --example admin_server -- 8099 Admin   # 指定端口 + 区域名（默认 Admin 也可）
//! DSMALLUI_ADMIN_AREA=Admin cargo run --example admin_server   # 用环境变量指定区域
//! ```
//!
//! 区域名对应 C# 的 `DHSetting.Current.AdminArea`：默认 `Admin`，本项目用 `Biz`，
//! 路由与视图目录随之变化（`/biz/...`、`Views/Biz/...`）。
//!
//! 可验证的入口（区域为 `Biz` 时）：
//! - `GET  /biz/home/index`             后台首页（控制台 + 菜单）
//! - `GET  /biz/config/index`           站点设置页（Razor 子集渲染）
//! - `POST /biz/config/updatebase`      保存设置
//! - `GET  /biz/login/index`            登录页
//! - `POST /biz/login/login`            登录（`UserName=admin&Password=admin123`）
//! - `GET  /fileuploader?action=config` UEditor 配置
//! - `POST /fileuploader?action=uploadimage` UEditor 图片上传（multipart，字段 `upfile`）

use std::sync::Arc;

use dhrust::net::http::{HttpOutcome, HttpResponse, HttpServer};
use dhrust::net::router::{route, Router};
use pek_dsmallui::{DsMallUi, MemoryBackend};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().and_then(|p| p.parse().ok()).unwrap_or(8080);
    // 区域名：命令行 > 环境变量 > 默认 Biz（示例代表一个消费方项目）
    let area_name = args
        .next()
        .or_else(|| std::env::var("DSMALLUI_ADMIN_AREA").ok())
        .unwrap_or_else(|| "Biz".to_string());

    // 示例数据源：admin / admin123（生产请换成自己的 AdminBackend 实现）
    let backend = Arc::new(MemoryBackend::new());

    let ui = match DsMallUi::builder()
        .web_root("wwwroot")
        .views_root("Views")
        .upload_path("upload")
        .admin_area(&area_name)
        .backend(backend)
        .build()
    {
        Ok(ui) => ui,
        Err(e) => {
            eprintln!("初始化 DsMallUI 失败：{e}");
            std::process::exit(1);
        }
    };

    if let Err(e) = ui.materialize_defaults() {
        eprintln!("落地默认视图/配置失败：{e}");
        std::process::exit(1);
    }

    let mut router = Router::new();
    ui.mount(&mut router);

    // 静态资源：wwwroot（/static/**、/ueconfig.json、/upload/** 等）
    let statics = ui.static_files();
    router.fallback(route(move |ctx| {
        let statics = statics.clone();
        async move {
            let method_ok = ctx.req.method.eq_ignore_ascii_case("GET")
                || ctx.req.method.eq_ignore_ascii_case("HEAD");
            let served = if method_ok {
                statics.try_serve_request(&ctx.req)
            } else {
                statics.try_serve_file(&ctx.req.path)
            };
            match served {
                Some(response) => HttpOutcome::Response(response),
                None => HttpOutcome::Response(HttpResponse::json(
                    404,
                    dhrust::net::http::DGResult::fail(format!("未找到：{}", ctx.req.path))
                        .to_json(),
                )),
            }
        }
    }));

    let addr = format!("127.0.0.1:{port}");
    let server = match HttpServer::bind(addr.as_str()).await {
        Ok(server) => server,
        Err(e) => {
            eprintln!("绑定 {addr} 失败：{e}");
            std::process::exit(1);
        }
    };

    let route_prefix = ui.admin().route_prefix();
    println!("Pek.DsMallUI 示例已启动（Ctrl+C 退出）：");
    println!(
        "  后台区域     {}（路由前缀 {route_prefix}）",
        ui.admin_area()
    );
    println!("  后台首页     http://{addr}{route_prefix}/home/index");
    println!("  后台设置页   http://{addr}{route_prefix}/config/index");
    println!("  登录页       http://{addr}{route_prefix}/login/index   （admin / admin123）");
    println!("  UEditor 配置 http://{addr}/fileuploader?action=config");
    println!("  上传根目录   {}", ui.upload_root().display());

    if let Err(e) = server.serve(router.into_handler()).await {
        eprintln!("服务退出：{e}");
    }
}
