//! 上传保存路径格式化（对齐 UEditor 官方 `PathFormatter`，C# 参考实现见
//! `Pek.DsMallUI/UEditor/PathFormatter.cs`）。
//!
//! 支持的占位符（与官方文档一致）：
//! `{filename}` 原文件名（不含扩展名）、`{rand:6}` / `{rand6}` 指定位数随机数、
//! `{time}` 毫秒时间戳、`{yyyy}`/`{yy}`/`{mm}`/`{dd}`/`{hh}`/`{ii}`/`{ss}` 时间片。
//!
//! 与 C# 实现的两点差异（均为修复而非改口径，已在 `docs/移植对照.md` 记录）：
//! 1. **文件名清洗走 `dhrust::io::sanitize_file_name`**：C# 直接删除 `\ / : * ? " < > |`，
//!    Rust 侧替换为 `_`，并且额外把 `..` 折叠为 `__`，避免 `{filename}` 造成目录穿越；
//! 2. **`{rand:n}` 生成 n 位数字**：C# 写作 `rand.Next(10^n, 10^(n+1))`，实际产出 **n+1 位**，
//!    与官方 PHP/Java 实现（n 位）不一致，这里按官方口径修正。

use chrono::{DateTime, Datelike, Local, Timelike};

/// 默认命名规则（`pathFormat` 为空时使用）。
pub const DEFAULT_PATH_FORMAT: &str = "{filename}_{rand:6}";

/// 按当前时间格式化保存路径（返回以 `/` 分隔的相对路径，含扩展名）。
pub fn format_path(original_filename: &str, path_format: &str) -> String {
    format_path_at(original_filename, path_format, &Local::now(), random_digits)
}

/// 可注入时间与随机源的格式化（便于测试；内部与 [`format_path`] 同一实现）。
pub(crate) fn format_path_at<F>(
    original_filename: &str,
    path_format: &str,
    now: &DateTime<Local>,
    make_random: F,
) -> String
where
    F: Fn(usize) -> String,
{
    let pattern = if path_format.trim().is_empty() {
        DEFAULT_PATH_FORMAT
    } else {
        path_format
    };

    let sanitized = dhrust::io::sanitize_file_name(original_filename);
    let extension = extension_of(&sanitized);
    let stem = if extension.is_empty() {
        sanitized.as_str()
    } else {
        &sanitized[..sanitized.len() - extension.len()]
    };

    let mut out = String::with_capacity(pattern.len() + stem.len() + 16);
    let mut rest = pattern;
    while let Some(index) = rest.find('{') {
        out.push_str(&rest[..index]);
        let tail = &rest[index..];
        let Some(close) = tail.find('}') else {
            // 无配对 `}`：原样保留
            out.push_str(tail);
            rest = "";
            break;
        };
        let token = &tail[1..close];
        match token {
            "filename" => out.push_str(stem),
            "time" => out.push_str(&now.timestamp_millis().to_string()),
            "yyyy" => out.push_str(&format!("{:04}", now.year())),
            "yy" => out.push_str(&format!("{:02}", now.year().rem_euclid(100))),
            "mm" => out.push_str(&format!("{:02}", now.month())),
            "dd" => out.push_str(&format!("{:02}", now.day())),
            "hh" => out.push_str(&format!("{:02}", now.hour())),
            "ii" => out.push_str(&format!("{:02}", now.minute())),
            "ss" => out.push_str(&format!("{:02}", now.second())),
            other => {
                if let Some(digits) = rand_digits_of(other) {
                    out.push_str(&make_random(digits));
                } else {
                    // 未知占位符原样输出（便于发现拼写错误）
                    out.push_str(&tail[..=close]);
                }
            }
        }
        rest = &tail[close + 1..];
    }
    out.push_str(rest);
    out.push_str(&extension);
    out
}

/// 取扩展名（含 `.`，保持原始大小写）；无扩展名返回空串。
pub fn extension_of(filename: &str) -> String {
    let name = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    match name.rfind('.') {
        Some(pos) if pos > 0 && pos + 1 < name.len() => name[pos..].to_string(),
        _ => String::new(),
    }
}

/// 解析 `rand:6` / `rand6` 形式，返回位数（上限 12 位，防手改配置写出超长随机串）。
fn rand_digits_of(token: &str) -> Option<usize> {
    let rest = token.strip_prefix("rand")?;
    let rest = rest.strip_prefix(':').unwrap_or(rest);
    if rest.is_empty() || !rest.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let digits = rest.parse::<usize>().ok()?;
    Some(digits.clamp(1, 12))
}

/// 默认随机源：n 位十进制数字（首位可为 0，与官方实现一致）。
fn random_digits(digits: usize) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // 不新增依赖：用系统时间 + 进程内计数混合出伪随机数字串（仅用于文件命名，不用于安全用途）
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15)
        ^ (std::process::id() as u64).rotate_left(17);
    let mut out = String::with_capacity(digits);
    for _ in 0..digits {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        out.push(char::from(b'0' + (seed % 10) as u8));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn fixed_time() -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, 9, 8, 5, 3).unwrap()
    }

    #[test]
    fn formats_all_time_tokens() {
        let path = format_path_at(
            "报告 v2.PNG",
            "/images/{yyyy}/{mm}/{dd}/{filename}_{hh}{ii}{ss}",
            &fixed_time(),
            |_| "000".into(),
        );
        assert_eq!(path, "/images/2026/11/09/报告 v2_080503.PNG");
    }

    #[test]
    fn rand_token_is_requested_length() {
        let path = format_path_at("a.txt", "{filename}_{rand:6}", &fixed_time(), |n| {
            assert_eq!(n, 6);
            "123456".into()
        });
        assert_eq!(path, "a_123456.txt");
    }

    #[test]
    fn rand_without_colon_is_supported() {
        let path = format_path_at("a.txt", "{rand3}", &fixed_time(), |n| {
            assert_eq!(n, 3);
            "987".into()
        });
        assert_eq!(path, "987.txt");
    }

    #[test]
    fn default_pattern_when_empty() {
        let path = format_path_at("x.jpg", "", &fixed_time(), |n| "1".repeat(n));
        assert_eq!(path, "x_111111.jpg");
    }

    #[test]
    fn sanitizes_path_traversal_in_filename() {
        let path = format_path_at("..\\..\\web.config", "{filename}.txt", &fixed_time(), |_| {
            String::new()
        });
        assert!(!path.contains(".."), "{path}");
        assert!(!path.contains('/'), "{path}");
        assert!(!path.contains('\\'), "{path}");
    }

    #[test]
    fn extension_of_variants() {
        assert_eq!(extension_of("a.tar.gz"), ".gz");
        assert_eq!(extension_of("a"), "");
        assert_eq!(extension_of(".gitignore"), "");
        assert_eq!(extension_of("dir/a.PNG"), ".PNG");
    }

    #[test]
    fn unknown_token_is_kept_verbatim() {
        let path = format_path_at("a.txt", "{unknown}_{filename}", &fixed_time(), |_| String::new());
        assert_eq!(path, "{unknown}_a.txt");
    }
}
