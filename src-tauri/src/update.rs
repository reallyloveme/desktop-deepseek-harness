//! dsh 包自更新：启动时检测 npm registry 最新版，手动触发更新后重启服务。
use crate::dsh;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// npm 包名
const PKG_NAME: &str = "@deepseek-ai/dsh";
/// npm registry 最新版本元数据接口
const REGISTRY_META: &str = "https://registry.npmjs.org/@deepseek-ai/dsh/latest";
/// 更新进度事件名（前端通过 tauri 事件监听）
pub const UPDATE_EVENT: &str = "dsh-update";

/// 版本检查结果
#[derive(Serialize, Clone)]
pub struct UpdateInfo {
    /// 当前部署版本
    pub current: String,
    /// npm 上的最新版本
    pub latest: String,
    /// 是否有可更新版本
    pub has_update: bool,
    /// 检查失败原因（离线 / 网络错误等）
    pub error: Option<String>,
}

/// 更新进度事件负载
#[derive(Clone, Serialize)]
pub struct UpdateEvent {
    /// started / success / error
    pub kind: &'static str,
    pub message: String,
}

/// 定位实际使用的 dsh 安装目录（node_modules）与便携 node
fn locate(app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {
    let (node_bin, cli_entry) = dsh::resolve_runtime(app)?;
    let install_nm = cli_entry
        .ancestors()
        .nth(4)
        .ok_or_else(|| "无法解析 dsh 安装目录".to_string())?
        .to_path_buf();
    Ok((node_bin, install_nm))
}

/// 查询 npm registry 上的最新版本
fn fetch_latest() -> Result<String, String> {
    let resp = ureq::get(REGISTRY_META)
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|e| format!("网络请求失败: {e}"))?;
    let text = resp
        .into_string()
        .map_err(|e| format!("读取响应失败: {e}"))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("解析响应失败: {e}"))?;
    v.get("version")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "响应缺少 version 字段".to_string())
}

/// 拆分为 (主/次/补丁数字, 预发布段)，如 `0.1.1-rc.2` → ([0,1,1], "rc.2")
fn split_ver(s: &str) -> (Vec<u64>, String) {
    let (main, pre) = match s.split_once('-') {
        Some((m, p)) => (m, Some(p.to_string())),
        None => (s, None),
    };
    let nums = main
        .split('.')
        .filter_map(|x| x.parse::<u64>().ok())
        .collect();
    (nums, pre.unwrap_or_default())
}

/// 版本比较（semver 简化版：主/次/补丁数字 + `-rc` 预发布后缀）
fn ver_gt(a: &str, b: &str) -> bool {
    let (an, ap) = split_ver(a);
    let (bn, bp) = split_ver(b);
    for (x, y) in an.iter().zip(bn.iter()) {
        if x != y {
            return x > y;
        }
    }
    if an.len() != bn.len() {
        return an.len() > bn.len();
    }
    // 预发布：正式版 > 预发布版；同为预发布时按段比较
    match (ap.is_empty(), bp.is_empty()) {
        (true, true) => false,
        (true, false) => true,
        (false, true) => false,
        (false, false) => ap > bp,
    }
}

/// 读取当前部署的 dsh 版本
pub fn current_version(app: &AppHandle) -> Result<String, String> {
    let (_, install_nm) = locate(app)?;
    dsh::dsh_version(&install_nm)
}

/// 检查是否有新版本（启动时 / 手动按钮调用）
pub fn check_update(app: &AppHandle) -> UpdateInfo {
    let current = current_version(app).unwrap_or_else(|e| format!("未知（{e}）"));
    match fetch_latest() {
        Ok(latest) => {
            let has_update = !latest.is_empty() && latest != current && ver_gt(&latest, &current);
            UpdateInfo {
                current,
                latest,
                has_update,
                error: None,
            }
        }
        Err(e) => UpdateInfo {
            current,
            latest: String::new(),
            has_update: false,
            error: Some(e),
        },
    }
}

