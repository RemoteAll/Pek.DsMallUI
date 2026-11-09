//! UEditor 前后端通信配置（`ueconfig.json`）加载。
//!
//! 原始文件由富文本编辑器官方提供、**带 `/* ... */` 块注释**（见 `assets/ueconfig.json`）。
//! C# 侧 `UeditorConfig.Items` 用正则 `/\*.*?\*/` 去掉块注释后交给 `JsonHelper.DecodeJson`
//! 解析；Rust 侧不能直接 `serde_json::from_str`（会报错），故这里提供等价且有界的清洗：
//! - 去 UTF-8 BOM；
//! - 去 `/* ... */` 块注释与 `//` 行注释（**字符串内的注释符号原样保留**）；
//! - 去对象/数组的尾随逗号（容忍手改配置常见的多余逗号）。
//!
//! 与 `dhrust::json::parse_lax` 的差异：后者只处理 `//` 行注释且做了更多宽松转换，
//! 而 UEditor 要求配置**原样回吐**给前端（`action=config`），故这里保持严格 JSON 语义。

use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Map, Value};

/// UEditor 配置。
#[derive(Debug, Clone)]
pub struct UeditorConfig {
    items: Map<String, Value>,
}

impl UeditorConfig {
    /// 使用 crate 内置配置（与 C# 参考工程 `Pek.DsMallUI/ueconfig.json` 同一份）。
    pub fn embedded() -> UeditorConfig {
        Self::parse(include_str!("../../assets/ueconfig.json"))
            .expect("内置 ueconfig.json 必须可解析（构建期资产，解析失败即代码缺陷）")
    }

    /// 从 JSON 文本解析（容忍块注释/行注释/尾逗号）。
    pub fn parse(raw: &str) -> Result<UeditorConfig, String> {
        let cleaned = strip_comments(raw);
        let value: Value = serde_json::from_str(&cleaned)
            .map_err(|e| format!("ueconfig.json 解析失败（已去注释）：{e}"))?;
        let items = value
            .as_object()
            .ok_or_else(|| "ueconfig.json 顶层必须是 JSON 对象".to_string())?
            .clone();
        Ok(UeditorConfig { items })
    }

    /// 从文件解析（UTF-8）。
    pub fn from_file(path: impl AsRef<Path>) -> Result<UeditorConfig, String> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path)
            .map_err(|e| format!("读取 ueconfig 失败（{}）：{e}", path.display()))?;
        Self::parse(&raw).map_err(|e| format!("{}：{e}", path.display()))
    }

    /// 取字符串配置（缺失或非字符串返回 `None`）。
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.items.get(key).and_then(Value::as_str)
    }

    /// 取整数配置（兼容数字字符串，如手写成 `"20480000"`）。
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        match self.items.get(key)? {
            Value::Number(n) => n.as_i64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    /// 取布尔配置（`true/false`、`1/0`、`yes/no` 均可）。
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        match self.items.get(key)? {
            Value::Bool(flag) => Some(*flag),
            Value::Number(n) => n.as_i64().map(|v| v != 0),
            Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" | "on" => Some(true),
                "false" | "0" | "no" | "off" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    /// 取字符串数组配置，并**归一化为带前导点的扩展名小写集合**。
    ///
    /// `ueconfig.json` 里 `imageAllowFiles` 存在 `"webp"`（缺前导点）这类脏值，
    /// 归一化后比较才不会漏判；空数组表示「未配置 → 拒绝所有类型」（与 C# 同口径）。
    pub fn get_string_list(&self, key: &str) -> Vec<String> {
        let Some(value) = self.items.get(key) else {
            return Vec::new();
        };
        let Some(array) = value.as_array() else {
            return Vec::new();
        };
        let mut seen = BTreeSet::new();
        let mut out = Vec::new();
        for item in array {
            let Some(text) = item.as_str() else { continue };
            let ext = normalize_extension(text);
            if ext.is_empty() {
                continue;
            }
            if seen.insert(ext.clone()) {
                out.push(ext);
            }
        }
        out
    }

    /// 配置对象（只读）。
    pub fn items(&self) -> &Map<String, Value> {
        &self.items
    }

    /// 配置回吐为紧凑 JSON（`action=config` 的响应体）。
    pub fn to_json(&self) -> String {
        Value::Object(self.items.clone()).to_string()
    }
}

impl Default for UeditorConfig {
    fn default() -> Self {
        Self::embedded()
    }
}

/// 标准 `FromStr`（与 [`UeditorConfig::parse`] 同一语义）。
impl std::str::FromStr for UeditorConfig {
    type Err = String;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::parse(raw)
    }
}

