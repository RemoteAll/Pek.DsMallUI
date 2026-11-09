//! 上传控制器（对应 C# `Pek.DsMallUI/Controllers/UploadController.cs`）。
//!
//! | 路由 | 说明 |
//! |---|---|
//! | `GET/POST /fileuploader?action=...` | UEditor 统一入口（C# 的绝对路由，前端配置的 `serverUrl`） |
//! | `GET/POST /Upload/fileuploader` | 同上，MVC 风格别名（`{controller}/{action}`） |
//! | `GET /Upload/downloads/{path}` | 下载上传目录内的文件（对应 C# `[Route("downloads")]`） |
//!
//! 下载为什么走**中间件**：`dhrust::net::router` 的路径占位只支持单段 `{name}`
//! （不支持 `{**path}`），而上传目录天然是多层（`images/2026/11/09/x.png`），
//! 故用前缀中间件拦截 `/Upload/downloads/` 再做静态文件解析（复用 `StaticFiles` 的
//! 路径穿越防护 + ETag/304）。

use std::path::PathBuf;

use dhrust::net::controller::Controller;
use dhrust::net::http::{HttpOutcome, HttpResponse};
use dhrust::net::router::{middleware, route, Ctx, Next, RouteHandler, Router};
use dhrust::net::static_files::StaticFiles;

use crate::ueditor::UeditorService;

/// 下载路由前缀（大小写不敏感匹配）。
pub const DOWNLOAD_PREFIX: &str = "/upload/downloads/";

/// 上传控制器。
#[derive(Clone)]
pub struct UploadController {
    service: UeditorService,
    downloads: StaticFiles,
    download_prefix: String,
}

impl UploadController {
    /// `upload_root` 同时作为下载根与静态根（与 C# 的 `WebRootPath + UploadPath` 一致）。
    pub fn new(service: UeditorService, upload_root: impl Into<PathBuf>) -> UploadController {
        UploadController {
            service,
            downloads: StaticFiles::new(upload_root),
            download_prefix: DOWNLOAD_PREFIX.to_string(),
        }
    }

    /// UEditor 服务引用（供上层读取配置）。
    pub fn service(&self) -> &UeditorService {
        &self.service
    }

    /// 挂载全部路由与下载中间件。
    pub fn mount(&self, router: &mut Router) {
        // ① C# 绝对路由：/fileuploader（UEditor 前端 serverUrl 的默认形态）
        router.map("*", "/fileuploader", self.ueditor_handler());

        // ② MVC 风格别名：/Upload/fileuploader
        Controller::new("Upload")
            .map("*", "fileuploader", {
                let service = self.service.clone();
                move |ctx: &Ctx| ueditor_result(&service, ctx)
            })
            .mount(router);

        // ③ 下载：/Upload/downloads/{path}（多层路径，前缀中间件）
        router.use_middleware(download_middleware(
            self.downloads.clone(),
            self.download_prefix.clone(),
        ));
    }

    fn ueditor_handler(&self) -> RouteHandler {
        let service = self.service.clone();
        route(move |ctx| {
            let service = service.clone();
            async move { ueditor_outcome(&service, &ctx) }
        })
    }
}

/// 统一入口：执行一次 UEditor 请求并返回 JSON。
fn ueditor_result(service: &UeditorService, ctx: &Ctx) -> dhrust::net::controller::ActionResult {
    match ueditor_outcome(service, ctx) {
        HttpOutcome::Response(response) => {
            dhrust::net::controller::ActionResult::Response(response)
        }
        // UEditor 入口不会升级 WebSocket；真出现即明确报错而不是静默
        _ => dhrust::net::controller::ActionResult::Response(HttpResponse::text(
            500,
            "UEditor 入口不支持 WebSocket 升级",
        )),
    }
}

/// 供 `RouteHandler`（async）复用的实现。
fn ueditor_outcome(service: &UeditorService, ctx: &Ctx) -> HttpOutcome {
    // 未带 action 时按 UEditor 惯例回配置（前端初始化会先取一次配置）
    let action = ctx.query_value("action").unwrap_or("config");
    let json = service.handle(
        action,
        &ctx.query,
        ctx.header("content-type"),
        &ctx.req.body,
    );
    HttpOutcome::Response(HttpResponse::bytes(
        200,
        "application/json; charset=utf-8",
        json.into_bytes(),
    ))
}

