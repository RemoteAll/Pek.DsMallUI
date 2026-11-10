//! Area（区域）命名与约定（对齐 ASP.NET Core Areas / C# `Pek.NCubeUI.Areas.Admin`）。
//!
//! C# 参考工程的区域名由配置决定（`DHSetting.Current.AdminArea`，默认 `Admin`），
//! 路由是 `/{area}/{controller}/{action}`，视图是 `Areas/{Area}/Views/{Controller}/{Action}.cshtml`。
//! Rust 侧保持同一套两级约定，但**区域名在装配时由消费方决定**（可来自它自己的配置/环境变量）：
//!
//! ```no_run
//! # use pek_dsmallui::DsMallUi;
//! // 默认 Admin；本项目用 Biz → 路由 /biz/config/index、视图 Views/Biz/...
//! let ui = DsMallUi::builder().admin_area("Biz").build();
//! ```
//!
//! 命名规则（严格校验，避免歧义进路由）：
//! - 非空，只允许 ASCII 字母、数字、`_`、`-`，且首字符必须是字母；
//! - 存储/视图目录用 **Pascal 化**（`biz` → `Biz`），URL 段用 **小写**（`/biz/...`）；
//! - 视图解析带**回退**：`Views/{Area}/…` 不存在时用 `Views/Admin/…`
//!   （内置默认视图只在 `Admin` 下维护，改区域名不必复制一份）。

use std::fmt;

/// 默认后台区域名（与 C# `AdminArea.AreaName` 默认值一致）。
pub const DEFAULT_AREA: &str = "Admin";

/// 区域名错误（消息面向运维，含非法输入片段）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AreaError {
    message: String,
}

impl AreaError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// 错误文本。
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for AreaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AreaError {}

/// 规范化区域名：去空白、首字符大写；非法输入返回明确错误。
///
/// - `"Biz"` / `"biz"` / `" BIZ "` → `"Biz"`
/// - `""` / `"1admin"` / `"a/b"` → 错误
pub fn normalize(raw: &str) -> Result<String, AreaError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AreaError::new("区域名不可为空"));
    }
    let mut chars = trimmed.chars();
    let first = chars.next().expect("非空已校验");
    if !first.is_ascii_alphabetic() {
        return Err(AreaError::new(format!(
            "区域名必须以字母开头（实际：{trimmed}）"
        )));
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(AreaError::new(format!(
            "区域名只能包含字母/数字/下划线/连字符（实际：{trimmed}）"
        )));
    }
    let mut out = String::with_capacity(trimmed.len());
    out.push(first.to_ascii_uppercase());
    out.push_str(&trimmed[first.len_utf8()..].to_ascii_lowercase());
    Ok(out)
}

/// 路由段（小写、无前导斜杠）：`"Biz"` → `"biz"`。
pub fn route_segment(area: &str) -> String {
    area.trim().to_ascii_lowercase()
}

/// 路由前缀（带前导斜杠）：`"Biz"` → `"/biz"`。
pub fn route_prefix(area: &str) -> String {
    format!("/{}", route_segment(area))
}

/// 是否默认区域（`Admin`）。
pub fn is_default(area: &str) -> bool {
    area.trim().eq_ignore_ascii_case(DEFAULT_AREA)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_case_and_space() {
        assert_eq!(normalize(" Biz ").unwrap(), "Biz");
        assert_eq!(normalize("biz").unwrap(), "Biz");
        assert_eq!(normalize("BIZ").unwrap(), "Biz");
        assert_eq!(normalize("my_area-2").unwrap(), "My_area-2");
    }

    #[test]
    fn rejects_invalid_names() {
        assert!(normalize("").unwrap_err().message().contains("不可为空"));
        assert!(normalize("  ").unwrap_err().message().contains("不可为空"));
        assert!(normalize("1admin")
            .unwrap_err()
            .message()
            .contains("必须以字母开头"));
        assert!(normalize("a/b")
            .unwrap_err()
            .message()
            .contains("只能包含"));
        assert!(normalize("区域")
            .unwrap_err()
            .message()
            .contains("必须以字母开头"));
    }

    #[test]
    fn builds_route() {
        assert_eq!(route_segment("Biz"), "biz");
        assert_eq!(route_prefix("Biz"), "/biz");
        assert_eq!(route_prefix("Admin"), "/admin");
        assert!(is_default(" admin "));
        assert!(!is_default("Biz"));
    }
}
