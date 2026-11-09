//! 门面：一次性组装「静态资源 + 上传控制器 + 后台 Area」，对外只暴露 [`DsMallUi`]。
//!
//! 对应 C# 参考工程的 `PekStartup`（`IPekStartup` 实现）：
//! `ConfigureServices` → [`DsMallUiBuilder`]；`UseDHEndpoints`/`Configure` → [`DsMallUi::mount`]。
//!
//! ```no_run
//! use std::sync::Arc;
//! use dhrust::net::router::Router;
//! use pek_dsmallui::{backend::MemoryBackend, DsMallUi};
//!
//! # async fn demo() -> Result<(), String> {
//! let ui = DsMallUi::builder()
//!     .web_root("wwwroot")
//!     .views_root("Views")
//!     .backend(Arc::new(MemoryBackend::new()))
//!     .build()?;
//!
//! // 首次部署：把内置视图与 ueconfig 落到磁盘（只补缺失）
//! ui.materialize_defaults()?;
//!
//! let mut router = Router::new();
//! ui.mount(&mut router);
//! let statics = ui.static_files();   // wwwroot 目录 + ETag/304
//! # let _ = (router, statics);
//! # Ok(())
//! # }
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use dhrust::net::router::Router;
use dhrust::net::static_files::StaticFiles;

use crate::backend::{AdminBackend, MemoryBackend};
use crate::controllers::admin::AdminArea;
use crate::controllers::upload::UploadController;
use crate::ueditor::{UeditorConfig, UeditorService};
use crate::views::Views;

/// 默认上传目录名（C# `UploadPath` 默认值同为 `upload`）。
pub const DEFAULT_UPLOAD_PATH: &str = "upload";
/// 默认请求体上限（与 `dhrust::net::http::DEFAULT_MAX_BODY_SIZE` 一致）。
pub const DEFAULT_MAX_UPLOAD_BYTES: u64 = 64 * 1024 * 1024;

/// DsMallUI 门面。
#[derive(Clone)]
pub struct DsMallUi {
    web_root: PathBuf,
    upload_root: PathBuf,
    upload_prefix: String,
    views: Views,
    upload: UploadController,
    admin: AdminArea,
}

impl DsMallUi {
    /// 构造器。
    pub fn builder() -> DsMallUiBuilder {
        DsMallUiBuilder::default()
    }

    /// 站点物理根（`wwwroot`）。
    pub fn web_root(&self) -> &Path {
        &self.web_root
    }

    /// 上传物理根（`<web_root>/<upload_path>`）。
    pub fn upload_root(&self) -> &Path {
        &self.upload_root
    }

    /// 上传对外 URL 前缀（`/upload`）。
    pub fn upload_prefix(&self) -> &str {
        &self.upload_prefix
    }

    /// 视图根。
    pub fn views(&self) -> &Views {
        &self.views
    }

    /// UEditor 服务（读取配置/单独挂载时使用）。
    pub fn ueditor(&self) -> &UeditorService {
        self.upload.service()
    }

    /// 后台 Area（读取数据源/复用页面模型时使用）。
    pub fn admin(&self) -> &AdminArea {
        &self.admin
    }

    /// 把路由挂到 `Router`：`/fileuploader`、`/Upload/*`、`/admin/config/*`、`/admin/login/*`。
    pub fn mount(&self, router: &mut Router) {
        self.upload.mount(router);
        self.admin.mount(router);
    }

    /// 静态资源服务（`wwwroot` 目录；由调用方挂到 `Router::fallback`）。
    pub fn static_files(&self) -> StaticFiles {
        StaticFiles::new(self.web_root.clone())
    }

    /// 首次部署：把内置视图与 `ueconfig.json` 落到磁盘（**只补缺失，不覆盖**）。
    ///
    /// - 视图 → `<views_root>/…`；
    /// - 配置 → `<web_root>/ueconfig.json`（若已存在则跳过）。
    pub fn materialize_defaults(&self) -> Result<usize, String> {
        let written = crate::views::materialize(self.views.root())
            .map_err(|e| format!("落地默认视图失败（{}）：{e}", self.views.root().display()))?;
        let config_path = self.web_root.join("ueconfig.json");
        if !config_path.is_file() {
            if let Some(parent) = config_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("创建目录失败（{}）：{e}", parent.display()))?;
            }
            std::fs::write(&config_path, crate::UE_CONFIG_JSON)
                .map_err(|e| format!("写入 ueconfig.json 失败（{}）：{e}", config_path.display()))?;
            return Ok(written + 1);
        }
        Ok(written)
    }
}

/// 构造器（对应 C# `PekStartup.ConfigureServices` 的配置面）。
pub struct DsMallUiBuilder {
    web_root: PathBuf,
    upload_path: String,
    views_root: PathBuf,
    backend: Arc<dyn AdminBackend>,
    ueditor_config: Option<PathBuf>,
    max_upload_bytes: u64,
}