/// 下载中间件：命中前缀则直接返回文件（含条件请求），否则交还后续链路。
fn download_middleware(statics: StaticFiles, prefix: String) -> dhrust::net::router::Middleware {
    let prefix = prefix.to_ascii_lowercase();
    middleware(move |ctx: Ctx, next: Next| {
        let statics = statics.clone();
        let prefix = prefix.clone();
        async move {
            let path_lower = ctx.req.path.to_ascii_lowercase();
            if !path_lower.starts_with(&prefix) {
                return next(ctx).await;
            }

            let method = ctx.req.method.to_ascii_uppercase();
            if method != "GET" && method != "HEAD" {
                return HttpOutcome::Response(HttpResponse::text(405, "Method Not Allowed"));
            }

            // 相对路径：/Upload/downloads/images/x.png → /images/x.png
            let rest = &ctx.req.path[prefix.len()..];
            if rest.trim().is_empty() {
                return HttpOutcome::Response(HttpResponse::text(404, "Not Found"));
            }

            let mut req = ctx.req.clone();
            req.path = format!("/{}", rest.trim_start_matches('/'));

            match statics.try_serve_request(&req) {
                Some(response) => {
                    let file_name = req.path.rsplit('/').next().unwrap_or("download");
                    let response = response.with_header(
                        "Content-Disposition",
                        &content_disposition(file_name),
                    );
                    HttpOutcome::Response(response)
                }
                None => HttpOutcome::Response(HttpResponse::text(404, "Not Found")),
            }
        }
    })
}

/// 构造 `Content-Disposition`（ASCII 名 + RFC 5987 UTF-8 名，中文文件名不丢）。
fn content_disposition(file_name: &str) -> String {
    let ascii: String = file_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "attachment; filename=\"{ascii}\"; filename*=UTF-8''{}",
        percent_encode(file_name)
    )
}

/// RFC 5987 百分号编码（仅保留 unreserved 字符）。
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.as_bytes() {
        let keep = byte.is_ascii_alphanumeric()
            || matches!(*byte, b'-' | b'.' | b'_' | b'~');
        if keep {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dhrust::net::http::HttpRequest;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dsmallui-ctrl-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn content_disposition_keeps_utf8() {
        let header = content_disposition("报表 2026.pdf");
        // 非 ASCII 逐字符替换为 `_`，空格保留
        assert!(header.contains("filename=\"__ 2026.pdf\""), "{header}");
        assert!(header.contains("filename*=UTF-8''"), "{header}");
        assert!(header.contains("%E6%8A%A5"), "{header}");
    }

    #[test]
    fn ueditor_route_serves_config_json() {
        let root = temp_root("cfg");
        let control = UploadController::new(UeditorService::new(&root, "/upload"), &root);
        let ctx = Ctx::build(HttpRequest {
            method: "GET".into(),
            path: "/fileuploader".into(),
            query: "action=config".into(),
            headers: Vec::new(),
            body: Vec::new().into(),
            remote_addr: None,
        });
        let outcome = ueditor_outcome(control.service(), &ctx);
        match outcome {
            HttpOutcome::Response(response) => {
                let text = String::from_utf8_lossy(&response.body);
                assert!(text.contains("imageActionName"), "{text}");
            }
            _ => panic!("应直接返回响应"),
        }
    }

    #[test]
    fn download_serves_existing_file() {
        let root = temp_root("dl");
        let dir = root.join("images");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();

        let control = UploadController::new(UeditorService::new(&root, "/upload"), &root);
        let mut router = Router::new();
        control.mount(&mut router);

        // 直接调用分发器验证中间件路径（不启 HTTP 服务）
        let statics = StaticFiles::new(&root);
        let mut req = HttpRequest {
            method: "GET".into(),
            path: "/images/a.txt".into(),
            query: String::new(),
            headers: Vec::new(),
            body: Vec::new().into(),
            remote_addr: None,
        };
        req.path = "/images/a.txt".into();
        let response = statics.try_serve_request(&req).expect("应命中文件");
        assert_eq!(response.status, 200);
        assert_eq!(response.body.as_ref(), b"hello");
    }
}
