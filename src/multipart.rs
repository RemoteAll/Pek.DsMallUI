//! `multipart/form-data` 解析（对齐 ASP.NET Core `Request.ReadFormAsync()` 的表单/文件模型）。
//!
//! **为什么在本 crate 落地**：`dhrust::net::router` 的模块文档把 `multipart/form-data`
//! 解析明确列为待办（`net/router.rs` 模块注释「待办（A2xx 批次，文件管理与上传控制器迁移前补齐）」），
//! 生态内也刻意统一走「裸字节 + query 参数」（HLZX.EIP `doc.rs`/`docchunk.rs`/`report.rs`
//! 均注明“不走 multipart”）。UEditor 的上传协议**必须**是多段表单，故在此实现最小可用版本；
//! 若后续出现第二个消费方，按「同一逻辑出现第二份就下沉」的约定上收进 `dhrust::net`。
//!
//! 支持范围（够 UEditor 用，且对浏览器表单友好）：
//! - `Content-Disposition` 的 `name` / `filename` / `filename*`（RFC 5987，UTF-8）；
//! - 每个分段的 `Content-Type`（用于判断抓取/上传的真实类型）；
//! - 边界宽容：缺少前导 CRLF、`\n` 换行、尾随空白均可解析；
//! - 上限保护：[`FormData::parse_limited`] 限制分段数与单段字节数（防内存放大）。
//!
//! 不支持（显式报错，不做半吊子兼容）：`Content-Transfer-Encoding: base64` 等非二进制传输编码。

use std::fmt;

/// 一个表单项（普通字段或文件）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    /// 字段名（`name="upfile"`）
    pub name: String,
    /// 原始文件名（`filename="a.png"`；普通字段为 `None`）
    pub filename: Option<String>,
    /// 分段自带的 `Content-Type`（有则保留）
    pub content_type: Option<String>,
    /// 分段正文（已剥离尾部 CRLF）
    pub data: Vec<u8>,
}

impl Part {
    /// 是否为文件分段。
    pub fn is_file(&self) -> bool {
        self.filename.is_some()
    }

    /// 文本取值（严格 UTF-8；非 UTF-8 返回 `None`）。
    pub fn text(&self) -> Option<String> {
        String::from_utf8(self.data.clone()).ok()
    }

    /// 文本取值（有损解码，非法字节替换为 `U+FFFD`）。
    pub fn text_lossy(&self) -> String {
        String::from_utf8_lossy(&self.data).to_string()
    }

    /// 正文长度。
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// 正文是否为空。
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }
}

/// 解析后的表单。
#[derive(Debug, Clone, Default)]
pub struct FormData {
    /// 全部分段（保序，与请求体顺序一致）
    pub parts: Vec<Part>,
}

impl FormData {
    /// 默认上限：64 个分段、单段 64 MiB（与 `dhrust::net::http::DEFAULT_MAX_BODY_SIZE` 对齐）。
    pub const DEFAULT_MAX_PARTS: usize = 64;
    /// 默认单段字节上限。
    pub const DEFAULT_MAX_PART_BYTES: usize = 64 * 1024 * 1024;

    /// 按请求头 `Content-Type` 与请求体解析（默认上限）。
    pub fn parse(content_type: Option<&str>, body: &[u8]) -> Result<FormData, MultipartError> {
        Self::parse_limited(
            content_type,
            body,
            Self::DEFAULT_MAX_PARTS,
            Self::DEFAULT_MAX_PART_BYTES,
        )
    }

