//! 小智管理台 Tauri 2 桌面壳。
//!
//! 这里只负责：按运行模式创建窗口、注册窄命令、挂载最小插件。
//! 业务逻辑一律留在前端 React SPA 中，不往 Rust 侧搬。

mod commands;

use tauri::{WebviewUrl, WebviewWindowBuilder};

/// 主窗口 label，必须与 capabilities/default.json 的 `windows` 保持一致。
const MAIN_WINDOW_LABEL: &str = "main";

/// 内嵌 SPA 入口，路径相对 `build.frontendDist`（../dist）。
const EMBEDDED_ENTRY: &str = "index.html";

/// 运行时服务器地址开关：设置后改为加载该 http(s) 服务器页面（同源访问 `/api`）。
const REMOTE_URL_ENV: &str = "XIAOZHI_ADMIN_URL";

/// 生产模式默认加载的服务器页面地址。
///
/// 生产/局域网部署需显式设置 `XIAOZHI_ADMIN_URL`，后续可接 mDNS 发现或地址配置页。
/// 服务器页面与 Rust server 同源，`/api`、`/ws`、`/em/` 相对路径天然可用；
/// 内嵌 SPA（`tauri://localhost`）下这些相对路径只会打到资产协议，API 不可达。
const DEFAULT_SERVER_URL: &str = "http://127.0.0.1:8000";

/// 显式内嵌预览开关，仅值精确等于 `EMBEDDED_ENV_ENABLED` 时生效。
const EMBEDDED_ENV: &str = "XIAOZHI_ADMIN_EMBEDDED";
const EMBEDDED_ENV_ENABLED: &str = "1";

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

/// 窗口地址决策（纯函数：不读环境变量，也不依赖编译期 cfg，避免单测间环境变量竞态）。
///
/// 优先级自高而低：
/// 1. `is_dev`：dev 构建恒返回 `WebviewUrl::App("index.html")`。Tauri 的
///    `get_app_url()` 在 `cfg(dev)` 下把 App 映射到 `build.devUrl`
///    （http://127.0.0.1:5173），保持 Vite dev server + proxy + HMR 不变。
/// 2. `embedded_raw` 精确等于 `"1"`：显式内嵌预览，仅用于离线 UI 预览（API 不可达）。
/// 3. `remote_raw` 非空白：走 `parse_external_url()`（http/https 白名单与两种 Err 不变）。
/// 4. 其余：生产默认加载 `DEFAULT_SERVER_URL` 的服务器页面，构造
///    `WebviewUrl::External`，使 `/api`、`/ws`、`/em/` 同源可达。
fn decide_window_url(
    is_dev: bool,
    remote_raw: Option<&str>,
    embedded_raw: Option<&str>,
) -> Result<WebviewUrl, String> {
    if is_dev {
        return Ok(WebviewUrl::App(EMBEDDED_ENTRY.into()));
    }

    if embedded_raw == Some(EMBEDDED_ENV_ENABLED) {
        return Ok(WebviewUrl::App(EMBEDDED_ENTRY.into()));
    }

    match remote_raw {
        Some(raw) if !raw.trim().is_empty() => parse_external_url(raw),
        // 常量本身是合法 http URL；复用同一个解析函数保证 scheme 校验只有一处。
        _ => parse_external_url(DEFAULT_SERVER_URL),
    }
}

/// 解析窗口地址：读取运行时环境变量，dev 由 `cfg(dev)` 编译期判定后交给纯决策函数。
///
/// `cfg(dev)` 由 `tauri_build::build()` 通过 `cfg_alias("dev", is_dev())` 暴露给业务
/// crate（tauri-build-2.7.4/src/lib.rs:714、:304-308 同时发出
/// `cargo:rustc-check-cfg=cfg(dev)`，故不会产生 unexpected_cfg 警告）。
fn resolve_window_url() -> Result<WebviewUrl, String> {
    let remote_raw: Option<String> = std::env::var(REMOTE_URL_ENV).ok();
    let embedded_raw: Option<String> = std::env::var(EMBEDDED_ENV).ok();
    decide_window_url(cfg!(dev), remote_raw.as_deref(), embedded_raw.as_deref())
}

