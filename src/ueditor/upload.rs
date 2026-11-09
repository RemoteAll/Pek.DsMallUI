//! UEditor 文件上传（`uploadimage` / `uploadscrawl` / `uploadvideo` / `uploadfile`）。
//!
//! 行为对齐 C# 参考实现 `Pek.DsMallUI/UEditor/UploadHandler.cs` 的**设计意图**
//! （该文件当前把处理逻辑整体注释掉了，等价于「任何上传都返回未知错误」；
//! 这里按官方 UEditor 后端协议补齐，字段与状态文案保持 C# 一致）。
//!
//! 安全要点：
//! - **扩展名白名单**：取 `ueconfig.json` 的 `*AllowFiles`，归一化后比较（空列表 = 全拒）；
//! - **体积上限**：取 `*MaxSize`，与调用方 `max_upload_bytes` 取小值；
//! - **落盘路径二次校验**：格式化结果不得含 `..`，且必须落在 `root` 之内
//!   （`{filename}` 来自客户端，`sanitize_file_name` 已折叠 `..`，这里再做一次纵深防御）。

use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use crate::multipart::FormData;

use super::config::{normalize_extension, UeditorConfig};
use super::path::{extension_of, format_path};

/// 上传结果状态（文案与 C# `UploadHandler.GetStateMessage` 完全一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UploadState {
    /// 成功
    Success,
    /// 不允许的文件格式
    TypeNotAllow,
    /// 文件大小超出服务器限制
    SizeLimitExceed,
    /// 文件访问出错，请检查写入权限
    FileAccessError,
    /// 网络错误
    NetworkError,
    /// 未知错误
    #[default]
    Unknown,
}

impl UploadState {
    /// 返回给 UEditor 前端的 `state` 文本（同时也是 UEditor 判定成功的唯一依据）。
    pub fn message(&self) -> &'static str {
        match self {
            UploadState::Success => "SUCCESS",
            UploadState::TypeNotAllow => "不允许的文件格式",
            UploadState::SizeLimitExceed => "文件大小超出服务器限制",
            UploadState::FileAccessError => "文件访问出错，请检查写入权限",
            UploadState::NetworkError => "网络错误",
            UploadState::Unknown => "未知错误",
        }
    }

    /// 是否成功。
    pub fn is_success(&self) -> bool {
        matches!(self, UploadState::Success)
    }
}

/// 单类上传的配置（对应 C# `UploadConfig`）。
#[derive(Debug, Clone)]
pub struct UploadConfig {
    /// 允许的扩展名（已归一化为小写、带前导点）
    pub allow_extensions: Vec<String>,
    /// 保存路径模板（如 `/images/{yyyy}/{mm}/{dd}/{filename}_{hh}{ii}`）
    pub path_format: String,
    /// 体积上限（字节）
    pub size_limit: u64,
    /// 表单字段名
    pub field_name: String,
    /// 是否 base64 上传（涂鸦）
    pub base64: bool,
    /// base64 上传时的文件名
    pub base64_filename: String,
}

impl UploadConfig {
    /// 图片上传（`image*` 配置）。
    pub fn image(config: &UeditorConfig) -> UploadConfig {
        Self::from_prefix(config, "image", "upfile")
    }

    /// 涂鸦上传（`scrawl*` 配置；base64 表单值）。
    pub fn scrawl(config: &UeditorConfig) -> UploadConfig {
        let mut cfg = Self::from_prefix(config, "scrawl", "upfile");
        cfg.allow_extensions = vec![".png".to_string()];
        cfg.base64 = true;
        cfg.base64_filename = "scrawl.png".to_string();
        cfg
    }

    /// 视频上传（`video*` 配置）。
    pub fn video(config: &UeditorConfig) -> UploadConfig {
        Self::from_prefix(config, "video", "upfile")
    }

    /// 附件上传（`file*` 配置）。
    pub fn file(config: &UeditorConfig) -> UploadConfig {
        Self::from_prefix(config, "file", "upfile")
    }

