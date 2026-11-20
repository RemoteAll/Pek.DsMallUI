//! 图形验证码（对齐 C# `DHSetting.Current.CaptChaUrl` 的登录页交互）。
//!
//! C# 参考工程把验证码交给外部地址；Rust 侧不引入图像库，直接生成 **SVG**：
//! - `GET /captcha` → 4 位数字 SVG + `Set-Cookie: dsmallui_captcha=<token>`；
//! - 登录 POST 读取 Cookie，用 `dhrust::verify_code::CodeStore` 一次性校验。
//!
//! 仓库为进程内全局（`OnceLock`），TTL 5 分钟、失败 5 次作废；验证码仅在
//! `AdminArea::with_captcha(true)`（默认开启）时参与登录校验。

use std::sync::OnceLock;

use dhrust::net::http::HttpResponse;
use dhrust::verify_code::CodeStore;

/// 验证码 Cookie 名。
pub const COOKIE_NAME: &str = "dsmallui_captcha";

/// 进程内验证码仓库（key = 浏览器 Cookie 里的随机令牌）。
fn store() -> &'static CodeStore {
    static STORE: OnceLock<CodeStore> = OnceLock::new();
    STORE.get_or_init(|| CodeStore::new(300, 0, 5))
}

/// 生成新验证码，返回 `(令牌, 验证码)`。
pub fn issue() -> (String, String) {
    let token = dhrust::random::hex(16);
    let code = dhrust::verify_code::numeric_code(4);
    // 令牌刚生成，resend 间隔为 0，正常不会失败；失败时仍返回一个可用验证码。
    let _ = store().issue_with(&token, &code);
    (token, code)
}

/// 校验并消费验证码（成功即失效）。
pub fn check(token: &str, code: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    store().check(token, code)
}

/// 从 `Cookie` 头里取验证码令牌。
pub fn token_from_cookie(header: Option<&str>) -> String {
    let Some(header) = header else {
        return String::new();
    };
    for part in header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix(&format!("{COOKIE_NAME}=")) {
            return value.trim().to_string();
        }
    }
    String::new()
}

/// 生成 4 位数字的 SVG 验证码图片（带随机颜色/轻微旋转，无外部依赖）。
pub fn svg(code: &str) -> String {
    let bytes = dhrust::random::bytes(16);
    let colors = ["#1b4f72", "#117a65", "#7d6608", "#7b241c", "#4a235a"];
    let mut out = String::with_capacity(1200);
    out.push_str(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"38\" viewBox=\"0 0 100 38\">",
    );
    out.push_str("<rect width=\"100\" height=\"38\" fill=\"#f7f7f7\"/>");
    // 干扰线
    for i in 0..3 {
        let x1 = 4 + (bytes[i] as usize % 20);
        let y1 = 4 + (bytes[i + 3] as usize % 30);
        let x2 = 70 + (bytes[i + 6] as usize % 26);
        let y2 = 4 + (bytes[i + 9] as usize % 30);
        out.push_str(&format!(
            "<line x1=\"{x1}\" y1=\"{y1}\" x2=\"{x2}\" y2=\"{y2}\" stroke=\"#c9c9c9\" stroke-width=\"1\"/>"
        ));
    }
    for (i, ch) in code.chars().enumerate() {
        let color = colors[bytes[i] as usize % colors.len()];
        let x = 12 + i * 21;
        let y = 27;
        let rotate = (bytes[i + 4] as i32 % 17) - 8;
        out.push_str(&format!(
            "<text x=\"{x}\" y=\"{y}\" font-family=\"Arial,Helvetica,sans-serif\" \
             font-size=\"26\" font-weight=\"700\" fill=\"{color}\" \
             transform=\"rotate({rotate} {x} {y})\">{ch}</text>"
        ));
    }
    out.push_str("</svg>");
    out
}

/// 生成验证码响应（`Cache-Control: no-store` + Cookie）。
pub fn response() -> HttpResponse {
    let (token, code) = issue();
    let cookie = format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age=300"
    );
    HttpResponse::bytes(
        200,
        "image/svg+xml; charset=utf-8",
        svg(&code).into_bytes(),
    )
    .with_header("Cache-Control", "no-store, no-cache, must-revalidate")
    .with_header("Pragma", "no-cache")
    .with_header("Set-Cookie", &cookie)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_contains_each_digit() {
        let html = svg("1234");
        assert!(html.starts_with("<svg"));
        for ch in ["1", "2", "3", "4"] {
            assert!(html.contains(&format!(">{ch}</text>")), "{html}");
        }
    }

    #[test]
    fn cookie_token_parsing() {
        assert_eq!(
            token_from_cookie(Some("a=1; dsmallui_captcha=deadbeef; b=2")),
            "deadbeef"
        );
        assert_eq!(token_from_cookie(None), "");
        assert_eq!(token_from_cookie(Some("a=1")), "");
    }

    #[test]
    fn check_is_one_shot() {
        let (token, code) = issue();
        assert!(check(&token, &code), "正确验证码应通过");
        assert!(!check(&token, &code), "验证码必须一次性");
    }
}
