use tauri::menu::{Menu, PredefinedMenuItem, Submenu};
#[cfg(target_os = "macos")]
use tauri::Manager;
use tauri::AppHandle;

/// Returns `true` when the system locale starts with "zh" (any Chinese variant).
fn is_chinese_locale() -> bool {
    sys_locale::get_locale()
        .unwrap_or_default()
        .starts_with("zh")
}

// ---------------------------------------------------------------------------
// Translatable labels
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct Labels {
    // macOS app-menu
    about_prefix: &'static str,
    hide_prefix: &'static str,
    hide_others: &'static str,
    show_all: &'static str,
    quit_prefix: &'static str,
    // Edit
    edit: &'static str,
    undo: &'static str,
    redo: &'static str,
    cut: &'static str,
    copy: &'static str,
    paste: &'static str,
    select_all: &'static str,
    // View
    view: &'static str,
    fullscreen: &'static str,
    // Window
    window: &'static str,
    minimize: &'static str,
    zoom: &'static str,
    close_window: &'static str,
    // Non-macOS extras
    file: &'static str,
    maximize: &'static str,
}

const LABELS_EN: Labels = Labels {
    about_prefix: "About",
    hide_prefix: "Hide",
    hide_others: "Hide Others",
    show_all: "Show All",
    quit_prefix: "Quit",
    edit: "Edit",
    undo: "Undo",
    redo: "Redo",
    cut: "Cut",
    copy: "Copy",
    paste: "Paste",
    select_all: "Select All",
    view: "View",
    fullscreen: "Toggle Full Screen",
    window: "Window",
    minimize: "Minimize",
    zoom: "Zoom",
    close_window: "Close Window",
    file: "File",
    maximize: "Maximize",
};

const LABELS_ZH: Labels = Labels {
    about_prefix: "关于",
    hide_prefix: "隐藏",
    hide_others: "隐藏其他",
    show_all: "显示全部",
    quit_prefix: "退出",
    edit: "编辑",
    undo: "撤销",
    redo: "重做",
    cut: "剪切",
    copy: "复制",
    paste: "粘贴",
    select_all: "全选",
    view: "视图",
    fullscreen: "切换全屏",
    window: "窗口",
    minimize: "最小化",
    zoom: "缩放",
    close_window: "关闭窗口",
    file: "文件",
    maximize: "最大化",
};

fn get_labels() -> &'static Labels {
    if is_chinese_locale() {
        &LABELS_ZH
    } else {
        &LABELS_EN
    }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn build_menu(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let l = get_labels();

    #[cfg(target_os = "macos")]
    return build_macos_menu(app, l);

    #[cfg(not(target_os = "macos"))]
    return build_non_macos_menu(app, l);
}

// ---------------------------------------------------------------------------
// macOS menu
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn build_macos_menu(app: &AppHandle, l: &Labels) -> tauri::Result<Menu<tauri::Wry>> {
    let app_name = &app.package_info().name;

    // ── App menu ──────────────────────────────────────────────────────
    let about_text = format!("{} {}", l.about_prefix, app_name);
    let hide_text = format!("{} {}", l.hide_prefix, app_name);
    let quit_text = format!("{} {}", l.quit_prefix, app_name);

    let app_menu = Submenu::with_items(
        app,
        app_name,
        true,
        &[
            &PredefinedMenuItem::about(app, Some(&about_text), None)?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::hide(app, Some(&hide_text))?,
            &PredefinedMenuItem::hide_others(app, Some(l.hide_others))?,
            &PredefinedMenuItem::show_all(app, Some(l.show_all))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some(&quit_text))?,
        ],
    )?;

    // ── Edit menu ─────────────────────────────────────────────────────
    let edit_menu = Submenu::with_items(
        app,
        l.edit,
        true,
        &[
            &PredefinedMenuItem::undo(app, Some(l.undo))?,
            &PredefinedMenuItem::redo(app, Some(l.redo))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some(l.cut))?,
            &PredefinedMenuItem::copy(app, Some(l.copy))?,
            &PredefinedMenuItem::paste(app, Some(l.paste))?,
            &PredefinedMenuItem::select_all(app, Some(l.select_all))?,
        ],
    )?;

    // ── View menu ─────────────────────────────────────────────────────
    let view_menu = Submenu::with_items(
        app,
        l.view,
        true,
        &[&PredefinedMenuItem::fullscreen(app, Some(l.fullscreen))?],
    )?;

    // ── Window menu (with multi-screen / display support) ─────────────
    let window_menu = Submenu::with_items(
        app,
        l.window,
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some(l.minimize))?,
            &PredefinedMenuItem::zoom(app, Some(l.zoom))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::close_window(app, Some(l.close_window))?,
        ],
    )?;

    // Register this submenu as the NSApp windowsMenu so macOS will
    // auto-add "Bring All to Front", "Move to [Display]", window list, etc.
    window_menu.set_as_windows_menu_for_nsapp();

    Menu::with_items(app, &[&app_menu, &edit_menu, &view_menu, &window_menu])
}

// ---------------------------------------------------------------------------
// Non-macOS menu (Linux / Windows)
// ---------------------------------------------------------------------------

#[cfg(not(target_os = "macos"))]
fn build_non_macos_menu(app: &AppHandle, l: &Labels) -> tauri::Result<Menu<tauri::Wry>> {
    // ── File menu ─────────────────────────────────────────────────────
    let file_menu = Submenu::with_items(
        app,
        l.file,
        true,
        &[
            &PredefinedMenuItem::close_window(app, Some(l.close_window))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::quit(app, Some(l.quit_prefix))?,
        ],
    )?;

    // ── Edit menu ─────────────────────────────────────────────────────
    let edit_menu = Submenu::with_items(
        app,
        l.edit,
        true,
        &[
            &PredefinedMenuItem::undo(app, Some(l.undo))?,
            &PredefinedMenuItem::redo(app, Some(l.redo))?,
            &PredefinedMenuItem::separator(app)?,
            &PredefinedMenuItem::cut(app, Some(l.cut))?,
            &PredefinedMenuItem::copy(app, Some(l.copy))?,
            &PredefinedMenuItem::paste(app, Some(l.paste))?,
            &PredefinedMenuItem::select_all(app, Some(l.select_all))?,
        ],
    )?;

    // ── Window menu ───────────────────────────────────────────────────
    let window_menu = Submenu::with_items(
        app,
        l.window,
        true,
        &[
            &PredefinedMenuItem::minimize(app, Some(l.minimize))?,
            &PredefinedMenuItem::maximize(app, Some(l.maximize))?,
            &PredefinedMenuItem::close_window(app, Some(l.close_window))?,
        ],
    )?;

    Menu::with_items(app, &[&file_menu, &edit_menu, &window_menu])
}
