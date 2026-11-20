//! 页面资源登记（对齐 C# `PekHtmlHelper` 的 `AddCssFileParts` / `AddScriptParts` / `AddInline*`）。
//!
//! 本引擎不支持自定义 Html 助手，所以把「登记结果」放进模型，由布局 foreach 输出标签；
//! 语义与 C# 一致：**按 URL 去重、保持登记顺序**。

use std::collections::HashSet;

use serde_json::{json, Value};

/// 一次响应内的资源清单。
#[derive(Clone, Debug, Default)]
pub struct Assets {
    css: Vec<String>,
    head_scripts: Vec<String>,
    footer_scripts: Vec<String>,
    inline_css: Vec<String>,
    inline_footer_scripts: Vec<String>,
    seen: HashSet<String>,
}

impl Assets {
    /// 空清单。
    pub fn new() -> Assets {
        Assets::default()
    }

    fn push_once(&mut self, list: &mut Vec<String>, key: &str) -> bool {
        // 去重键带分类前缀，避免同一 URL 被当成 css 与 js 互相顶掉
        if !self.seen.insert(key.to_string()) {
            return false;
        }
        list.push(key.split_once(':').map(|(_, v)| v).unwrap_or(key).to_string());
        true
    }

    /// 追加样式表（`<link rel="stylesheet">`）。
    pub fn add_css(&mut self, href: impl Into<String>) -> &mut Self {
        let href = href.into();
        if href.trim().is_empty() {
            return self;
        }
        let key = format!("css:{href}");
        let mut list = std::mem::take(&mut self.css);
        self.push_once(&mut list, &key);
        self.css = list;
        self
    }

    /// 追加 `<head>` 内脚本。
    pub fn add_head_script(&mut self, src: impl Into<String>) -> &mut Self {
        let src = src.into();
        if src.trim().is_empty() {
            return self;
        }
        let key = format!("head:{src}");
        let mut list = std::mem::take(&mut self.head_scripts);
        self.push_once(&mut list, &key);
        self.head_scripts = list;
        self
    }

    /// 追加页脚脚本（对齐 C# `ResourceLocation.Footer`）。
    pub fn add_footer_script(&mut self, src: impl Into<String>) -> &mut Self {
        let src = src.into();
        if src.trim().is_empty() {
            return self;
        }
        let key = format!("footer:{src}");
        let mut list = std::mem::take(&mut self.footer_scripts);
        self.push_once(&mut list, &key);
        self.footer_scripts = list;
        self
    }

    /// 追加内联样式片段。
    pub fn add_inline_css(&mut self, code: impl Into<String>) -> &mut Self {
        let code = code.into();
        if code.trim().is_empty() {
            return self;
        }
        let key = format!("icss:{code}");
        let mut list = std::mem::take(&mut self.inline_css);
        self.push_once(&mut list, &key);
        self.inline_css = list;
        self
    }

    /// 追加页脚内联脚本片段。
    pub fn add_inline_footer_script(&mut self, code: impl Into<String>) -> &mut Self {
        let code = code.into();
        if code.trim().is_empty() {
            return self;
        }
        let key = format!("ifooter:{code}");
        let mut list = std::mem::take(&mut self.inline_footer_scripts);
        self.push_once(&mut list, &key);
        self.inline_footer_scripts = list;
        self
    }

    /// 序列化为模板可遍历的对象（字段名与 `.cshtml` 里一致）。
    pub fn to_value(&self) -> Value {
        json!({
            "css": self.css,
            "headScripts": self.head_scripts,
            "footerScripts": self.footer_scripts,
            "inlineCss": self.inline_css,
            "inlineFooterScripts": self.inline_footer_scripts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assets_dedupe_and_keep_order() {
        let mut a = Assets::new();
        a.add_css("/a.css").add_css("/b.css").add_css("/a.css");
        a.add_footer_script("/x.js").add_footer_script("/x.js");
        a.add_inline_css("body{}").add_inline_css("body{}");
        let v = a.to_value();
        assert_eq!(v["css"], json!(["/a.css", "/b.css"]), "应去重且保序");
        assert_eq!(v["footerScripts"], json!(["/x.js"]));
        assert_eq!(v["inlineCss"], json!(["body{}"]));
    }

    #[test]
    fn css_and_script_with_same_url_are_kept_separately() {
        let mut a = Assets::new();
        a.add_css("/same").add_head_script("/same");
        let v = a.to_value();
        assert_eq!(v["css"], json!(["/same"]));
        assert_eq!(v["headScripts"], json!(["/same"]));
    }
}