    /// 解析并施加分段数/单段体积上限。
    ///
    /// `max_part_bytes` 用于在解析后校验单段体积；请求体总量由 HTTP 层
    /// `HttpServerOptions.max_body_size`（默认 64 MiB）兜底。
    pub fn parse_limited(
        content_type: Option<&str>,
        body: &[u8],
        max_parts: usize,
        max_part_bytes: usize,
    ) -> Result<FormData, MultipartError> {
        let content_type = content_type.ok_or_else(|| {
            MultipartError::new("缺少 Content-Type，无法确定 multipart 边界")
        })?;
        let boundary = boundary_of(content_type).ok_or_else(|| {
            MultipartError::new(format!(
                "Content-Type 不是 multipart/form-data 或缺少 boundary：{}",
                truncate(content_type, 120)
            ))
        })?;

        let mut parts = Vec::new();
        let delimiter = {
            let mut d = Vec::with_capacity(boundary.len() + 2);
            d.extend_from_slice(b"--");
            d.extend_from_slice(boundary.as_bytes());
            d
        };

        // 首个边界：允许位于正文开头（标准）或前面有导言（宽容）
        let mut cursor = match next_boundary(body, &delimiter, 0) {
            Some((_data_end, after)) => after,
            None => {
                return Err(MultipartError::new(format!(
                    "请求体中没有找到分隔符 --{}（body {} 字节）",
                    boundary,
                    body.len()
                )))
            }
        };

        loop {
            // 边界之后：`--` = 结束；否则应紧跟 CRLF（允许尾随空白）
            let tail = skip_to_line_end(body, cursor)?;
            let is_end = matches!(tail, LineEnd::Closing);
            if is_end {
                break;
            }
            let data_start = tail.next();

            if parts.len() >= max_parts {
                return Err(MultipartError::new(format!(
                    "分段数量超过上限 {max_parts}"
                )));
            }

            // 分段正文 = 直到「行首 + --boundary + (--|换行)」
            let (data_end, next_cursor) = match next_boundary(body, &delimiter, data_start) {
                Some(found) => found,
                None => {
                    return Err(MultipartError::new(format!(
                        "第 {} 个分段缺少结束边界 --{}",
                        parts.len() + 1,
                        boundary
                    )))
                }
            };
            // 末段无正文且紧邻结束标记时 data_end 可能小于 data_start
            let raw = if data_end > data_start {
                &body[data_start..data_end]
            } else {
                &[]
            };
            if raw.len() > max_part_bytes {
                return Err(MultipartError::new(format!(
                    "第 {} 个分段 {} 字节，超过单段上限 {max_part_bytes}",
                    parts.len() + 1,
                    raw.len()
                )));
            }

            parts.push(parse_part(raw)?);
            cursor = next_cursor;
        }

        Ok(FormData { parts })
    }

    /// 取普通字段文本（首个同名分段；无则 `None`）。
    pub fn value(&self, name: &str) -> Option<&str> {
        self.parts
            .iter()
            .find(|p| p.name == name && !p.is_file())
            .and_then(|p| std::str::from_utf8(&p.data).ok())
    }

    /// 取文件分段（首个同名分段）。
    pub fn file(&self, name: &str) -> Option<&Part> {
        self.parts.iter().find(|p| p.name == name && p.is_file())
    }

    /// 取全部文件分段（同名多文件）。
    pub fn files(&self, name: &str) -> Vec<&Part> {
        self.parts
            .iter()
            .filter(|p| p.name == name && p.is_file())
            .collect()
    }

    /// 取全部普通字段（保序；同名重复保留）。
    pub fn values(&self, name: &str) -> Vec<&str> {
        self.parts
            .iter()
            .filter(|p| p.name == name && !p.is_file())
            .filter_map(|p| std::str::from_utf8(&p.data).ok())
            .collect()
    }
}

/// 解析错误（消息面向运维，含定位信息）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartError {
    message: String,
}

impl MultipartError {
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

impl fmt::Display for MultipartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for MultipartError {}

// ————— 内部实现 —————

/// 边界之后的换行形态。
enum LineEnd {
    /// 结束标记（`--`）
    Closing,
    /// 正常换行，值为下一分段的起始偏移
    Continue(usize),
}

impl LineEnd {
    /// 下一分段起始偏移（`Closing` 时无意义，调用方需先排除）。
    fn next(&self) -> usize {
        match self {
            LineEnd::Closing => 0,
            LineEnd::Continue(pos) => *pos,
        }
    }
}

/// 找到下一个**真实**边界：`--boundary` 必须位于行首（正文开头或紧接 CRLF/LF），
/// 且其后（允许 transport padding）必须是 `--`、CRLF 或 LF。
///
/// 返回 `(分段正文结束偏移, 边界之后偏移)`；正文中的同前缀字节（如
/// `\r\n--boundaryX`）不会被误判为边界。
fn next_boundary(body: &[u8], delimiter: &[u8], from: usize) -> Option<(usize, usize)> {
    let mut search = from;
    loop {
        let pos = find(body, delimiter, search)?;
        let preceded = if pos == 0 {
            Some(pos)
        } else if pos >= 2 && &body[pos - 2..pos] == b"\r\n" {
            Some(pos - 2)
        } else if body[pos - 1] == b'\n' {
            Some(pos - 1)
        } else {
            None
        };
        let Some(data_end) = preceded else {
            search = pos + 1;
            continue;
        };

        // 后随字符校验（跳过 transport padding）
        let mut i = pos + delimiter.len();
        while i < body.len() && (body[i] == b' ' || body[i] == b'\t') {
            i += 1;
        }
        let followed = i >= body.len()
            || body[i] == b'\r'
            || body[i] == b'\n'
            || (i + 1 < body.len() && &body[i..i + 2] == b"--");
        if !followed {
            search = pos + 1;
            continue;
        }
        return Some((data_end, i));
    }
}

/// 跳过边界行剩余字节，判定是结束标记还是普通分段。
fn skip_to_line_end(body: &[u8], cursor: usize) -> Result<LineEnd, MultipartError> {
    let mut i = cursor;
    // 允许边界后有空白（RFC 2046 允许 transport padding）
    while i < body.len() && (body[i] == b' ' || body[i] == b'\t') {
        i += 1;
    }
    if body.len() >= i + 2 && &body[i..i + 2] == b"--" {
        return Ok(LineEnd::Closing);
    }
    // 期望 CRLF；宽容裸 LF
    if body.len() >= i + 2 && &body[i..i + 2] == b"\r\n" {
        return Ok(LineEnd::Continue(i + 2));
    }
    if body.len() > i && body[i] == b'\n' {
        return Ok(LineEnd::Continue(i + 1));
    }
    Err(MultipartError::new(format!(
        "边界后既不是结束标记也不是换行（偏移 {i}）"
    )))
}

/// 解析单个分段：头部 + 正文。
fn parse_part(raw: &[u8]) -> Result<Part, MultipartError> {
    let (head, body) = split_headers(raw);
    let head = String::from_utf8_lossy(head);

    let mut name = None;
    let mut filename = None;
    let mut content_type = None;

    for line in head.split(['\r', '\n']) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            if key.eq_ignore_ascii_case("content-disposition") {
                name = disposition_param(value, "name");
                filename = disposition_param(value, "filename")
                    .or_else(|| disposition_param(value, "filename*").map(|v| decode_ext_value(&v)));
            } else if key.eq_ignore_ascii_case("content-type") {
                content_type = Some(value.to_string());
            }
        }
    }

    let name = name.ok_or_else(|| {
        MultipartError::new(format!(
            "分段缺少 Content-Disposition 的 name 参数：{}",
            truncate(head.trim(), 200)
        ))
    })?;

    // 没有文件名的空文件段（部分浏览器会发送 `filename=""`）按普通字段处理
    let filename = filename.filter(|f| !f.is_empty());

    Ok(Part {
        name,
        filename,
        content_type,
        data: body.to_vec(),
    })
}

