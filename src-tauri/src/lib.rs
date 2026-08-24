mod dsh;

use serde_json::json;
use tauri::{Manager, RunEvent};

/// 查询 dsh 服务状态
#[tauri::command]
fn get_status(manager: tauri::State<dsh::DshManager>) -> serde_json::Value {
    let inner = manager.inner.lock().unwrap();
    json!({
        "state": inner.state,
        "stage": inner.stage,
        "pid": inner.pid,
        "port": inner.port,
        "log_path": inner.log_path.to_string_lossy().to_string(),
        "error": inner.error,
    })
}

/// 重新启动 dsh 服务
#[tauri::command]
fn restart_dsh(app: tauri::AppHandle) -> Result<(), String> {
    dsh::restart(&app)
}

/// 读取日志尾部若干行（上限 5000 行，防止一次读取过大）
#[tauri::command]
fn read_log(app: tauri::AppHandle, tail_lines: usize) -> String {
    dsh::read_log(&app, tail_lines.min(5000))
}

/// 打开日志所在目录
#[tauri::command]
fn open_log_dir(app: tauri::AppHandle) -> Result<(), String> {
    let manager = app.state::<dsh::DshManager>();
    let path = manager.inner.lock().unwrap().log_path.clone();
    dsh::open_log_dir(&path)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }))
        .manage(dsh::DshManager::new())
        .setup(|app| {
            // 记录 fallback 页 URL，供失败时切回
            let fallback_url = app
                .get_webview_window("main")
                .and_then(|w| w.url().ok());
            let manager = app.state::<dsh::DshManager>();
            manager.inner.lock().unwrap().fallback_url = fallback_url;
            // 启动时自动拉起 dsh
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                if let Err(e) = dsh::start(&handle) {
                    let manager = handle.state::<dsh::DshManager>();
                    let mut inner = manager.inner.lock().unwrap();
                    inner.state = dsh::DshState::Failed;
                    inner.stage = "failed".to_string();
                    inner.error = Some(e);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // 主窗口关闭时停止服务
            if let tauri::WindowEvent::Destroyed = event {
                if window.label() == "main" {
                    window.app_handle().exit(0);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            restart_dsh,
            read_log,
            open_log_dir
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            if let RunEvent::ExitRequested { .. } = event {
                dsh::stop(app);
            }
        });
}
