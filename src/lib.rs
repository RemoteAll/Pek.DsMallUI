//! # Pek DsMallUI（Rust）
//!
//! 「猿人易 NCube / DsMallUI」后台 UI 能力的 **Rust 统一封装**，与 C# 参考工程
//! `G:\Code\Pek.FrameWork\Pek.DsMallUI` 一一对应，复用 [DH.RustBase](https://github.com/RemoteAll/DH.RustBase)
//! （crate `dhrust`）的 **C# 风格 Web 规范**：`Controller` + `Views`/Razor 子集 + `StaticFiles`。
//!
//! ## 提供什么
//!
//! | 能力 | 入口 | C# 对应 |
//! |---|---|---|
//! | 门面/装配 | [`DsMallUi::builder`] | `PekStartup`（`IPekStartup`） |
//! | 上传/下载 | [`controllers::upload`] | `Controllers/UploadController.cs` |
//! | UEditor 后端 | [`ueditor::UeditorService`] | `UEditor/*.cs`（Handler/Upload/Crawler/ListFile/PathFormatter） |
//! | 多段表单解析 | [`multipart::FormData`] | `Request.ReadFormAsync()` |
//! | 后台 Area | [`controllers::admin`] | `Areas/Admin/**`（Config/Login） |
//! | 视图约定 | [`views::Views`] | `Areas/Admin/Views/**`（Razor 子集） |
//! | 折叠实体 | [`models::Hierarchy`] | `Models/Hierarchy.cs` |
//!
//! ## 最小用法
//!
//! ```no_run
//! use dhrust::net::http::{HttpOutcome, HttpResponse, HttpServer};
//! use dhrust::net::router::{route, Router};
//! use pek_dsmallui::DsMallUi;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), String> {
//!     let ui = DsMallUi::builder()
//!         .web_root("wwwroot")
//!         .views_root("Views")
//!         .build()?;
//!     ui.materialize_defaults()?;      // 首次：落地默认视图 + ueconfig.json
//!
//!     let mut router = Router::new();
//!     ui.mount(&mut router);           // /fileuploader、/Upload/*、/admin/*
//!
//!     // 静态资源（wwwroot）挂 fallback；DSMallUI 的路由优先命中
//!     let statics = ui.static_files();
//!     router.fallback(route(move |ctx| {
//!         let statics = statics.clone();
//!         async move {
//!             match statics.try_serve_request(&ctx.req) {
//!                 Some(response) => HttpOutcome::Response(response),
//!                 None => HttpOutcome::Response(HttpResponse::text(404, "Not Found")),
//!             }
//!         }
//!     }));
//!
//!     let server = HttpServer::bind("127.0.0.1:8080").await.map_err(|e| e.to_string())?;
//!     server.serve(router.into_handler()).await.map_err(|e| e.to_string())
//! }
//! ```
//!
//! ## 与 C# 端的差异（都是「补实现」而非改口径）
//!
//! - C# `UploadHandler.Process` / `Crawler.Fetch` / `UeditorConfig.GetStringList` 当前是**注释掉的**
//!   （上传一律返回「未知错误」、扩展名列表恒为空）；Rust 侧按 UEditor 官方协议补齐。
//! - 视图从 C# 版（依赖 `@PekHtml.*` / `@T()` / tag helper）**改写为 Razor 子集**
//!   （`dhrust::razor` 双端可渲染的子集），详见 [`docs/移植对照.md`](../docs/移植对照.md)。
//! - 静态资源（`wwwroot/static`，约 24.6 MB 第三方前端资产）**不入库**，
//!   由 `tools/sync-wwwroot.ps1` 从 C# 参考工程同步到消费方 `wwwroot`。
//!
//! ## 复用检索（写码前的证据）
//!
//! - 全仓 `G:\Code\Rust` 检索 `ueditor|UEditor`：**无命中**（无既有实现可复用）；
//! - 检索 `multipart|form-data|boundary=`：仅 `dhrust::net::router` 的**待办注释**与
//!   HLZX.EIP 三处「刻意不走 multipart」的说明 → 无实现可复用，故本 crate 落地
//!   [`multipart`]，并在模块文档里写明「若出现第二个消费方就下沉进 `dhrust::net`」；
//! - 控制器/视图/静态资源/日志/口令/出网客户端一律用 `dhrust` 既有能力，未另起炉灶。

