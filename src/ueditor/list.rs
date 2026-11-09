//! UEditor 文件管理器（`listimage` / `listfile`）：按目录递归列出、按扩展名过滤、分页。
//!
//! 对齐 C# `Pek.DsMallUI/UEditor/ListFileManager.cs` 的结果结构
//! （`{state, list:[{url}], start, size, total}`）；C# 版用
//! `Directory.GetFiles(..., SearchOption.AllDirectories)`，这里用同样的递归 + 深度上限
//! （防配置成 `/` 时扫全盘）。

use std::path::{Path, PathBuf};

/// 递归遍历深度上限（UEditor 目录一般是 `/images/2026/11/09`，5 层足够）。
const MAX_DEPTH: usize = 8;

/// 分页结果。
#[derive(Debug, Clone, Default)]
pub struct FileList {
    /// 全量匹配数
    pub total: usize,
    /// 本页文件 URL（已带 `public_prefix`）
    pub items: Vec<String>,
    /// 起始偏移（回显）
    pub start: usize,
    /// 请求页大小（回显）
    pub size: usize,
}

/// 列出目录下的文件。
///
/// - `root`：物理根目录（如 `wwwroot/upload`）；
/// - `relative`：`ueconfig` 里的 `imageManagerListPath`（如 `/images`）；
/// - `extensions`：允许的扩展名（小写、带点）；
/// - `public_prefix`：URL 前缀（如 `/upload`）。
pub fn list_files(
    root: &Path,
    relative: &str,
    extensions: &[String],
    public_prefix: &str,
    start: usize,
    size: usize,
) -> Result<FileList, String> {
    let relative = relative.trim().trim_start_matches('/');
    let dir = root.join(relative);
    if !dir.is_dir() {
        return Err(format!("目录不存在：{}", dir.display()));
    }

    let mut found = Vec::new();
    walk(&dir, MAX_DEPTH, &mut found)?;
    found.sort();

    let prefix = public_prefix.trim_end_matches('/');
    let matched: Vec<String> = found
        .into_iter()
        .filter(|path| {
            let ext = super::path::extension_of(&path.to_string_lossy()).to_ascii_lowercase();
            extensions.contains(&ext)
        })
        .map(|path| {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            format!("{prefix}/{rel}")
        })
        .collect();

    let total = matched.len();
    let items = matched
        .into_iter()
        .skip(start)
        .take(size.max(1))
        .collect();

    Ok(FileList {
        total,
        items,
        start,
        size,
    })
}

/// 递归收集文件路径。
fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if depth == 0 {
        return Ok(());
    }
    let entries = std::fs::read_dir(dir).map_err(|e| format!("读取目录失败（{}）：{e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("遍历目录失败（{}）：{e}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|e| format!("读取文件类型失败（{}）：{e}", path.display()))?;
        if file_type.is_dir() {
            walk(&path, depth - 1, out)?;
        } else if file_type.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_and_pages() {
        let root = std::env::temp_dir().join(format!("dsmallui-list-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let images = root.join("images/2026");
        std::fs::create_dir_all(&images).unwrap();
        for name in ["a.png", "b.jpg", "c.txt", "d.PNG"] {
            std::fs::write(images.join(name), b"x").unwrap();
        }

        let exts = vec![".png".to_string(), ".jpg".to_string()];
        let all = list_files(&root, "/images", &exts, "/upload", 0, 10).unwrap();
        assert_eq!(all.total, 3, "{:?}", all.items);
        assert!(all.items.iter().all(|u| u.starts_with("/upload/images/")));

        let page = list_files(&root, "/images", &exts, "/upload", 2, 1).unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.items.len(), 1);

        let missing = list_files(&root, "/nope", &exts, "/upload", 0, 10).unwrap_err();
        assert!(missing.contains("目录不存在"), "{missing}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
