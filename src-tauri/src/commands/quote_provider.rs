use crate::db::Database;
use crate::models::quote_provider::QuoteProviderConfig;
use crate::services::quote_service::QuoteServiceState;
use crate::services::{quote_provider_service, quote_service};
use tauri::{Manager, State};

#[tauri::command(rename_all = "camelCase")]
pub async fn get_quote_provider_config(
    db: State<'_, Database>,
) -> Result<QuoteProviderConfig, String> {
    quote_provider_service::get_quote_provider_config(&db)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn update_quote_provider_config(
    db: State<'_, Database>,
    quote_state: State<'_, QuoteServiceState>,
    config: QuoteProviderConfig,
) -> Result<bool, String> {
    // Apply the user-provided Xueqiu cookie and `u` value immediately so that
    // subsequent API requests use them without waiting for a restart.
    quote_service::set_xueqiu_user_cookie(&quote_state, config.xueqiu_cookie.clone());
    quote_service::set_xueqiu_user_u(&quote_state, config.xueqiu_u.clone());

    quote_provider_service::update_quote_provider_config(&db, &config)
}

/// Capture `xq_a_token` and `u` cookies from the embedded Xueqiu login window.
///
/// The frontend opens a separate WebviewWindow labelled `"xueqiu_login"` that
/// loads `https://xueqiu.com/`. After the user completes login inside that
/// window, the frontend calls this command. We read the window's cookie store
/// (which includes HttpOnly cookies – `document.cookie` cannot) and persist
/// both values via the same path as manual entry.
///
/// When `close_window` is `true` (used by the auto-capture-on-close flow), the
/// login window is destroyed from the backend regardless of whether capture
/// succeeded. Closing from the backend avoids the cross-window fragility of
/// `WebviewWindow.destroy()` called from the login window's own renderer after
/// `preventDefault()` has intercepted the close.
///
/// Returns the updated config so the frontend can refresh its inputs.
#[tauri::command(rename_all = "camelCase")]
pub async fn capture_xueqiu_cookies(
    app: tauri::AppHandle,
    db: State<'_, Database>,
    quote_state: State<'_, QuoteServiceState>,
    close_window: Option<bool>,
) -> Result<QuoteProviderConfig, String> {
    let win = app.get_webview_window("xueqiu_login").ok_or_else(|| {
        "未找到雪球登录窗口，请先点击「一键登录雪球」按钮打开登录窗口。".to_string()
    })?;

    let cookies = win
        .cookies_for_url(url::Url::parse("https://xueqiu.com").unwrap())
        .map_err(|e| format!("读取雪球登录窗口 Cookie 失败：{}", e))?;

    let mut xq_a_token: Option<String> = None;
    let mut u_value: Option<String> = None;
    for c in cookies {
        let name = c.name();
        if name == "xq_a_token" && !c.value().is_empty() {
            xq_a_token = Some(c.value().to_string());
        } else if name == "u" && !c.value().is_empty() {
            u_value = Some(c.value().to_string());
        }
    }

    // If the caller asked us to close the window, do so from the backend in
    // every outcome (success or failure). This guarantees the window always
    // closes, which the frontend's post-preventDefault destroy() cannot.
    let should_close = close_window.unwrap_or(false);
    let close_window = || {
        if let Some(w) = app.get_webview_window("xueqiu_login") {
            let _ = w.destroy();
        }
    };

    // Both values are required: xq_a_token for the API session, u for the kline API.
    let result = match (xq_a_token, u_value) {
        (Some(token), Some(u)) => {
            // Read the existing config so we preserve provider and cost-adjust choices.
            let mut config = quote_provider_service::get_quote_provider_config(&db)?;
            config.xueqiu_cookie = Some(token);
            config.xueqiu_u = Some(u);

            // Apply to in-memory state immediately, then persist.
            quote_service::set_xueqiu_user_cookie(&quote_state, config.xueqiu_cookie.clone());
            quote_service::set_xueqiu_user_u(&quote_state, config.xueqiu_u.clone());
            quote_service::reset_xueqiu_token(&quote_state);
            quote_provider_service::update_quote_provider_config(&db, &config)?;
            Ok(config)
        }
        (None, _) => Err(
            "未检测到登录态（缺少 xq_a_token）。请在弹出的雪球窗口内完成登录后再点击「我已登录」。"
                .to_string(),
        ),
        (_, None) => Err(
            "未检测到用户 ID（缺少 u）。请确认已在雪球窗口内完成登录（扫码 / 账号密码）。"
                .to_string(),
        ),
    };

    if should_close {
        close_window();
    }
    result
}

/// Parse a raw cookie string pasted by the user and persist `xq_a_token` + `u`.
///
/// Accepts multiple paste formats:
/// - Full `Cookie:` header:   `xq_a_token=xxx; u=123; other=...`
/// - A bare token value:      `6a7dc04b2c6770dc8e...`
/// - cURL copy:               `xq_a_token=xxx; u=123`
///
/// Only `xq_a_token` is strictly required (so the caller can at least use the
/// realtime quote API). `u` is parsed opportunistically; if absent the existing
/// stored `u` value is preserved.
#[tauri::command(rename_all = "camelCase")]
pub async fn parse_xueqiu_cookie_text(
    db: State<'_, Database>,
    quote_state: State<'_, QuoteServiceState>,
    raw: String,
    existing: QuoteProviderConfig,
) -> Result<QuoteProviderConfig, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Cookie 文本为空，请粘贴从浏览器复制的整段 Cookie。".to_string());
    }

    // Case 1: bare token value (no '=' anywhere) – treat as xq_a_token.
    if !trimmed.contains('=') {
        let mut config = existing;
        config.xueqiu_cookie = Some(trimmed.to_string());
        quote_service::set_xueqiu_user_cookie(&quote_state, config.xueqiu_cookie.clone());
        quote_service::set_xueqiu_user_u(&quote_state, config.xueqiu_u.clone());
        quote_service::reset_xueqiu_token(&quote_state);
        quote_provider_service::update_quote_provider_config(&db, &config)?;
        return Ok(config);
    }

    // Case 2: `name=value; name=value; ...` – split and pick the ones we need.
    let mut xq_a_token: Option<String> = None;
    let mut u_value: Option<String> = None;
    for pair in trimmed.split(';') {
        let pair = pair.trim();
        if let Some(eq_idx) = pair.find('=') {
            let name = pair[..eq_idx].trim();
            let value = pair[eq_idx + 1..].trim();
            if name == "xq_a_token" && !value.is_empty() {
                xq_a_token = Some(value.to_string());
            } else if name == "u" && !value.is_empty() {
                u_value = Some(value.to_string());
            }
        }
    }

    let token = xq_a_token.ok_or_else(|| {
        "未在粘贴内容中找到 xq_a_token。请确认复制的是 xueqiu.com 域名下的完整 Cookie。".to_string()
    })?;

    let mut config = existing;
    config.xueqiu_cookie = Some(token);
    // Preserve the previously stored `u` if the pasted text omits it.
    if u_value.is_some() {
        config.xueqiu_u = u_value;
    }

    quote_service::set_xueqiu_user_cookie(&quote_state, config.xueqiu_cookie.clone());
    quote_service::set_xueqiu_user_u(&quote_state, config.xueqiu_u.clone());
    quote_service::reset_xueqiu_token(&quote_state);
    quote_provider_service::update_quote_provider_config(&db, &config)?;
    Ok(config)
}
