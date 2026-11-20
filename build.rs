//! 编译期扫描视图根，生成内置视图清单（对齐 C# RCL：**目录结构即约定**）。
//!
//! 视图根优先 `Areas/`（C# 规范），缺失时回退 `Views/`（平铺项目）。
//! 生成 `$OUT_DIR/embedded_views.rs`：
//! ```ignore
//! pub static EMBEDDED_VIEWS: &[(&str, &str)] = &[
//!     ("Admin/Views/Shared/_Layout.cshtml", include_str!(".../Areas/Admin/Views/Shared/_Layout.cshtml")),
//!     ...
//! ];
//! ```
//! 相对路径与 `ViewConvention::Areas` 的查找路径**逐字节一致**，运行时不需要
//! 任何「分类 / 改名 / 拼路径」逻辑；新增或删除 `.cshtml` 只动目录。
//!
//! **只收集 `.cshtml` 模板文本**；css/js/图片等静态资源不入包（由磁盘静态服务提供）。

use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));

    let root = ["Areas", "Views"]
        .iter()
        .map(|name| manifest.join(name))
        .find(|path| path.is_dir());

    let mut files: Vec<String> = Vec::new();
    if let Some(root) = &root {
        collect(root, root, &mut files);
        println!("cargo:rerun-if-changed={}", root.display());
    }
    files.sort();

    let mut code = String::from("/// 编译期生成：视图根下全部 `.cshtml`（相对路径 → 源码）。\n");
    code.push_str("pub static EMBEDDED_VIEWS: &[(&str, &str)] = &[\n");
    for relative in &files {
        let absolute = root
            .as_ref()
            .expect("有文件必然有根目录")
            .join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        code.push_str(&format!(
            "    ({:?}, include_str!({:?})),\n",
            relative,
            absolute.display().to_string()
        ));
    }
    code.push_str("];\n");

    fs::write(out_dir.join("embedded_views.rs"), code).expect("写入 embedded_views.rs 失败");
}

/// 递归收集 `.cshtml`（跳过隐藏文件与目录）。
fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            collect(root, &path, out);
            continue;
        }
        if !name.to_ascii_lowercase().ends_with(".cshtml") {
            continue;
        }
        if let Ok(relative) = path.strip_prefix(root) {
            let relative = relative
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR_STR, "/");
            println!("cargo:rerun-if-changed={}", path.display());
            out.push(relative);
        }
    }
}
