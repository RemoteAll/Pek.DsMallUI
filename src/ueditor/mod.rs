//! UEditor 服务端协议实现（对齐百度 UEditor 1.4 后端约定）。
//!
//! 入口是 [`UeditorService::handle`]：按 `?action=...` 分发到配置 / 上传 / 涂鸦 /
//! 远程抓取 / 文件列表，统一返回 JSON（可用 `?callback=` 包裹为 JSONP，与 C# `Handler.WriteJson` 一致）。
//!
//! 配置来源优先级：`UeditorService::with_config_path` 指定的文件 → 内置
//! `assets/ueconfig.json`（与 C# 参考工程同一份）。
//!
//! ```no_run
//! use pek_dsmallui::ueditor::UeditorService;
//!
//! let service = UeditorService::new("wwwroot/upload", "/upload");
//! // HTTP 层把 action / query / Content-Type / body 传进来即可
//! let json = service.handle("config", &[], None, b"");
//! ```

pub mod config;
#[cfg(feature = "crawler")]
pub mod crawler;
pub mod list;
pub mod path;
pub mod upload;

use std::path::{Path, PathBuf};

pub use config::UeditorConfig;
pub use list::FileList;
pub use upload::{UploadConfig, UploadOutcome, UploadState, Uploader};

use crate::multipart::FormData;

/// UEditor 服务（配置 + 落盘器；`Clone` 可共享进路由闭包）。
#[derive(Clone)]
pub struct UeditorService {
    config: UeditorConfig,
    uploader: Uploader,
    /// 全局上传体积上限（与配置里的 `*MaxSize` 取小值）
    max_bytes: u64,
}

impl UeditorService {
    /// 使用内置配置创建服务。
    ///
    /// - `upload_root`：物理上传根目录（如 `wwwroot/upload`）；
    /// - `public_prefix`：对外 URL 前缀（如 `/upload`）。
    pub fn new(upload_root: impl Into<PathBuf>, public_prefix: &str) -> UeditorService {
        UeditorService {
            config: UeditorConfig::embedded(),
            uploader: Uploader::new(upload_root, public_prefix),
            max_bytes: u64::MAX,
        }
    }

    /// 指定 `ueconfig.json` 路径（解析失败请用 [`UeditorService::try_with_config_path`]）。
    pub fn with_config_path(mut self, path: impl AsRef<Path>) -> UeditorService {
        match UeditorConfig::from_file(path) {
            Ok(config) => self.config = config,
            Err(e) => {
                dhrust::logs::warn!(
                    "[DsMallUI] 加载 ueconfig 失败，退回内置配置：{e}"
                );
            }
        }
        self
    }

    /// 指定 `ueconfig.json` 路径（失败即返回错误，不做静默回退）。
    pub fn try_with_config_path(
        mut self,
        path: impl AsRef<Path>,
    ) -> Result<UeditorService, String> {
        self.config = UeditorConfig::from_file(path)?;
        Ok(self)
    }

    /// 直接注入配置（测试/多站点共用一份配置时使用）。
    pub fn with_config(mut self, config: UeditorConfig) -> UeditorService {
        self.config = config;
        self
    }

    /// 全局体积上限。
    pub fn with_max_bytes(mut self, max_bytes: u64) -> UeditorService {
        self.max_bytes = max_bytes.max(1);
        self.uploader = self.uploader.clone().with_max_bytes(max_bytes);
        self
    }

    /// 当前配置。
    pub fn config(&self) -> &UeditorConfig {
        &self.config
    }

    /// 落盘器。
    pub fn uploader(&self) -> &Uploader {
        &self.uploader
    }

    /// 处理一次 UEditor 请求，返回 JSON 文本（`callback` 存在时为 JSONP）。
    ///
    /// - `action`：查询串 `action`；
    /// - `query`：完整查询串（取 `callback` / `start` / `size`）；
    /// - `content_type`：请求头 `Content-Type`（multipart 解析用）；
    /// - `body`：请求体原始字节。
    pub fn handle(
        &self,
        action: &str,
        query: &[(String, String)],
        content_type: Option<&str>,
        body: &[u8],
    ) -> String {
        let json = self.dispatch(action, query, content_type, body);
        match query_value(query, "callback").filter(|v| !v.trim().is_empty()) {
            Some(callback) => format!("{callback}({json});"),
            None => json,
        }
    }

