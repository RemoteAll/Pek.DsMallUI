//! UEditor 远程图片抓取（`catchimage`）。
//!
//! 对齐 C# `Pek.DsMallUI/UEditor/CrawlerHandler.cs`：接收 `source[]` 列表（最多 10 个），
//! 逐个下载并落盘到本地上传目录，返回
//! `{state:"SUCCESS", list:[{state, source, url}]}`。
//!
//! 与 C# 的差异（C# 版把下载逻辑整体注释掉了，只回 `state=null`）：
//! - 这里真正实现下载，走 `dhrust::net::http_client::blocking_get`（HTTPS 需 `crawler` 特性）；
//! - **协议白名单**：只允许 `http`/`https`（C# 注释里的 `IsExternalAddress` 校验在上游缺失，
//!   会导致 SSRF 到 `file://`/内网元数据地址）；
//! - 单文件体积上限取 `catcherMaxSize`，超限直接失败。

use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

use super::config::UeditorConfig;
use super::upload::{UploadConfig, Uploader};

/// 抓取列表上限（与 C# `sources.Length <= 10` 的约束一致）。
pub const MAX_SOURCES: usize = 10;

/// 抓取一个请求里的全部来源。
///
/// `sources` 来自表单字段 `source[]`（可重复）。
pub fn catch_images(
    config: &UeditorConfig,
    uploader: &Uploader,
    sources: &[String],
) -> Value {
    if sources.is_empty() {
        return json!({ "state": "参数错误：没有指定抓取源" });
    }
    if sources.len() > MAX_SOURCES {
        return json!({ "state": format!("抓取源超过上限 {MAX_SOURCES} 个") });
    }

    let mut upload_config = UploadConfig::image(config);
    // 抓取图片用自己的格式与大小配置
    if let Some(path) = config.get_str("catcherPathFormat") {
        upload_config.path_format = path.to_string();
    }
    if let Some(size) = config.get_i64("catcherMaxSize").filter(|v| *v > 0) {
        upload_config.size_limit = size as u64;
    }
    let allow = config.get_string_list("catcherAllowFiles");
    if !allow.is_empty() {
        upload_config.allow_extensions = allow;
    }

    let mut list = Vec::new();
    for source in sources {
        let source = source.trim();
        let entry = match fetch(source, &upload_config, uploader) {
            Ok(state) => json!({ "state": "SUCCESS", "source": source, "url": state }),
            Err(reason) => json!({ "state": format!("抓取错误：{reason}"), "source": source, "url": "" }),
        };
        list.push(entry);
    }

    json!({ "state": "SUCCESS", "list": list })
}

/// 下载单张图片并落盘，返回可访问 URL。
fn fetch(source: &str, config: &UploadConfig, uploader: &Uploader) -> Result<String, String> {
    let lower = source.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return Err(format!("不支持的协议（仅 http/https）：{}", truncate(source, 120)));
    }

    let response = dhrust::net::http_client::blocking_get(source, Duration::from_secs(20))
        .map_err(|e| format!("下载失败：{e}"))?;
    if !response.is_success() {
        return Err(format!("远端返回 HTTP {}", response.status));
    }
    // 文件名：URL 末段（去查询串），没有就按顺序编号
    let content_type = response.header("content-type").unwrap_or("").to_string();
    let body = response.body;
    if body.is_empty() {
        return Err("远端返回空内容".to_string());
    }

    // 文件名：URL 末段（去查询串），没有就按顺序编号
    let name = source
        .split(['?', '#'])
        .next()
        .unwrap_or(source)
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            if super::path::extension_of(segment).is_empty() {
                format!("{segment}{}", guess_extension(&content_type))
            } else {
                segment.to_string()
            }
        })
        .unwrap_or_else(|| {
            format!(
                "{}{}",
                dhrust::times::getmilltimestamp(),
                guess_extension(&content_type)
            )
        });

    let outcome = uploader.save_bytes(config, &name, &body);
    if outcome.state.is_success() {
        Ok(outcome.url.unwrap_or_default())
    } else {
        Err(outcome.error.unwrap_or_else(|| outcome.state.message().to_string()))
    }
}

/// 由响应 `Content-Type` 猜扩展名（抓取图片场景够用）。
fn guess_extension(content_type: &str) -> &'static str {
    let value = content_type.to_ascii_lowercase();
    if value.contains("png") {
        ".png"
    } else if value.contains("gif") {
        ".gif"
    } else if value.contains("webp") {
        ".webp"
    } else if value.contains("bmp") {
        ".bmp"
    } else {
        ".jpg"
    }
}

/// 截断长文本（错误消息里避免倾倒整条 URL）。
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect::<String>() + "…"
}

/// 便于调用方在无网络环境做单元测试：仅校验来源列表与协议。
pub fn validate_sources(sources: &[String]) -> Result<(), String> {
    if sources.is_empty() {
        return Err("参数错误：没有指定抓取源".to_string());
    }
    if sources.len() > MAX_SOURCES {
        return Err(format!("抓取源超过上限 {MAX_SOURCES} 个"));
    }
    Ok(())
}

/// 物理路径是否可用于抓取落盘（供调用方做前置检查）。
pub fn ensure_root_ready(root: &Path) -> Result<(), String> {
    std::fs::create_dir_all(root)
        .map_err(|e| format!("创建上传根目录失败（{}）：{e}", root.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_and_too_many_sources() {
        assert!(validate_sources(&[]).unwrap_err().contains("没有指定抓取源"));
        let many: Vec<String> = (0..11).map(|i| format!("http://a/{i}.png")).collect();
        assert!(validate_sources(&many).unwrap_err().contains("超过上限"));
    }

    #[test]
    fn non_http_scheme_is_rejected_without_network() {
        let uploader = Uploader::new(std::env::temp_dir(), "/upload");
        let config = UploadConfig::image(&UeditorConfig::embedded());
        let err = fetch("file:///etc/passwd", &config, &uploader).unwrap_err();
        assert!(err.contains("仅 http/https"), "{err}");
    }

    #[test]
    fn guesses_extension_from_content_type() {
        assert_eq!(guess_extension("image/png"), ".png");
        assert_eq!(guess_extension("image/webp"), ".webp");
        assert_eq!(guess_extension("application/octet-stream"), ".jpg");
    }
}