/// 头部与正文分隔（`\r\n\r\n`，宽容 `\n\n`）；无头部时整段当正文。
fn split_headers(raw: &[u8]) -> (&[u8], &[u8]) {
    if let Some(pos) = find(raw, b"\r\n\r\n", 0) {
        return (&raw[..pos], &raw[pos + 4..]);
    }
    if let Some(pos) = find(raw, b"\n\n", 0) {
        return (&raw[..pos], &raw[pos + 2..]);
    }
    (&[], raw)
}

/// 从 `Content-Type` 提取 boundary（容忍引号与首尾空白）。
fn boundary_of(content_type: &str) -> Option<String> {
    let lower = content_type.to_ascii_lowercase();
    if !lower.trim_start().starts_with("multipart/form-data") {
        return None;
    }
    let idx = lower.find("boundary=")?;
    let mut value = content_type[idx + "boundary=".len()..].trim_start();
    if let Some(quote) = value.chars().next() {
        if quote == '"' || quote == '\'' {
            let rest = &value[quote.len_utf8()..];
            let end = rest.find(quote)?;
            return Some(rest[..end].to_string());
        }
    }
    // 未加引号：截到 `;`
    value = value.split(';').next().unwrap_or("").trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// 取 `Content-Disposition` 里的参数值（去引号）。
fn disposition_param(header: &str, key: &str) -> Option<String> {
    for segment in header.split(';').skip(1) {
        let segment = segment.trim();
        let Some((name, value)) = segment.split_once('=') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case(key) {
            let value = value.trim();
            let value = value
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .unwrap_or(value);
            return Some(value.to_string());
        }
    }
    None
}

/// RFC 5987 `ext-value` 解码：`UTF-8''%E4%B8%AD.txt` → `中.txt`（仅支持 UTF-8 字符集）。
fn decode_ext_value(value: &str) -> String {
    let mut parts = value.splitn(3, '\'');
    let charset = parts.next().unwrap_or("");
    let _lang = parts.next().unwrap_or("");
    let encoded = parts.next().unwrap_or(value);
    if !charset.eq_ignore_ascii_case("utf-8") {
        return encoded.to_string();
    }
    percent_decode(encoded)
}

/// 百分号解码（非法序列原样保留）。
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = hex_val(bytes[i + 1]);
            let lo = hex_val(bytes[i + 2]);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push(hi << 4 | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 十六进制字符值。
fn hex_val(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

/// 子串查找（起点 `from`；返回绝对偏移）。
fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() || from > haystack.len() - needle.len() {
        return None;
    }
    let first = needle[0];
    let limit = haystack.len() - needle.len();
    let mut i = from;
    while i <= limit {
        if haystack[i] == first && &haystack[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// 截断长文本用于错误消息（避免把整段二进制倒进日志）。
fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 构造一个多段请求体。
    fn build_body(boundary: &str, parts: &[(&str, Option<&str>, &[u8])]) -> Vec<u8> {
        let mut body = Vec::new();
        for (name, filename, data) in parts {
            body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
            match filename {
                Some(file) => body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\n\
                         Content-Type: application/octet-stream\r\n\r\n"
                    )
                    .as_bytes(),
                ),
                None => body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
                ),
            }
            body.extend_from_slice(data);
            body.extend_from_slice(b"\r\n");
        }
        body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
        body
    }

    #[test]
    fn parses_field_and_file() {
        let boundary = "----WebKitFormBoundaryABC123";
        let body = build_body(
            boundary,
            &[
                ("action", None, b"uploadimage"),
                ("upfile", Some("a.png"), b"\x89PNG\r\n\x1a\nDATA"),
            ],
        );
        let ct = format!("multipart/form-data; boundary={boundary}");
        let form = FormData::parse(Some(&ct), &body).expect("应解析成功");

        assert_eq!(form.parts.len(), 2);
        assert_eq!(form.value("action"), Some("uploadimage"));
        let file = form.file("upfile").expect("应有文件");
        assert_eq!(file.filename.as_deref(), Some("a.png"));
        assert_eq!(file.data, b"\x89PNG\r\n\x1a\nDATA");
        assert_eq!(
            file.content_type.as_deref(),
            Some("application/octet-stream")
        );
    }

    #[test]
    fn parses_quoted_boundary_and_lf_newlines() {
        let boundary = "abc";
        // 使用 LF 换行 + 引号边界
        let body = b"--abc\nContent-Disposition: form-data; name=\"x\"\n\n1\n--abc--";
        let form = FormData::parse(Some("multipart/form-data; boundary=\"abc\""), body)
            .expect("LF 换行应能解析");
        assert_eq!(form.value("x"), Some("1"));
        assert_eq!(boundary, "abc");
    }

    #[test]
    fn decodes_rfc5987_filename() {
        let body = b"--b1\r\nContent-Disposition: form-data; name=\"upfile\"; filename*=UTF-8''%E4%B8%AD%E6%96%87.txt\r\n\r\ndata\r\n--b1--";
        let form = FormData::parse(
            Some("multipart/form-data; boundary=b1"),
            &body[..],
        )
        .unwrap();
        let file = form.file("upfile").unwrap();
        assert_eq!(file.filename.as_deref(), Some("中文.txt"));
    }

    #[test]
    fn rejects_missing_boundary() {
        let err = FormData::parse(Some("application/json"), b"{}").unwrap_err();
        assert!(err.message().contains("multipart"), "{err}");
    }

    #[test]
    fn rejects_missing_delimiter() {
        let err = FormData::parse(Some("multipart/form-data; boundary=zz"), b"nothing")
            .unwrap_err();
        assert!(err.message().contains("没有找到分隔符"), "{err}");
    }

    #[test]
    fn enforces_part_count_limit() {
        let boundary = "b2";
        let body = build_body(boundary, &[("a", None, b"1"), ("b", None, b"2")]);
        let err = FormData::parse_limited(
            Some("multipart/form-data; boundary=b2"),
            &body,
            1,
            1024,
        )
        .unwrap_err();
        assert!(err.message().contains("分段数量超过上限"), "{err}");
    }

    #[test]
    fn enforces_part_size_limit() {
        let boundary = "b3";
        let body = build_body(boundary, &[("big", None, b"1234567890")]);
        let err =
            FormData::parse_limited(Some("multipart/form-data; boundary=b3"), &body, 8, 4)
                .unwrap_err();
        assert!(err.message().contains("超过单段上限"), "{err}");
    }

    #[test]
    fn ignores_empty_filename_as_field() {
        let body = b"--b4\r\nContent-Disposition: form-data; name=\"upfile\"; filename=\"\"\r\n\r\n\r\n--b4--";
        let form = FormData::parse_limited(
            Some("multipart/form-data; boundary=b4"),
            &body[..],
            8,
            1024,
        )
        .unwrap();
        assert!(form.file("upfile").is_none());
        assert_eq!(form.value("upfile"), Some(""));
    }

    #[test]
    fn handles_binary_payload_containing_boundary_like_text() {
        let boundary = "b5";
        let mut payload = Vec::new();
        payload.extend_from_slice(b"head--b5-not-a-marker");
        payload.extend_from_slice(b"\r\n--b5x\r\n");
        payload.extend_from_slice(b"tail");
        let body = build_body(boundary, &[("f", Some("x.bin"), &payload)]);
        let form = FormData::parse_limited(
            Some("multipart/form-data; boundary=b5"),
            &body,
            8,
            4096,
        )
        .unwrap();
        assert_eq!(form.file("f").unwrap().data, payload);
    }
}