    /// 分发实现（不含 JSONP 包装）。
    fn dispatch(
        &self,
        action: &str,
        query: &[(String, String)],
        content_type: Option<&str>,
        body: &[u8],
    ) -> String {
        let action = action.trim();
        if action.eq_ignore_ascii_case("config")
            || action.eq_ignore_ascii_case(
                self.config.get_str("configActionName").unwrap_or("config"),
            )
        {
            return self.config.to_json();
        }

        if self.is_action(action, "imageActionName") {
            return self.upload_multipart(UploadConfig::image(&self.config), content_type, body);
        }
        if self.is_action(action, "scrawlActionName") {
            return self.upload_base64(UploadConfig::scrawl(&self.config), content_type, body);
        }
        if self.is_action(action, "videoActionName") {
            return self.upload_multipart(UploadConfig::video(&self.config), content_type, body);
        }
        if self.is_action(action, "fileActionName") {
            return self.upload_multipart(UploadConfig::file(&self.config), content_type, body);
        }
        if self.is_action(action, "catcherActionName") {
            return self.catch_images(content_type, body);
        }
        if self.is_action(action, "imageManagerActionName") {
            return self.list_files(
                "imageManagerListPath",
                "imageManagerAllowFiles",
                "imageManagerListSize",
                query,
            );
        }
        if self.is_action(action, "fileManagerActionName") {
            return self.list_files(
                "fileManagerListPath",
                "fileManagerAllowFiles",
                "fileManagerListSize",
                query,
            );
        }

        serde_json::json!({ "state": format!("不支持的操作：{action}") }).to_string()
    }

    /// 动作名是否匹配配置里的键（大小写不敏感；配置缺失时回退到官方默认名）。
    fn is_action(&self, action: &str, key: &str) -> bool {
        let configured = self.config.get_str(key).unwrap_or(default_action_name(key));
        action.eq_ignore_ascii_case(configured)
    }

    /// 多段表单上传（图片/视频/附件）。
    fn upload_multipart(
        &self,
        config: UploadConfig,
        content_type: Option<&str>,
        body: &[u8],
    ) -> String {
        let form = match FormData::parse(content_type, body) {
            Ok(form) => form,
            Err(e) => {
                return serde_json::json!({ "state": format!("表单解析失败：{e}") }).to_string()
            }
        };
        self.save_upload(&config, &form, false)
    }

    /// base64 表单上传（涂鸦）。
    fn upload_base64(
        &self,
        config: UploadConfig,
        content_type: Option<&str>,
        body: &[u8],
    ) -> String {
        let form = match FormData::parse(content_type, body) {
            Ok(form) => form,
            Err(e) => {
                return serde_json::json!({ "state": format!("表单解析失败：{e}") }).to_string()
            }
        };
        self.save_upload(&config, &form, true)
    }

    fn save_upload(&self, config: &UploadConfig, form: &FormData, base64: bool) -> String {
        let outcome = if base64 {
            self.uploader.save_base64(config, form)
        } else {
            self.uploader.save_multipart(config, form)
        };
        if !outcome.state.is_success() {
            // 失败必须留下可定位日志（含状态与具体原因）
            dhrust::logs::warn!(
                "[DsMallUI] UEditor 上传失败：{}（字段 {}，路径模板 {}）",
                outcome
                    .error
                    .clone()
                    .unwrap_or_else(|| outcome.state.message().to_string()),
                config.field_name,
                config.path_format
            );
        }
        outcome.to_json_value().to_string()
    }

    /// 远程图片抓取。
    #[cfg(feature = "crawler")]
    fn catch_images(&self, content_type: Option<&str>, body: &[u8]) -> String {
        let form = match FormData::parse(content_type, body) {
            Ok(form) => form,
            Err(e) => {
                return serde_json::json!({ "state": format!("表单解析失败：{e}") }).to_string()
            }
        };
        // UEditor 前端提交 `source[]`（可能出现空名 `source[]` 多值，也可能写成 `source`）
        let mut sources: Vec<String> = form.values("source[]").iter().map(|s| s.to_string()).collect();
        if sources.is_empty() {
            sources = form.values("source").iter().map(|s| s.to_string()).collect();
        }
        crawler::catch_images(&self.config, &self.uploader, &sources).to_string()
    }

    /// 无 `crawler` 特性时的明确提示（不静默返回空）。
    #[cfg(not(feature = "crawler"))]
    fn catch_images(&self, _content_type: Option<&str>, _body: &[u8]) -> String {
        serde_json::json!({ "state": "未启用 crawler 特性，无法抓取远程图片" }).to_string()
    }