/// 应用入口，被 `src/main.rs` 调用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
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
    use super::{
        decide_window_url, parse_external_url, DEFAULT_SERVER_URL, EMBEDDED_ENTRY,
        EMBEDDED_ENV_ENABLED,
    };
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

    /// 断言决策结果为内嵌 SPA 入口 App("index.html")。
    fn assert_app(result: Result<WebviewUrl, String>) {
        match result {
            Ok(WebviewUrl::App(path)) => assert_eq!(path.to_string_lossy(), EMBEDDED_ENTRY),
            other => panic!("期望 WebviewUrl::App({EMBEDDED_ENTRY:?})，实际为 {other:?}"),
        }
    }

    /// 断言决策结果为 External，且 URL 与期望一致（忽略 url crate 补的尾斜杠）。
    fn assert_external_result(result: Result<WebviewUrl, String>, expected: &str) {
        match result {
            Ok(WebviewUrl::External(url)) => assert_eq!(
                url.as_str().trim_end_matches('/'),
                expected.trim_end_matches('/'),
            ),
            other => panic!("期望 WebviewUrl::External({expected:?})，实际为 {other:?}"),
        }
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

    #[test]
    fn dev_build_always_uses_embedded_entry() {
        // dev 优先于一切运行时开关：即使同时设置了服务器地址与内嵌开关仍是 App。
        assert_app(decide_window_url(true, None, None));
        assert_app(decide_window_url(
            true,
            Some("http://10.0.0.5:8000"),
            Some(EMBEDDED_ENV_ENABLED),
        ));
    }

    #[test]
    fn production_defaults_to_server_page() {
        // 未设置任何环境变量：生产默认加载服务器页面，而不是内嵌 SPA。
        assert_external_result(decide_window_url(false, None, None), DEFAULT_SERVER_URL);
        // 空白值等同于未设置。
        assert_external_result(decide_window_url(false, Some("   "), None), DEFAULT_SERVER_URL);
    }

    #[test]
    fn production_uses_configured_http_and_https_urls() {
        // XIAOZHI_ADMIN_URL 为合法 http/https 时沿用，scheme 白名单不变。
        assert_external_result(
            decide_window_url(false, Some("http://192.168.1.20:8000"), None),
            "http://192.168.1.20:8000",
        );
        assert_external_result(
            decide_window_url(false, Some("https://admin.example.com/"), None),
            "https://admin.example.com/",
        );
    }

    #[test]
    fn embedded_flag_requires_exact_one_and_forces_app() {
        // 仅精确 "1" 生效；显式内嵌开关优先于服务器地址。
        assert_app(decide_window_url(false, None, Some(EMBEDDED_ENV_ENABLED)));
        assert_app(decide_window_url(
            false,
            Some("http://10.0.0.5:8000"),
            Some(EMBEDDED_ENV_ENABLED),
        ));
        // 其它值（含 "true" / "0" / 带空格 / 空串）都不启用，回落到生产默认服务器页面。
        for raw in ["true", "0", "yes", "", " 1 ", "01"] {
            assert_external_result(decide_window_url(false, None, Some(raw)), DEFAULT_SERVER_URL);
        }
    }

    #[test]
    fn production_still_rejects_non_http_schemes() {
        // 生产模式下 XIAOZHI_ADMIN_URL 非 http(s) 依然返回 Err，不被默认地址吞掉。
        for raw in [
            "ftp://127.0.0.1:8000",
            "file:///etc/passwd",
            "data:text/plain,x",
            "not a url",
        ] {
            let result = decide_window_url(false, Some(raw), None);
            assert!(result.is_err(), "应拒绝 {raw:?}，实际为 {result:?}");
        }
    }
}
