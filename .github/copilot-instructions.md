# Pek.DsMallUI · 项目指令

> 全项目通用规则见用户级《跨项目接续协议》（本机 `%APPDATA%\Code\User\prompts\跨项目接续协议.instructions.md`；模板在 `tools/continuity/templates/`）。
> **开工先读（存在则读，按序）**：`.github/copilot-instructions.md` → `docs/项目接续指南.md` → `docs/用户档案.md` → `docs/项目记忆.md` → `docs/决策记录.md` 最新节。
> **机器自愈（有则快跑）**：`tools/continuity/install-global-instruction.ps1`（幂等；打开项目时 `.vscode/tasks.json` 也会自动跑）。

## 项目定位

`Pek.DsMallUI`（crate `pek-dsmallui`）是 C# `Pek.DsMallUI`（猿人易 / NCube 后台 UI）的 Rust 对应实现：
**后台 UI 能力库**（UEditor 后端、上传下载、后台 Area 与 Razor 子集视图），供生态内 Rust 服务以 path 依赖复用。
它**不是**完整站点，也不绑定任何 ORM——持久化由消费方实现 `AdminBackend` 注入。

- C# 参考工程：`G:\Code\Pek.FrameWork\Pek.DsMallUI`
- 远程仓库：`https://github.com/RemoteAll/Pek.DsMallUI.git`（remote 名 `origin`，分支 `main`）

## 依赖约定（不要重复造轮子）

| 能力 | 复用来源 | 说明 |
|---|---|---|
| HTTP 服务/连接层 | `dhrust::net::http`（feature `net`） | 不自己起 hyper/监听器 |
| 路由与上下文 | `dhrust::net::router` | `{name}` 单段占位；无 `{**path}` |
| 控制器/参数绑定/JSON 信封 | `dhrust::net::controller` | 对齐 C# `MapController<T>` 与 `arg()` 绑定顺序 |
| 视图引擎 | `dhrust::razor` + `net::controller::ViewRenderer`（feature `razor`） | **Razor 子集**，双端可渲染 |
| Area 视图约定 | `dhrust::net::controller::ViewRenderer::with_fallback_area`（2026-11-10 补齐） | 逐段 Pascal + 默认区域回退 |
| 静态文件（ETag/304/穿越防护） | `dhrust::net::static_files` | 上传目录下载复用它 |
| 口令散列 | `dhrust::sign::password` | 与 C# `SaltPasswordProvider` 字节级互通 |
| 日志 | `dhrust::logs` | 失败必须含「做什么/为什么/上下文」 |
| 出网（抓图） | `dhrust::net::http_client` | HTTPS 需 `http-tls`（本 crate 的 `crawler` 特性） |
| 文件名清洗 | `dhrust::io::sanitize_file_name` | 不自己写非法字符表 |
| 时间片 | `chrono`（与 `dhrust` 同版本） | 路径模板 `{yyyy}` 等 |

**本 crate 自己实现的只有两处**（且有明确理由）：

1. `src/multipart.rs`：`dhrust::net::router` 把 `multipart/form-data` 解析列为待办，生态内无实现；
   **若出现第二个消费方，必须上收进 `dhrust::net`**，本 crate 只留薄壳。
2. UEditor 协议（`src/ueditor/**`）：C# 侧实现是注释状态，需按官方协议补齐。

## 硬约束

- **`DH.RustBase` 按需下沉、不各写一套**：通用 Web 约定（控制器/路由/视图/静态资源）一律先查公共库；
  确实要动公共库时**先问**，并连带回归其它消费方（至少 `Pek.RCard`、`Pek.RPanlServer` `cargo check`）。
  2026-11-10 已按用户要求把 Area 能力下沉到 `dhrust`（`ViewRenderer::with_fallback_area`，
  DH 提交 `3932333`）；本 crate 的 `views.rs` 只保留内置视图落地与模板默认键，不再自己解析区域。
- **Razor 子集红线**：视图里不得使用 `@using`/`@inject`/`@helper`/自由方法调用；
  **不得写 `Layout = null`**（引擎要求字符串，不声明 Layout 即独立页面）；
  **模型缺键即渲染失败**（`Views::render*` 会先补内置模板默认键，新增视图字段要同步补默认值）。
- **非 ASCII 实体**：Razor 子集等价 `HtmlEncoder.Default`，中文输出为 `&#x..;`；
  测试断言中文先用 `crate::decode_html_entities` 还原。
- **静态资源不入库**：`wwwroot/static`（824 文件 / 24.6 MB 第三方前端资产）由
  `tools/sync-wwwroot.ps1` 从 C# 工程同步；`wwwroot/ueconfig.json` 由 `materialize_defaults()` 物化。
- **上传安全**：扩展名白名单 + 体积上限 + 路径必须落在上传根内（三重校验），
  抓图只允许 `http`/`https`（防 SSRF）。
- **口令只走 `dhrust::sign::password`**；不要自写散列、不要把明文落盘。
- 公开 API 的文档注释必须写明与 C# 参考实现的对应关系与差异。

## 每轮收尾（务必）

① `docs/决策记录.md` 新增一节（**含用户原话**）；② 运行 `tools/continuity/sync-memory.ps1` 同步记忆；
③ 跑 `tools/regression.ps1`（含起服务冒烟）并如实汇报；④ 提交并推送（中文提交信息，`git commit -F` UTF-8 文件）。
