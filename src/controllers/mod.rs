//! 控制器（对齐 C#/ASP.NET Core MVC 约定，路由由 `dhrust::net::controller::Controller` 提供）。
//!
//! - [`upload`]：`/fileuploader`（UEditor 入口，C# 绝对路由）、`/Upload/{action}`、
//!   `/Upload/downloads/{**path}`（文件下载，带 Content-Disposition 与条件请求）；
//! - [`admin`]：后台 Area，`/{area}/home|config|login/{action}`（区域名默认 `Admin`，
//!   可随配置改成 `Biz` 等），动作返回视图时按 `Views/{Area}/{Controller}/{Action}.cshtml` 渲染。

pub mod admin;
pub mod upload;