/// 触发后台更新：停止服务 → 安装新版本 → 重启服务
pub fn start_update(app: AppHandle) -> Result<(), String> {
    let info = check_update(&app);
    if let Some(e) = &info.error {
        return Err(format!("检查更新失败：{e}"));
    }
    if !info.has_update {
        return Err("当前已是最新版本".to_string());
    }
    let latest = info.latest.clone();
    {
        let manager = app.state::<dsh::DshManager>();
        let inner = manager.inner.lock().unwrap();
        if inner.updating {
            return Err("更新正在进行中，请稍候".to_string());
        }
    }
    app.state::<dsh::DshManager>().inner.lock().unwrap().updating = true;

    std::thread::spawn(move || {
        let _ = app.emit(
            UPDATE_EVENT,
            UpdateEvent {
                kind: "started",
                message: format!("开始更新到 {PKG_NAME}@{latest}…"),
            },
        );
        match do_update(&app, &latest) {
            Ok(_) => {
                let _ = app.emit(
                    UPDATE_EVENT,
                    UpdateEvent {
                        kind: "success",
                        message: "更新完成，服务已重启".to_string(),
                    },
                );
            }
            Err(e) => {
                // 尽力恢复旧版本服务
                let _ = dsh::restart(&app);
                let _ = app.emit(
                    UPDATE_EVENT,
                    UpdateEvent {
                        kind: "error",
                        message: e,
                    },
                );
            }
        }
        app.state::<dsh::DshManager>().inner.lock().unwrap().updating = false;
    });
    Ok(())
}

/// 执行更新（在后台线程中运行）
fn do_update(app: &AppHandle, latest: &str) -> Result<(), String> {
    let (node_bin, install_nm) = locate(app)?;
    let dsh_dir = install_nm
        .parent()
        .ok_or_else(|| "无法定位 dsh 包目录".to_string())?
        .to_path_buf();
    let port = app.state::<dsh::DshManager>().inner.lock().unwrap().port;

    // 1. 停止当前服务
    {
        let manager = app.state::<dsh::DshManager>();
        let mut inner = manager.inner.lock().unwrap();
        dsh::kill_child(&mut inner);
    }
    std::thread::sleep(Duration::from_millis(600));
    if dsh::is_dsh_healthy(port) {
        return Err("现有 dsh 服务无法停止，更新已中止".to_string());
    }

    // 2. 使用便携 npm 安装新版本（会同步解析并安装其依赖）
    let npm_cli = node_bin
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("node_modules")
        .join("npm")
        .join("bin")
        .join("npm-cli.js");
    if !npm_cli.is_file() {
        return Err(format!("未找到便携 npm（{}）", npm_cli.display()));
    }
    let output = Command::new(&node_bin)
        .arg(&npm_cli)
        .args(["install", &format!("{PKG_NAME}@{latest}")])
        .current_dir(&dsh_dir)
        .env_remove("NODE_OPTIONS")
        .output()
        .map_err(|e| format!("执行 npm install 失败: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("未知错误");
        return Err(format!("npm install 失败：{tail}（请检查网络后重试）"));
    }

    // 3. 重启服务（start 内部会重新解析路径并部署）
    dsh::restart(app)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_comparison_works() {
        // 主/次/补丁升级
        assert!(ver_gt("0.1.2", "0.1.1"));
        assert!(ver_gt("0.2.0", "0.1.9"));
        assert!(ver_gt("1.0.0", "0.9.9"));
        // 预发布：正式版大于预发布
        assert!(ver_gt("0.1.1", "0.1.1-rc.2"));
        assert!(ver_gt("0.1.1-rc.3", "0.1.1-rc.2"));
        // 相等 / 降级
        assert!(!ver_gt("0.1.1", "0.1.1"));
        assert!(!ver_gt("0.1.1-rc.2", "0.1.1"));
        assert!(!ver_gt("0.1.1", "0.1.2"));
        assert!(!ver_gt("0.1.1-rc.2", "0.1.2"));
    }

    #[test]
    fn version_split_handles_pre_release() {
        assert_eq!(split_ver("0.1.1"), (vec![0, 1, 1], String::new()));
        assert_eq!(split_ver("0.1.1-rc.2"), (vec![0, 1, 1], "rc.2".to_string()));
        assert_eq!(split_ver("1.0.0-beta.10"), (vec![1, 0, 0], "beta.10".to_string()));
    }
}
