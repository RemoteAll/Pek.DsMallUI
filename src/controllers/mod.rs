//! 控制器（对齐 C#/ASP.NET Core MVC 约定，路由由 `dhrust::net::controller::Controller` 提供）。
//!
//! - [`upload`]：`/fileuploader`（UEditor 入口，C# 绝对路由）、`/Upload/{action}`、
//!   `/Upload/downloads/{**path}`（文件下载，带 Content-Disposition 与条件请求）；
//! - [`admin`]：后台 Area，`/admin/config/{action}`、`/admin/login/{action}`，
//!   动作返回视图时按 `Views/{Area}/{Controller}/{Action}.cshtml` 渲染。

pub mod admin;
pub mod upload;
