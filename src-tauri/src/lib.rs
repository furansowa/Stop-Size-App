mod config;
mod rates;

use config::{Config, RateState};
use rates::Pair;
use std::sync::Mutex;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};

struct AppState(Mutex<Config>);

/// The design canvas the frontend lays the table out on, and the aspect ratio
/// the window is locked to so the artwork never distorts.
const DESIGN_W: f64 = 1500.0;
const DESIGN_H: f64 = 1051.0;

/// Both cached rates, as handed to the frontend.
#[derive(Clone, serde::Serialize)]
struct RatesPayload {
    eurusd: RateState,
    usdjpy: RateState,
}

impl RatesPayload {
    fn of(cfg: &Config) -> Self {
        RatesPayload {
            eurusd: cfg.eurusd.clone(),
            usdjpy: cfg.usdjpy.clone(),
        }
    }
}

fn aspect_height(width: u32) -> u32 {
    ((width as f64) * DESIGN_H / DESIGN_W).round() as u32
}

fn today_string() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn rate_state<'a>(cfg: &'a mut Config, pair: Pair) -> &'a mut RateState {
    match pair {
        Pair::EurUsd => &mut cfg.eurusd,
        Pair::UsdJpy => &mut cfg.usdjpy,
    }
}

/// A pair is due when auto-fetch is on and the cache is not from today.
fn is_stale(state: &RateState) -> bool {
    state.auto_fetch && state.last_updated != today_string()
}

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.0.lock().unwrap().clone()
}

#[tauri::command(rename_all = "snake_case")]
fn save_config(
    mut new_config: Config,
    window: WebviewWindow,
    state: State<AppState>,
) -> Result<(), String> {
    config::migrate(&mut new_config);
    let always_on_top = new_config.window.always_on_top;
    {
        let mut cfg = state.0.lock().unwrap();
        *cfg = new_config;
        config::save(&cfg).map_err(|e| e.to_string())?;
    }
    window.set_always_on_top(always_on_top).map_err(|e| e.to_string())?;
    Ok(())
}

/// Refresh one pair on demand. Returns both rates either way, so a failed fetch
/// simply echoes the cached values back.
#[tauri::command]
async fn fetch_now(state: State<'_, AppState>, pair: String) -> Result<RatesPayload, String> {
    let pair = Pair::parse(&pair).ok_or_else(|| format!("unknown pair: {pair}"))?;
    let fetched = rates::fetch(pair).await;

    let mut cfg = state.0.lock().unwrap();
    if let Some(r) = fetched {
        let slot = rate_state(&mut cfg, pair);
        slot.rate = r;
        slot.last_updated = today_string();
        config::save(&cfg).map_err(|e| e.to_string())?;
    }
    Ok(RatesPayload::of(&cfg))
}

#[tauri::command]
fn start_drag(window: WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}

#[tauri::command]
fn hide_to_tray(window: WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|e| e.to_string())
}

#[tauri::command]
fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[tauri::command(rename_all = "snake_case")]
fn set_always_on_top(
    window: WebviewWindow,
    state: State<AppState>,
    value: bool,
) -> Result<(), String> {
    window.set_always_on_top(value).map_err(|e| e.to_string())?;
    let mut cfg = state.0.lock().unwrap();
    cfg.window.always_on_top = value;
    config::save(&cfg).map_err(|e| e.to_string())?;
    Ok(())
}

/// Refresh every stale pair once, in the background, then tell the frontend.
/// Silent on failure — the widget keeps rendering from the cached rates.
fn spawn_startup_fetch(app: &AppHandle, initial: &Config) {
    let due: Vec<Pair> = [(Pair::EurUsd, &initial.eurusd), (Pair::UsdJpy, &initial.usdjpy)]
        .into_iter()
        .filter(|(_, state)| is_stale(state))
        .map(|(pair, _)| pair)
        .collect();
    if due.is_empty() {
        return;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut updated = false;
        for pair in due {
            let Some(rate) = rates::fetch(pair).await else { continue };
            let Some(state) = app.try_state::<AppState>() else { continue };
            let mut cfg = state.0.lock().unwrap();
            let slot = rate_state(&mut cfg, pair);
            slot.rate = rate;
            slot.last_updated = today_string();
            let _ = config::save(&cfg);
            updated = true;
        }
        if updated {
            if let Some(state) = app.try_state::<AppState>() {
                let payload = RatesPayload::of(&state.0.lock().unwrap());
                let _ = app.emit("rates-updated", payload);
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let initial_config = config::load_or_create();

    tauri::Builder::default()
        .manage(AppState(Mutex::new(initial_config.clone())))
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            fetch_now,
            start_drag,
            hide_to_tray,
            quit_app,
            set_always_on_top
        ])
        .setup(move |app| {
            let window = app.get_webview_window("main").unwrap();

            let w = &initial_config.window;
            let corrected_height = aspect_height(w.width);
            let _ = window.set_position(PhysicalPosition::new(w.x, w.y));
            let _ = window.set_size(PhysicalSize::new(w.width, corrected_height));
            let _ = window.set_always_on_top(w.always_on_top);
            let _ = window.show();

            let restore_item = MenuItem::with_id(app, "restore", "Restore", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&restore_item, &quit_item])?;
            let icon = app.default_window_icon().cloned();

            let mut tray_builder = TrayIconBuilder::new()
                .menu(&tray_menu)
                .tooltip("Stop Size Widget")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "restore" => {
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.set_focus();
                        }
                    }
                });
            if let Some(icon) = icon {
                tray_builder = tray_builder.icon(icon);
            }
            tray_builder.build(app)?;

            let app_handle = app.handle().clone();
            window.on_window_event(move |event| match event {
                tauri::WindowEvent::Moved(pos) => {
                    if let Some(state) = app_handle.try_state::<AppState>() {
                        let mut cfg = state.0.lock().unwrap();
                        cfg.window.x = pos.x;
                        cfg.window.y = pos.y;
                        let _ = config::save(&cfg);
                    }
                }
                tauri::WindowEvent::Resized(size) => {
                    let expected_h = aspect_height(size.width);
                    if size.height.abs_diff(expected_h) > 1 {
                        if let Some(win) = app_handle.get_webview_window("main") {
                            let _ = win.set_size(PhysicalSize::new(size.width, expected_h));
                        }
                        return;
                    }
                    if let Some(state) = app_handle.try_state::<AppState>() {
                        let mut cfg = state.0.lock().unwrap();
                        cfg.window.width = size.width;
                        cfg.window.height = expected_h;
                        let _ = config::save(&cfg);
                    }
                }
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    if let Some(win) = app_handle.get_webview_window("main") {
                        let _ = win.hide();
                    }
                }
                _ => {}
            });

            spawn_startup_fetch(app.handle(), &initial_config);

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
