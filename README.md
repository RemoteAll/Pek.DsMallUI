# Pek.DsMallUI（Rust）

「猿人易 / NCube / DsMallUI」后台 UI 能力的 **Rust 统一封装**：UEditor 富文本后端、上传下载、
后台 Area（站点设置 / 登录）、Razor 子集视图与静态资源，全部复用
[DH.RustBase](../../DH.RustBase)（crate `dhrust`）的 **C# 风格 Web 规范**。

- 参考工程（C#）：`G:\Code\Pek.FrameWork\Pek.DsMallUI`
- 依赖公共库：`G:\Code\Rust\DH.RustBase`（`Controller` / `Router` / `StaticFiles` / `razor`）
- crate 名：`pek-dsmallui`

> **为什么要有这个包**：C# 侧的可移植资产只有三类——UEditor 后端协议、上传/下载控制器、
> 后台 Area 的视图与设置页。它们此前只能靠 ASP.NET Core + XCode 运行；现在 Rust 服务
> 也能用同一套路由/视图约定提供同样的后台界面（视图是 Razor 子集，双端可渲染）。

## 快速开始

```toml
[dependencies]
pek-dsmallui = { path = "../Pek.DsMallUI" }
dhrust = { path = "../DH.RustBase", features = ["net", "razor"] }
```

```rust,no_run
use std::sync::Arc;
use dhrust::net::http::{HttpOutcome, HttpResponse, HttpServer};
use dhrust::net::router::{route, Router};
use pek_dsmallui::{DsMallUi, MemoryBackend};

#[tokio::main]
async fn main() -> Result<(), String> {
    let ui = DsMallUi::builder()
        .web_root("wwwroot")          // 静态资源根（/static/** 等）
        .views_root("Views")          // Razor 视图根
        .upload_path("upload")        // 上传目录（相对 web_root），URL 前缀 /upload
        .backend(Arc::new(MemoryBackend::new()))  // 生产请换成自己的 AdminBackend
        .build()?;

    ui.materialize_defaults()?;       // 首次部署：落地默认视图 + ueconfig.json（只补缺失）

    let mut router = Router::new();
    ui.mount(&mut router);            // /fileuploader、/Upload/*、/admin/*

    let statics = ui.static_files();  // wwwroot + ETag/304
    router.fallback(route(move |ctx| {
        let statics = statics.clone();
        async move {
            match statics.try_serve_request(&ctx.req) {
                Some(response) => HttpOutcome::Response(response),
                None => HttpOutcome::Response(HttpResponse::text(404, "Not Found")),
            }
        }
    }));

    let server = HttpServer::bind("127.0.0.1:8080").await.map_err(|e| e.to_string())?;
    server.serve(router.into_handler()).await.map_err(|e| e.to_string())
}
```

本地跑起来看效果：

```powershell
cargo run --example admin_server            # 默认 127.0.0.1:8080
cargo run --example admin_server -- 8099    # 指定端口
```

| 入口 | 说明 |
|---|---|
| `GET /admin/config/index` | 站点设置页（Razor 子集渲染，套 `Views/Shared/_Layout.cshtml`） |
| `POST /admin/config/updatebase` | 保存设置（字段名与 C# `ConfigController.UpdateBase` 一致） |
| `GET /admin/login/index` | 登录页 |
| `POST /admin/login/login` | 登录（示例账号 `admin` / `admin123`） |
| `GET /fileuploader?action=config` | UEditor 配置（原样回吐 `ueconfig.json`） |
| `POST /fileuploader?action=uploadimage` | 图片上传（multipart，字段 `upfile`） |
| `POST /fileuploader?action=uploadscrawl` | 涂鸦（base64 data URL） |
| `POST /fileuploader?action=uploadfile` | 附件上传 |
| `POST /fileuploader?action=catchimage` | 远程图片抓取（`source[]`，需 `crawler` 特性） |
| `GET /fileuploader?action=listimage` | 图片管理器分页列表 |
| `GET /Upload/downloads/{path}` | 下载上传目录内文件（带 `Content-Disposition`、ETag/304） |

## 目录结构