    fn from_prefix(config: &UeditorConfig, prefix: &str, default_field: &str) -> UploadConfig {
        let field = format!("{prefix}FieldName");
        let path = format!("{prefix}PathFormat");
        let size = format!("{prefix}MaxSize");
        let allow = format!("{prefix}AllowFiles");
        UploadConfig {
            allow_extensions: config.get_string_list(&allow),
            path_format: config
                .get_str(&path)
                .unwrap_or(super::path::DEFAULT_PATH_FORMAT)
                .to_string(),
            size_limit: config
                .get_i64(&size)
                .filter(|v| *v > 0)
                .map(|v| v as u64)
                .unwrap_or(20 * 1024 * 1024),
            field_name: config
                .get_str(&field)
                .unwrap_or(default_field)
                .to_string(),
            base64: false,
            base64_filename: String::new(),
        }
    }
}

/// 一次上传的结果（序列化为 UEditor 约定的 JSON）。
#[derive(Debug, Clone, Default)]
pub struct UploadOutcome {
    /// 状态
    pub state: UploadState,
    /// 可访问 URL（成功时）
    pub url: Option<String>,
    /// 标题（原文件名，UEditor 用于展示）
    pub title: Option<String>,
    /// 原始文件名
    pub original: Option<String>,
    /// 文件类型（扩展名，含点）
    pub file_type: Option<String>,
    /// 文件大小（字节）
    pub size: u64,
    /// 错误详情（失败时；成功为 `None`）
    pub error: Option<String>,
}

impl UploadOutcome {
    /// 失败结果。
    pub fn fail(state: UploadState, error: impl Into<String>) -> UploadOutcome {
        UploadOutcome {
            state,
            error: Some(error.into()),
            ..Default::default()
        }
    }

    /// 转 UEditor JSON 对象（`state` 恒有；空字段省略，避免前端把 null 当字符串）。
    pub fn to_json_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("state".into(), json!(self.state.message()));
        if let Some(url) = &self.url {
            map.insert("url".into(), json!(url));
        }
        if let Some(title) = &self.title {
            map.insert("title".into(), json!(title));
        }
        if let Some(original) = &self.original {
            map.insert("original".into(), json!(original));
        }
        if let Some(file_type) = &self.file_type {
            map.insert("type".into(), json!(file_type));
        }
        if self.state.is_success() {
            map.insert("size".into(), json!(self.size));
        }
        if let Some(error) = &self.error {
            map.insert("error".into(), json!(error));
        }
        Value::Object(map)
    }
}

/// 上传落盘器（绑定一个物理根目录与对外 URL 前缀）。
#[derive(Debug, Clone)]
pub struct Uploader {
    root: PathBuf,
    public_prefix: String,
    max_bytes: u64,
}

impl Uploader {
    /// `root` = 物理上传根目录（如 `wwwroot/upload`）；
    /// `public_prefix` = 对外 URL 前缀（如 `/upload`，末尾斜杠自动去掉）。
    pub fn new(root: impl Into<PathBuf>, public_prefix: &str) -> Uploader {
        Uploader {
            root: root.into(),
            public_prefix: normalize_prefix(public_prefix),
            max_bytes: u64::MAX,
        }
    }

    /// 全局体积上限（与配置里的 `*MaxSize` 取小值；默认不额外限制）。
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Uploader {
        self.max_bytes = max_bytes.max(1);
        self
    }

    /// 物理根目录。
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// 对外 URL 前缀。
    pub fn public_prefix(&self) -> &str {
        &self.public_prefix
    }

    /// 从多段表单取文件并落盘。
    pub fn save_multipart(&self, config: &UploadConfig, form: &FormData) -> UploadOutcome {
        let Some(part) = form.file(&config.field_name) else {
            return UploadOutcome::fail(
                UploadState::Unknown,
                format!("表单缺少文件字段 {}（multipart 解析成功）", config.field_name),
            );
        };
        let filename = part.filename.clone().unwrap_or_else(|| "file".to_string());
        self.save_bytes(config, &filename, &part.data)
    }

