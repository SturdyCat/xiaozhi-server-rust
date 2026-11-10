//! 小智管理台 Tauri 2 桌面壳。
//!
//! 这里只负责：创建窗口（内嵌 SPA 或运行时指定的服务器页面）、注册窄命令。
//! 业务逻辑一律留在前端 React SPA 中，不往 Rust 侧搬。

mod commands;

use tauri::{WebviewUrl, WebviewWindowBuilder};

/// 主窗口 label，必须与 capabilities/default.json 的 `windows` 保持一致。
const MAIN_WINDOW_LABEL: &str = "main";

/// 内嵌 SPA 入口，路径相对 `build.frontendDist`（../dist）。
const EMBEDDED_ENTRY: &str = "index.html";

/// 运行时开关：设置后改为加载服务器页面（远程 URL 模式）。
const REMOTE_URL_ENV: &str = "XIAOZHI_ADMIN_URL";

/// 把运行时配置解析为外部 URL。
///
/// tauri-utils 的 `WebviewUrl::External` 要求「Must use either the http or https
/// schemes」；通过 Rust 直接构造枚举会绕过 Deserialize 的分类，因此这里必须
/// 自己做 scheme 白名单校验，避免 `ftp://` / `file://` / `data:` 被交给 wry。
/// 错误信息区分「URL 语法错误」与「scheme 不支持」，便于定位配置问题。
fn parse_external_url(raw: &str) -> Result<WebviewUrl, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(format!("{REMOTE_URL_ENV} 不能为空"));
    }

    let url: url::Url = trimmed
        .parse()
        .map_err(|error| format!("{REMOTE_URL_ENV} 不是合法 URL（语法错误）：{error}"))?;

    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!(
            "{REMOTE_URL_ENV} 仅支持 http/https scheme，当前为 `{}`",
            url.scheme()
        ));
    }

    Ok(WebviewUrl::External(url))
}

/// 解析窗口地址。
///
/// - 默认：`WebviewUrl::App("index.html")` → 内嵌 SPA；开发模式下 Tauri 会按
///   `build.devUrl`（http://127.0.0.1:5173）加载 Vite dev server。
/// - 设置 `XIAOZHI_ADMIN_URL=http://127.0.0.1:8000`：`WebviewUrl::External`，
///   窗口直接加载 Rust server 同源页面，`/api` 相对路径与 `/em/` iframe 天然可用。
///
/// 远程 URL 模式默认不授予 capability 的 remote.urls 权限，模板见 AGENTS.md。
fn resolve_window_url() -> Result<WebviewUrl, String> {
    match std::env::var(REMOTE_URL_ENV) {
        Ok(raw) if !raw.trim().is_empty() => parse_external_url(&raw),
        _ => Ok(WebviewUrl::App(EMBEDDED_ENTRY.into())),
    }
}

/// 应用入口，被 `src/main.rs` 调用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![commands::app_capabilities])
        .setup(|app| {
            let url = resolve_window_url()?;
            WebviewWindowBuilder::new(app, MAIN_WINDOW_LABEL, url)
                .title("小智管理台")
                .inner_size(1280.0, 800.0)
                .min_inner_size(960.0, 640.0)
                .build()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("启动 Tauri 应用失败");
}

#[cfg(test)]
mod tests {
    use super::parse_external_url;
    use tauri::WebviewUrl;

    /// 断言解析成功且返回 External，scheme 与期望一致。
    fn assert_external(raw: &str, expected_scheme: &str) {
        match parse_external_url(raw) {
            Ok(WebviewUrl::External(url)) => {
                assert_eq!(url.scheme(), expected_scheme, "输入: {raw:?}");
            }
            other => panic!("期望 WebviewUrl::External，输入 {raw:?}，实际为 {other:?}"),
        }
    }

    /// 断言解析被拒绝并返回 Err。
    fn assert_rejected(raw: &str) {
        let result = parse_external_url(raw);
        assert!(result.is_err(), "应拒绝输入 {raw:?}，实际为 {result:?}");
    }

    #[test]
    fn accepts_http_and_https_urls() {
        assert_external("http://127.0.0.1:8000", "http");
        assert_external("https://example.com", "https");
        assert_external("  https://example.com/admin/  ", "https");
    }

    #[test]
    fn rejects_non_http_schemes() {
        for raw in [
            "ftp://127.0.0.1:8000",
            "file:///etc/passwd",
            "data:text/plain,x",
            "ws://127.0.0.1:8000",
        ] {
            assert_rejected(raw);
        }
    }

    #[test]
    fn rejects_empty_and_invalid_urls() {
        for raw in ["", "   ", "not a url", "://missing-scheme", "http://"] {
            assert_rejected(raw);
        }
    }

    #[test]
    fn distinguishes_syntax_error_from_unsupported_scheme() {
        let syntax_error = parse_external_url("not a url").unwrap_err();
        assert!(
            syntax_error.contains("语法错误"),
            "实际错误信息: {syntax_error}"
        );

        let scheme_error = parse_external_url("ftp://127.0.0.1:8000").unwrap_err();
        assert!(
            scheme_error.contains("http/https scheme"),
            "实际错误信息: {scheme_error}"
        );
    }
}