pub mod backend;
pub mod controllers;
pub mod models;
pub mod multipart;
pub mod options;
pub mod ueditor;
pub mod views;

pub use backend::{default_settings, AdminBackend, AdminUser, MemoryBackend};
pub use controllers::admin::AdminArea;
pub use controllers::upload::UploadController;
pub use models::Hierarchy;
pub use multipart::{FormData, MultipartError, Part};
pub use options::{embedded_ueditor_config, DsMallUi, DsMallUiBuilder, UE_CONFIG_JSON};
pub use ueditor::{UploadConfig, UploadOutcome, UploadState, Uploader, UeditorConfig, UeditorService};
pub use views::Views;

/// crate 版本。
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// base64 解码（涂鸦上传）：先按标准字母表，失败再按 URL 安全字母表；
/// 解码前忽略空白（浏览器可能换行折行），缺少 `=` 填充也能解（UEditor 前端不保证填充）。
pub(crate) fn base64_decode(text: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let cleaned: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD
        .decode(cleaned.as_bytes())
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(cleaned.as_bytes()))
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(cleaned.as_bytes()))
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(cleaned.as_bytes()))
        .map_err(|e| format!("base64 解码失败：{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_variants_decode() {
        assert_eq!(base64_decode("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGVs\nbG8=").unwrap(), b"hello");
        assert_eq!(base64_decode("aGVsbG8").unwrap(), b"hello");
        assert!(base64_decode("!!!").is_err());
    }

    #[test]
    fn version_is_set() {
        assert_eq!(version(), "0.1.0");
    }
}

/// 测试专用：把 Razor 引擎输出的 HTML 实体还原为原文。
///
/// Razor 子集的隐式表达式等价 .NET `HtmlEncoder.Default`——**非 ASCII 会被转成
/// `&#x大写十六进制;`**（见 DH.RustBase `Doc/Razor子集模板引擎架构.md` §3.2），
/// 因此断言中文必须先还原。
#[cfg(test)]
pub(crate) fn decode_html_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find("&#") {
        out.push_str(&rest[..index]);
        let tail = &rest[index + 2..];
        let (value, consumed) = if let Some(hex) = tail.strip_prefix(['x', 'X']) {
            match hex.find(';') {
                Some(end) => (
                    u32::from_str_radix(&hex[..end], 16).ok(),
                    end + 4, // "&#x" + 十六进制位 + ";"
                ),
                None => (None, 0),
            }
        } else {
            match tail.find(';') {
                Some(end) => (tail[..end].parse::<u32>().ok(), end + 3), // "&#" + 数字 + ";"
                None => (None, 0),
            }
        };
        match (value, consumed) {
            (Some(code), consumed) if consumed > 0 => {
                if let Some(ch) = char::from_u32(code) {
                    out.push(ch);
                    rest = &rest[index + consumed..];
                    continue;
                }
                out.push_str("&#");
                rest = &rest[index + 2..];
            }
            _ => {
                out.push_str("&#");
                rest = &rest[index + 2..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod decode_tests {
    use super::decode_html_entities;

    #[test]
    fn decodes_hex_and_named_entities() {
        assert_eq!(decode_html_entities("&#x4FDD;&#x5B58;"), "保存");
        assert_eq!(decode_html_entities("a&amp;b"), "a&amp;b");
        assert_eq!(decode_html_entities("&#65;"), "A");
        assert_eq!(decode_html_entities("no entities"), "no entities");
    }
}