```
Pek.DsMallUI/
├─ assets/ueconfig.json          # UEditor 配置（与 C# 工程同一份，含块注释）
├─ Views/                        # Razor 子集默认视图（可被项目覆盖）
│  ├─ Shared/_Layout.cshtml
│  ├─ Shared/Prompt.cshtml
│  ├─ Admin/Config/Index.cshtml
│  └─ Admin/Login/Index.cshtml
├─ src/
│  ├─ lib.rs                     # crate 文档 + 门面导出 + 实体解码（测试用）
│  ├─ options.rs                 # DsMallUi / DsMallUiBuilder（对应 PekStartup）
│  ├─ backend.rs                 # AdminBackend 抽象 + MemoryBackend（示例）
│  ├─ multipart.rs               # multipart/form-data 解析（dhrust 待办项的落地）
│  ├─ views.rs                   # Area 视图约定 Views/{Area}/{Controller}/{Action}.cshtml
│  ├─ models.rs                  # Hierarchy（折叠实体）+ flatten
│  ├─ controllers/{upload,admin}.rs
│  └─ ueditor/{mod,config,path,upload,list,crawler}.rs
├─ examples/admin_server.rs      # 可运行示例（起服务 + 全部入口）
├─ tools/sync-wwwroot.ps1        # 从 C# 参考工程同步第三方静态资源
└─ tools/regression.ps1          # 一键回归（构建 → 测试 → 冒烟）
```

## 静态资源（wwwroot）

C# 参考工程的 `wwwroot/static` 是 **825 个文件 / 24.6 MB 的第三方前端资产**
（UEditor、layer、layui、jQuery UI、swiper 等），不适合复制进 Rust 仓库。做法：

```powershell
# 把 C# 工程的 wwwroot/static 同步到本项目 wwwroot（增量，已存在且大小/时间一致的跳过）
.\tools\sync-wwwroot.ps1
```

`StaticFiles` 支持「磁盘目录 + 编译期嵌入」叠加，默认用磁盘目录（改文件即生效）。

## 与 C# 参考实现的差异

见 [`docs/移植对照.md`](docs/移植对照.md)。要点：

1. C# `UploadHandler.Process`、`Crawler.Fetch`、`UeditorConfig.GetStringList` 当前是**注释掉的**
   （上传一律「未知错误」、扩展名列表恒为空）；Rust 侧按 UEditor 官方协议**补齐实现**。
2. 视图从 C# 版（依赖 `@PekHtml.*`、`@T()`、tag helper）**改写为 Razor 子集**
   （`dhrust::razor`：`@expr` / `@if` / `@foreach` / `@{ var }` / `Layout` / `RenderBody` / `section` / `PartialAsync`）。
   Razor 子集把非 ASCII 输出转成 `&#x..;` 字面实体（等价 .NET `HtmlEncoder.Default`），浏览器显示正常。
3. `{rand:n}` 生成 **n 位**数字（C# 的 `rand.Next(10^n, 10^(n+1))` 实际是 n+1 位）。
4. 文件名清洗用 `dhrust::io::sanitize_file_name`（非法字符→`_`、`..`→`__`），并额外做落盘路径越界校验。
5. 后台持久化通过 `AdminBackend` 抽象注入，不绑定任何 ORM；C# 版直接写 XCode 实体。

## 依赖与约束

- 需要 `dhrust` 的 `net` + `razor` 特性（HTTP 服务/路由/控制器/视图引擎/静态资源）；
- `crawler` 特性（默认开启）额外启用 `dhrust/http-tls`（HTTPS 抓图）；
- 请求体上限由 `HttpServerOptions.max_body_size`（默认 64 MiB）兜底，
  `DsMallUiBuilder::max_upload_bytes` 再与 `ueconfig` 的 `*MaxSize` 取小值；
- 口令存储/校验一律走 `dhrust::sign::password`（与 C# `SaltPasswordProvider` 字节级互通），
  不要自写散列。

## 开发

```powershell
cargo check --all-targets   # 编译检查
cargo build --profile fast  # 日常调测（release 优化 + 无 LTO + 增量）
cargo test                  # 单元测试
.\tools\regression.ps1      # 一键回归（含起服务冒烟）
```

- crates 镜像：`.cargo/config.toml`（rsproxy）；Windows 宿主链接器用 `rust-lld`。
- 新增功能的**第一动作是检索**：先看 `dhrust` 有没有现成能力，再看本仓库实现，
  最后才写；本 crate 不重复造轮子（HTTP/路由/视图/静态资源/日志/口令/出网客户端全部来自 `dhrust`）。