    /// 文件管理器列表。
    fn list_files(
        &self,
        path_key: &str,
        allow_key: &str,
        size_key: &str,
        query: &[(String, String)],
    ) -> String {
        let relative = self.config.get_str(path_key).unwrap_or("/");
        let extensions = self.config.get_string_list(allow_key);
        let start = query_value(query, "start")
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        let size = query_value(query, "size")
            .and_then(|v| v.trim().parse::<usize>().ok())
            .or_else(|| self.config.get_i64(size_key).map(|v| v.max(1) as usize))
            .unwrap_or(20);

        match list::list_files(
            self.uploader.root(),
            relative,
            &extensions,
            self.uploader.public_prefix(),
            start,
            size,
        ) {
            Ok(page) => {
                let list: Vec<serde_json::Value> = page
                    .items
                    .iter()
                    .map(|url| serde_json::json!({ "url": url }))
                    .collect();
                serde_json::json!({
                    "state": "SUCCESS",
                    "list": list,
                    "start": page.start,
                    "size": page.size,
                    "total": page.total,
                })
                .to_string()
            }
            Err(e) => serde_json::json!({ "state": e }).to_string(),
        }
    }
}

/// 配置键对应的官方默认动作名。
fn default_action_name(key: &str) -> &'static str {
    match key {
        "imageActionName" => "uploadimage",
        "scrawlActionName" => "uploadscrawl",
        "videoActionName" => "uploadvideo",
        "fileActionName" => "uploadfile",
        "catcherActionName" => "catchimage",
        "imageManagerActionName" => "listimage",
        "fileManagerActionName" => "listfile",
        _ => "",
    }
}

/// 查询串取值（大小写不敏感；首个匹配）。
pub fn query_value<'a>(query: &'a [(String, String)], name: &str) -> Option<&'a str> {
    query
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsmallui-ueditor-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn multipart_body(field: &str, filename: &str, data: &[u8]) -> (String, Vec<u8>) {
        let boundary = "----DsMallUITest";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\n\
                 Content-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={boundary}"), body)
    }

    #[test]
    fn config_action_returns_raw_config() {
        let service = UeditorService::new(temp_root("cfg"), "/upload");
        let json = service.handle("config", &[], None, b"");
        assert!(json.contains("imageActionName"), "{json}");
    }

    #[test]
    fn config_action_supports_jsonp() {
        let service = UeditorService::new(temp_root("jsonp"), "/upload");
        let query = vec![("callback".to_string(), "cb".to_string())];
        let json = service.handle("config", &query, None, b"");
        assert!(json.starts_with("cb("), "{json}");
        assert!(json.ends_with(");"), "{json}");
    }

    #[test]
    fn upload_image_writes_file_and_returns_success() {
        let root = temp_root("img");
        let service = UeditorService::new(&root, "/upload");
        let (ct, body) = multipart_body("upfile", "a.png", b"\x89PNG");
        let json = service.handle("uploadimage", &[], Some(&ct), &body);
        assert!(json.contains("\"state\":\"SUCCESS\""), "{json}");
        assert!(json.contains("/upload/images/"), "{json}");
    }

    #[test]
    fn upload_rejects_unknown_action() {
        let service = UeditorService::new(temp_root("act"), "/upload");
        let json = service.handle("nope", &[], None, b"");
        assert!(json.contains("不支持的操作"), "{json}");
    }

    #[test]
    fn list_image_pages_files() {
        let root = temp_root("list");
        let dir = root.join("images");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.png"), b"1").unwrap();
        let service = UeditorService::new(&root, "/upload");
        let query = vec![
            ("start".to_string(), "0".to_string()),
            ("size".to_string(), "10".to_string()),
        ];
        let json = service.handle("listimage", &query, None, b"");
        assert!(json.contains("\"total\":1"), "{json}");
        assert!(json.contains("/upload/images/x.png"), "{json}");
    }

    #[test]
    fn scrawl_decodes_data_url() {
        let root = temp_root("scrawl");
        let service = UeditorService::new(&root, "/upload");
        // "aGVsbG8=" = "hello"
        let boundary = "b";
        let body = "--b\r\nContent-Disposition: form-data; name=\"upfile\"\r\n\r\ndata:image/png;base64,aGVsbG8=\r\n--b--";
        let ct = format!("multipart/form-data; boundary={boundary}");
        let json = service.handle("uploadscrawl", &[], Some(&ct), body.as_bytes());
        assert!(json.contains("\"state\":\"SUCCESS\""), "{json}");
    }
}