impl Default for DsMallUiBuilder {
    fn default() -> Self {
        Self {
            web_root: PathBuf::from("wwwroot"),
            upload_path: DEFAULT_UPLOAD_PATH.to_string(),
            views_root: PathBuf::from("Views"),
            backend: Arc::new(MemoryBackend::new()),
            ueditor_config: None,
            max_upload_bytes: DEFAULT_MAX_UPLOAD_BYTES,
        }
    }
}

impl DsMallUiBuilder {
    /// 站点物理根（默认 `wwwroot`）。
    pub fn web_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.web_root = root.into();
        self
    }

    /// 上传目录名（默认 `upload`；相对 `web_root`）。
    pub fn upload_path(mut self, path: impl Into<String>) -> Self {
        self.upload_path = path.into();
        self
    }

    /// 视图根（默认 `Views`）。
    pub fn views_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.views_root = root.into();
        self
    }

    /// 后台数据源（默认 [`MemoryBackend`]，仅示例用）。
    pub fn backend(mut self, backend: Arc<dyn AdminBackend>) -> Self {
        self.backend = backend;
        self
    }

    /// 指定 `ueconfig.json`（默认内置；指定后解析失败即 `build()` 报错）。
    pub fn ueditor_config(mut self, path: impl Into<PathBuf>) -> Self {
        self.ueditor_config = Some(path.into());
        self
    }

    /// 全局上传体积上限（默认 64 MiB，与 HTTP 层默认 `max_body_size` 对齐）。
    pub fn max_upload_bytes(mut self, bytes: u64) -> Self {
        self.max_upload_bytes = bytes.max(1);
        self
    }

    /// 组装。
    pub fn build(self) -> Result<DsMallUi, String> {
        let upload_path = self.upload_path.trim().trim_matches('/');
        let upload_path = if upload_path.is_empty() {
            DEFAULT_UPLOAD_PATH
        } else {
            upload_path
        };
        let upload_root = self.web_root.join(upload_path);
        let upload_prefix = format!("/{upload_path}");

        let mut service = UeditorService::new(upload_root.clone(), &upload_prefix)
            .with_max_bytes(self.max_upload_bytes);
        if let Some(path) = self.ueditor_config {
            service = service.try_with_config_path(path)?;
        }

        let views = Views::new(self.views_root);
        let upload = UploadController::new(service, upload_root.clone());
        let admin = AdminArea::new(self.backend, views.clone());

        Ok(DsMallUi {
            web_root: self.web_root,
            upload_root,
            upload_prefix,
            views,
            upload,
            admin,
        })
    }
}

/// 内置 UEditor 配置原文（部署时可落地为 `wwwroot/ueconfig.json` 供前端读取）。
pub const UE_CONFIG_JSON: &str = include_str!("../assets/ueconfig.json");

/// 供测试/示例使用：内置配置对象。
pub fn embedded_ueditor_config() -> UeditorConfig {
    UeditorConfig::embedded()
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_wires_defaults() {
        let ui = DsMallUi::builder().build().unwrap();
        assert_eq!(ui.upload_prefix(), "/upload");
        assert!(ui.upload_root().ends_with("upload"));
        assert!(ui.ueditor().config().get_str("imageActionName").is_some());
    }

    #[test]
    fn builder_honours_paths() {
        let ui = DsMallUi::builder()
            .web_root("pub")
            .upload_path("files/")
            .views_root("Templates")
            .max_upload_bytes(1024)
            .build()
            .unwrap();
        assert_eq!(ui.upload_prefix(), "/files");
        assert!(ui.upload_root().ends_with("files"));
        assert_eq!(ui.views().root(), Path::new("Templates"));
    }

    #[test]
    fn missing_ueconfig_path_is_an_error() {
        let err = DsMallUi::builder()
            .ueditor_config("definitely-not-here.json")
            .build()
            .err()
            .expect("缺失配置必须报错");
        assert!(err.contains("读取 ueconfig 失败"), "{err}");
    }

    #[test]
    fn materialize_defaults_is_idempotent() {
        let root = std::env::temp_dir().join(format!("dsmallui-facade-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let ui = DsMallUi::builder()
            .web_root(root.join("wwwroot"))
            .views_root(root.join("Views"))
            .build()
            .unwrap();
        let first = ui.materialize_defaults().unwrap();
        assert!(first >= 4, "至少写入 4 个默认视图 + 配置");
        assert!(root.join("wwwroot/ueconfig.json").is_file());
        let second = ui.materialize_defaults().unwrap();
        assert_eq!(second, 0, "二次落地必须全部跳过");
        let _ = std::fs::remove_dir_all(&root);
    }
}
