//! 最小可运行示例：一个端口上同时提供 DsMallUI 的后台页面、UEditor 接口与静态资源。
//!
//! 运行：
//!
//! ```text
//! cargo run --example admin_server            # 默认 127.0.0.1:8080
//! cargo run --example admin_server -- 8099    # 指定端口
//! ```
//!
//! 可验证的入口：
//! - `GET  /admin/config/index`        站点设置页（Razor 子集渲染）
//! - `POST /admin/config/updatebase`   保存设置（默认账号登录后可用）
//! - `GET  /admin/login/index`         登录页
//! - `POST /admin/login/login`         登录（`UserName=admin&Password=admin123`）
//! - `GET  /fileuploader?action=config` UEditor 配置
//! - `POST /fileuploader?action=uploadimage` UEditor 图片上传（multipart，字段 `upfile`）

use std::sync::Arc;

use dhrust::net::http::{HttpOutcome, HttpResponse, HttpServer};
use dhrust::net::router::{route, Router};
use pek_dsmallui::{DsMallUi, MemoryBackend};

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    // 示例数据源：admin / admin123（生产请换成自己的 AdminBackend 实现）
    let backend = Arc::new(MemoryBackend::new());

    let ui = match DsMallUi::builder()
        .web_root("wwwroot")
        .views_root("Views")
        .upload_path("upload")
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

    println!("Pek.DsMallUI 示例已启动（Ctrl+C 退出）：");
    println!("  后台设置页   http://{addr}/admin/config/index");
    println!("  登录页       http://{addr}/admin/login/index   （admin / admin123）");
    println!("  UEditor 配置 http://{addr}/fileuploader?action=config");
    println!("  上传根目录   {}", ui.upload_root().display());

    if let Err(e) = server.serve(router.into_handler()).await {
        eprintln!("服务退出：{e}");
    }
}
