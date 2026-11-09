//! 后台数据接口（站点设置 / 登录），把「持久化」从 UI 层解耦。
//!
//! C# 参考工程里 `ConfigController.UpdateBase` 直接写 `SiteInfo` / `DHSetting` / `SysConfig`
//! （XCode 实体），Rust 侧不应该把某个具体 ORM 焊死在这个 UI 包里，故定义
//! [`AdminBackend`] 由消费方实现（数据库、配置文件、Pek.RCode 都行）；
//! 内置 [`MemoryBackend`] 仅用于示例与测试。

use std::sync::Mutex;

use serde_json::{json, Map, Value};

/// 登录成功后的用户信息（字段与 C# `IManage` 常用面保持一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminUser {
    /// 主键
    pub id: i64,
    /// 登录名
    pub user_name: String,
    /// 显示名
    pub display_name: String,
    /// 是否管理员（对应 C# `User.Ex1 == 1` 的约定）
    pub is_admin: bool,
}

/// 后台数据接口。
pub trait AdminBackend: Send + Sync {
    /// 读取站点设置（作为配置页的 `@Model`）。
    ///
    /// 约定至少包含 [`default_settings`] 的同名键；缺键会让模板取不到值。
    fn settings(&self) -> Value;

    /// 保存站点设置（`form` 为原始表单键值，键名与 C# 视图字段一致）。
    fn save_settings(&self, form: &[(String, String)]) -> Result<(), String>;

    /// 校验登录。
    ///
    /// 口令校验必须使用 `dhrust::sign::password`（与 C# 字节级互通），不要自写散列。
    fn login(&self, user_name: &str, password: &str) -> Result<AdminUser, String>;

    /// 系统显示名（渲染页面标题用）。
    fn display_name(&self) -> String {
        self.settings()
            .get("displayName")
            .and_then(Value::as_str)
            .filter(|v| !v.trim().is_empty())
            .unwrap_or("管理后台")
            .to_string()
    }
}

/// 站点设置默认值（键为 camelCase，与模板 `@Model.xxx` 对应）。
pub fn default_settings() -> Value {
    json!({
        "siteName": "",
        "displayName": "管理后台",
        "seoTitle": "",
        "seoKey": "",
        "seoDescribe": "",
        "registration": "",
        "copyright": "",
        "customerTel": "",
        "customerEmail": "",
        "urlSuffix": ".html",
        "statistical": "",
        "pageFoot": "",
        "headerCustomHtml": "",
        "footerCustomHtml": "",
        "curDomainUrl": "",
    })
}

/// 表单键（C# 视图字段名）→ 设置键（camelCase）。
fn form_key_to_setting(key: &str) -> Option<&'static str> {
    Some(match key {
        "SiteName" => "siteName",
        "DisplayName" => "displayName",
        "SeoTitle" => "seoTitle",
        "SeoKey" => "seoKey",
        "SeoDescribe" => "seoDescribe",
        "Registration" => "registration",
        "Copyright" => "copyright",
        "CustomerTel" => "customerTel",
        "CustomerEmail" => "customerEmail",
        "UrlSuffix" => "urlSuffix",
        "Statistical" => "statistical",
        "PageFoot" => "pageFoot",
        "HeaderCustomHtml" => "headerCustomHtml",
        "FooterCustomHtml" => "footerCustomHtml",
        "curDomainUrl" => "curDomainUrl",
        _ => return None,
    })
}

/// 把表单键值合并进设置对象（未识别的键忽略；值一律按字符串处理）。
pub fn merge_form(settings: &mut Value, form: &[(String, String)]) {
    let Some(map) = settings.as_object_mut() else {
        return;
    };
    for (key, value) in form {
        if let Some(target) = form_key_to_setting(key) {
            map.insert(target.to_string(), json!(value));
        }
    }
}

/// 内存实现（示例 / 测试专用；重启即丢，生产请实现自己的 [`AdminBackend`]）。
pub struct MemoryBackend {
    state: Mutex<MemoryState>,
}

struct MemoryState {
    settings: Value,
    users: Vec<MemoryUser>,
}

struct MemoryUser {
    id: i64,
    user_name: String,
    display_name: String,
    password: String,
    is_admin: bool,
}

impl MemoryBackend {
    /// 默认账号 `admin` / `admin123`（仅示例；生产环境请务必替换）。
    pub fn new() -> MemoryBackend {
        let backend = MemoryBackend {
            state: Mutex::new(MemoryState {
                settings: default_settings(),
                users: Vec::new(),
            }),
        };
        backend.add_user("admin", "admin123", "管理员", true);
        backend
    }