    /// 从 base64 表单值落盘（涂鸦）：取值形如 `data:image/png;base64,iVBOR...`。
    pub fn save_base64(&self, config: &UploadConfig, form: &FormData) -> UploadOutcome {
        let Some(raw) = form.value(&config.field_name) else {
            return UploadOutcome::fail(
                UploadState::Unknown,
                format!("表单缺少涂鸦字段 {}", config.field_name),
            );
        };
        let payload = raw.split("base64,").nth(1).unwrap_or(raw);
        // data URL 里的 `+` 可能被 urlencoded 转义成空格，这里还原
        let payload = payload.replace(' ', "+");
        let data = match crate::base64_decode(&payload) {
            Ok(data) => data,
            Err(e) => {
                return UploadOutcome::fail(
                    UploadState::FileAccessError,
                    format!("涂鸦 base64 解码失败：{e}"),
                )
            }
        };
        let name = if config.base64_filename.is_empty() {
            "scrawl.png"
        } else {
            &config.base64_filename
        };
        self.save_bytes(config, name, &data)
    }

    /// 校验并落盘（扩展名 → 体积 → 路径 → 写入）。
    pub fn save_bytes(
        &self,
        config: &UploadConfig,
        original_name: &str,
        data: &[u8],
    ) -> UploadOutcome {
        let original_name = original_name.trim();
        let original_name = if original_name.is_empty() {
            "file"
        } else {
            original_name
        };

        let extension = normalize_extension(&extension_of(original_name));
        if !config
            .allow_extensions
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(&extension))
        {
            return UploadOutcome::fail(
                UploadState::TypeNotAllow,
                format!(
                    "扩展名 {} 不在允许列表 [{}]",
                    if extension.is_empty() { "(无)" } else { &extension },
                    config.allow_extensions.join(", ")
                ),
            );
        }

        let size = data.len() as u64;
        let limit = config.size_limit.min(self.max_bytes);
        if size > limit {
            return UploadOutcome::fail(
                UploadState::SizeLimitExceed,
                format!("文件 {size} 字节，超过上限 {limit} 字节"),
            );
        }

        let relative = format_path(original_name, &config.path_format);
        let relative = relative.trim_start_matches('/');
        if relative.contains("..") {
            return UploadOutcome::fail(
                UploadState::FileAccessError,
                format!("保存路径包含非法片段：{relative}"),
            );
        }

        let physical = self.root.join(relative.replace('\\', "/"));
        if !is_within(&self.root, &physical) {
            return UploadOutcome::fail(
                UploadState::FileAccessError,
                format!("保存路径越界：{}", physical.display()),
            );
        }

        if let Some(dir) = physical.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                return UploadOutcome::fail(
                    UploadState::FileAccessError,
                    format!("创建目录失败（{}）：{e}", dir.display()),
                );
            }
        }
        if let Err(e) = std::fs::write(&physical, data) {
            return UploadOutcome::fail(
                UploadState::FileAccessError,
                format!("写入文件失败（{}）：{e}", physical.display()),
            );
        }

        UploadOutcome {
            state: UploadState::Success,
            url: Some(format!(
                "{}/{}",
                self.public_prefix,
                relative.replace('\\', "/")
            )),
            title: Some(original_name.to_string()),
            original: Some(original_name.to_string()),
            file_type: Some(extension),
            size,
            error: None,
        }
    }
}