/// 扩展名归一化：去空白与引号、补前导 `.`、统一小写。
pub fn normalize_extension(raw: &str) -> String {
    let text = raw.trim().trim_matches('"').trim_matches('\'').trim();
    if text.is_empty() || text == "." {
        return String::new();
    }
    let text = text.to_ascii_lowercase();
    if text.starts_with('.') {
        text
    } else {
        format!(".{text}")
    }
}

/// 清洗 JSON 文本：去 BOM、块注释、行注释、尾随逗号（字符串内一律原样）。
pub fn strip_comments(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let start = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        3
    } else {
        0
    };

    // 全程按字节操作：注释符号都是 ASCII，多字节内容原样搬运，最后再解 UTF-8
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = start;
    let mut in_string = false;
    let mut escaped = false;

    while i < bytes.len() {
        let byte = bytes[i];

        if in_string {
            out.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if byte == b'"' {
            in_string = true;
            out.push(b'"');
            i += 1;
            continue;
        }

        if byte == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }

        if byte == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        out.push(byte);
        i += 1;
    }

    remove_trailing_commas(&String::from_utf8_lossy(&out))
}

/// 去掉 `,]` / `,}` 前的逗号（字符串内不动）。
fn remove_trailing_commas(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut in_string = false;
    let mut escaped = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if in_string {
            out.push(byte);
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        if byte == b'"' {
            in_string = true;
            out.push(b'"');
            continue;
        }
        if byte == b',' {
            // 向后看第一个非空白字符
            let mut j = index + 1;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'}' || bytes[j] == b']') {
                continue; // 丢弃这个逗号
            }
        }
        out.push(byte);
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_config_is_parseable_and_complete() {
        let config = UeditorConfig::embedded();
        assert_eq!(config.get_str("imageActionName"), Some("uploadimage"));
        assert_eq!(config.get_str("fileActionName"), Some("uploadfile"));
        assert_eq!(config.get_str("imageFieldName"), Some("upfile"));
        assert!(config.get_i64("imageMaxSize").unwrap_or(0) > 0);
        assert!(config.to_json().starts_with('{'));
    }

    #[test]
    fn strips_block_and_line_comments_but_keeps_strings() {
        let raw = r#"{
            /* 注释 */
            "url": "http://a/b//c", // 行注释
            "list": [1, 2,] /* 尾逗号 */
        }"#;
        let cleaned = strip_comments(raw);
        assert!(!cleaned.contains("注释"));
        let value: Value = serde_json::from_str(&cleaned).expect("清洗后应可解析");
        assert_eq!(value["url"], "http://a/b//c");
        assert_eq!(value["list"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn normalizes_extension_lists() {
        let config = UeditorConfig::parse(r#"{"imageAllowFiles":[".PNG","jpg"," webp ",""]}"#).unwrap();
        let exts = config.get_string_list("imageAllowFiles");
        assert_eq!(exts, vec![".png", ".jpg", ".webp"]);
    }

    #[test]
    fn missing_keys_are_lenient() {
        let config = UeditorConfig::parse("{}").unwrap();
        assert!(config.get_str("nope").is_none());
        assert!(config.get_i64("nope").is_none());
        assert!(config.get_string_list("nope").is_empty());
    }

    #[test]
    fn bom_is_tolerated() {
        let raw = "\u{feff}{\"a\":1}";
        let config = UeditorConfig::parse(raw).unwrap();
        assert_eq!(config.get_i64("a"), Some(1));
    }
}