    /// 添加（或覆盖）一个用户；口令按 `dhrust::sign::password` 存库。
    pub fn add_user(
        &self,
        user_name: &str,
        password: &str,
        display_name: &str,
        is_admin: bool,
    ) -> &Self {
        let stored = dhrust::sign::password::hash(password);
        let mut state = self.state.lock().expect("MemoryBackend 锁中毒");
        let id = state.users.len() as i64 + 1;
        match state
            .users
            .iter_mut()
            .find(|u| u.user_name.eq_ignore_ascii_case(user_name))
        {
            Some(user) => {
                user.password = stored;
                user.display_name = display_name.to_string();
                user.is_admin = is_admin;
            }
            None => state.users.push(MemoryUser {
                id,
                user_name: user_name.to_string(),
                display_name: display_name.to_string(),
                password: stored,
                is_admin,
            }),
        }
        drop(state);
        self
    }

    /// 直接写入某个设置键（示例/测试便捷入口）。
    pub fn set_setting(&self, key: &str, value: impl Into<String>) -> &Self {
        let mut state = self.state.lock().expect("MemoryBackend 锁中毒");
        if let Some(map) = state.settings.as_object_mut() {
            map.insert(key.to_string(), json!(value.into()));
        }
        drop(state);
        self
    }
}

impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl AdminBackend for MemoryBackend {
    fn settings(&self) -> Value {
        let state = self.state.lock().expect("MemoryBackend 锁中毒");
        let mut merged = match state.settings.clone() {
            Value::Object(map) => map,
            _ => Map::new(),
        };
        // 补齐默认键（项目只覆盖部分键时模板也能取到值）
        if let Value::Object(defaults) = default_settings() {
            for (key, value) in defaults {
                merged.entry(key).or_insert(value);
            }
        }
        Value::Object(merged)
    }

    fn save_settings(&self, form: &[(String, String)]) -> Result<(), String> {
        let site_name = form
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("SiteName"))
            .map(|(_, v)| v.trim())
            .unwrap_or("");
        if site_name.is_empty() {
            return Err("网站名称不可为空".to_string());
        }
        let mut state = self.state.lock().expect("MemoryBackend 锁中毒");
        merge_form(&mut state.settings, form);
        // 系统显示名与站点名分开存（对应 C# SysConfig.DisplayName）
        Ok(())
    }

    fn login(&self, user_name: &str, password: &str) -> Result<AdminUser, String> {
        let user_name = user_name.trim();
        if user_name.is_empty() || password.is_empty() {
            return Err("用户名与密码均不可为空".to_string());
        }
        let state = self.state.lock().expect("MemoryBackend 锁中毒");
        let Some(user) = state
            .users
            .iter()
            .find(|u| u.user_name.eq_ignore_ascii_case(user_name))
        else {
            // 不区分「用户不存在」与「密码错误」，避免用户名枚举
            return Err("用户名或密码不正确".to_string());
        };
        let (ok, upgraded) = dhrust::sign::password::verify_and_upgrade(password, &user.password);
        if !ok {
            return Err("用户名或密码不正确".to_string());
        }
        let result = AdminUser {
            id: user.id,
            user_name: user.user_name.clone(),
            display_name: user.display_name.clone(),
            is_admin: user.is_admin,
        };
        drop(state);
        if let Some(upgraded) = upgraded {
            let mut state = self.state.lock().expect("MemoryBackend 锁中毒");
            if let Some(user) = state
                .users
                .iter_mut()
                .find(|u| u.user_name.eq_ignore_ascii_case(user_name))
            {
                user.password = upgraded;
            }
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_settings_has_template_keys() {
        let settings = default_settings();
        for key in [
            "siteName",
            "displayName",
            "seoTitle",
            "seoKey",
            "seoDescribe",
            "registration",
            "copyright",
            "customerTel",
            "customerEmail",
            "urlSuffix",
            "statistical",
            "pageFoot",
            "headerCustomHtml",
            "footerCustomHtml",
            "curDomainUrl",
        ] {
            assert!(settings.get(key).is_some(), "缺少键 {key}");
        }
    }

    #[test]
    fn saves_and_reads_settings() {
        let backend = MemoryBackend::new();
        let form = vec![
            ("SiteName".to_string(), "商城".to_string()),
            ("SeoTitle".to_string(), "标题".to_string()),
            ("Unknown".to_string(), "忽略".to_string()),
        ];
        backend.save_settings(&form).unwrap();
        let settings = backend.settings();
        assert_eq!(settings["siteName"], "商城");
        assert_eq!(settings["seoTitle"], "标题");
        assert!(settings.get("Unknown").is_none());
    }

    #[test]
    fn rejects_empty_site_name() {
        let backend = MemoryBackend::new();
        let err = backend
            .save_settings(&[("SiteName".to_string(), "  ".to_string())])
            .unwrap_err();
        assert!(err.contains("网站名称不可为空"), "{err}");
    }

    #[test]
    fn login_uses_csharp_compatible_hash() {
        let backend = MemoryBackend::new();
        let user = backend.login("admin", "admin123").expect("默认账号应可用");
        assert!(user.is_admin);
        assert!(backend.login("admin", "badpass").is_err());
        assert!(backend.login("nobody", "admin123").is_err());
    }

    #[test]
    fn display_name_falls_back() {
        let backend = MemoryBackend::new();
        assert_eq!(backend.display_name(), "管理后台");
    }
}