/// URL 前缀归一化：保证以 `/` 开头且无尾斜杠（空串 → 空串）。
fn normalize_prefix(prefix: &str) -> String {
    let trimmed = prefix.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

/// `path` 是否位于 `root` 之内（不解析符号链接；用于阻断 `..` 逃逸）。
fn is_within(root: &Path, path: &Path) -> bool {
    let root = absolute(root);
    let path = absolute(path);
    path.starts_with(&root)
}

/// 尽量转绝对路径（不做 IO，仅字符串层归一 `..`）。
fn absolute(path: &Path) -> PathBuf {
    let base = if path.is_absolute() {
        PathBuf::new()
    } else {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    };
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "dsmallui-upload-{tag}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn image_config() -> UploadConfig {
        UploadConfig {
            allow_extensions: vec![".png".into(), ".jpg".into()],
            path_format: "/images/{yyyy}/{filename}".into(),
            size_limit: 1024,
            field_name: "upfile".into(),
            base64: false,
            base64_filename: String::new(),
        }
    }

    #[test]
    fn saves_allowed_file() {
        let root = temp_root("ok");
        let uploader = Uploader::new(&root, "/upload");
        let outcome = uploader.save_bytes(&image_config(), "pic.png", b"abc");
        assert!(outcome.state.is_success(), "{outcome:?}");
        let url = outcome.url.unwrap();
        assert!(url.starts_with("/upload/images/"), "{url}");
        assert!(url.ends_with("/pic.png"), "{url}");
        let physical = root.join(url.trim_start_matches("/upload/"));
        assert_eq!(std::fs::read(physical).unwrap(), b"abc");
    }

    #[test]
    fn rejects_disallowed_extension() {
        let root = temp_root("ext");
        let uploader = Uploader::new(&root, "/upload");
        let outcome = uploader.save_bytes(&image_config(), "run.exe", b"x");
        assert_eq!(outcome.state, UploadState::TypeNotAllow);
        assert!(outcome.error.unwrap().contains("不在允许列表"));
    }

    #[test]
    fn rejects_oversize() {
        let root = temp_root("size");
        let uploader = Uploader::new(&root, "/upload");
        let data = vec![0u8; 2048];
        let outcome = uploader.save_bytes(&image_config(), "big.png", &data);
        assert_eq!(outcome.state, UploadState::SizeLimitExceed);
    }

    #[test]
    fn global_limit_wins() {
        let root = temp_root("global");
        let uploader = Uploader::new(&root, "/upload").with_max_bytes(4);
        let outcome = uploader.save_bytes(&image_config(), "a.png", b"12345");
        assert_eq!(outcome.state, UploadState::SizeLimitExceed);
    }

    #[test]
    fn missing_field_reports_clear_reason() {
        let root = temp_root("nofield");
        let uploader = Uploader::new(&root, "/upload");
        let form = FormData::default();
        let outcome = uploader.save_multipart(&image_config(), &form);
        assert!(!outcome.state.is_success());
        assert!(outcome.error.unwrap().contains("upfile"));
    }

    #[test]
    fn json_shape_matches_ueditor() {
        let root = temp_root("json");
        let uploader = Uploader::new(&root, "/upload");
        let outcome = uploader.save_bytes(&image_config(), "a.png", b"1");
        let value = outcome.to_json_value();
        assert_eq!(value["state"], "SUCCESS");
        assert!(value["url"].as_str().unwrap().ends_with("a.png"));
        assert_eq!(value["original"], "a.png");
        assert_eq!(value["type"], ".png");
        assert_eq!(value["size"], 1);
    }

    #[test]
    fn path_traversal_is_blocked() {
        let root = temp_root("trav");
        let uploader = Uploader::new(&root, "/upload");
        let cfg = UploadConfig {
            allow_extensions: vec![".txt".into()],
            path_format: "/{filename}".into(),
            ..image_config()
        };
        let outcome = uploader.save_bytes(&cfg, "..%2F..%2Fetc.txt", b"x");
        // `%2F` 不是分隔符，清洗后仍是普通文件名 → 允许；关键是不能越出 root
        let url = outcome.url.unwrap_or_default();
        assert!(!url.contains(".."), "{url}");
    }
}
