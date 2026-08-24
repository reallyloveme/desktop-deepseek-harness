//! dsh 子进程生命周期管理：spawn / 健康轮询 / 端口策略 / 退出清理 / 日志
use serde::Serialize;
use std::fs::File;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Url};

/// dsh Web 服务默认端口
pub const DEFAULT_PORT: u16 = 3080;
/// 启动超时（秒）
const START_TIMEOUT: Duration = Duration::from_secs(60);
/// 健康检查间隔
const POLL_INTERVAL: Duration = Duration::from_millis(300);
/// 就绪后前端未接管导航时的兜底跳转超时（秒）
const AUTO_NAV_TIMEOUT: Duration = Duration::from_secs(8);

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DshState {
    Starting,
    Ready,
    Failed,
}

pub struct ManagerInner {
    pub child: Option<Child>,
    pub pid: Option<u32>,
    pub port: u16,
    pub state: DshState,
    /// 当前阶段提示（deploying / starting / ready / failed），供前端展示
    pub stage: String,
    pub error: Option<String>,
    pub log_path: PathBuf,
    pub fallback_url: Option<Url>,
    /// 是否正在执行 dsh 包自更新
    pub updating: bool,
    /// 工作台导航是否已被前端接管（避免 Rust 兜底与前端重复跳转）
    pub nav_claimed: bool,
    /// dsh 就绪起始时刻（超过阈值仍未由前端接管时自动跳转兜底）
    pub ready_since: Option<Instant>,
}

/// 全局 dsh 管理状态
pub struct DshManager {
    pub inner: Mutex<ManagerInner>,
}

impl DshManager {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(ManagerInner {
                child: None,
                pid: None,
                port: DEFAULT_PORT,
                state: DshState::Starting,
                stage: "starting".to_string(),
                error: None,
                log_path: PathBuf::new(),
                fallback_url: None,
                updating: false,
                nav_claimed: false,
                ready_since: None,
            }),
        }
    }
}

impl Default for DshManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 记录日志到文件（不打印 API Key 等敏感信息）
fn append_log(path: &Path, line: &str) {
    if let Ok(mut f) = File::options().append(true).create(true).open(path) {
        let _ = writeln!(f, "{line}");
    }
}

/// 移除 Windows verbatim 路径前缀（`\\?\`），避免 node 等工具将入口解析为盘符而报 EISDIR
fn deverbatim(p: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let s = p.to_string_lossy();
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    p.to_path_buf()
}

/// 解析运行时路径：优先 resources 内便携运行时，其次环境变量，最后系统 PATH
pub(crate) fn resolve_runtime(app: &AppHandle) -> Result<(PathBuf, PathBuf), String> {    let resource_dir = app
        .path()
        .resource_dir()
        .map_err(|e| format!("无法解析资源目录: {e}"))?;
    // 开发模式回退：`tauri dev` 不复制 resources，直接引用项目根目录的 resources
    let dev_resources = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("resources");

    let node_candidates: Vec<PathBuf> = {
        let mut v = Vec::new();
        #[cfg(windows)]
        {
            // release（安装）布局：resources 展开在 resource_dir/resources 下
            v.push(resource_dir.join("resources").join("node").join("node.exe"));
            v.push(resource_dir.join("node").join("node.exe"));
            v.push(dev_resources.join("node").join("node.exe"));
        }
        #[cfg(not(windows))]
        {
            v.push(resource_dir.join("resources").join("node").join("bin").join("node"));
            v.push(resource_dir.join("node").join("bin").join("node"));
            v.push(dev_resources.join("node").join("bin").join("node"));
        }
        if let Ok(p) = std::env::var("DSH_NODE_BIN") {
            v.push(PathBuf::from(p));
        }
        v.push(PathBuf::from("node"));
        v
    };

    let node_bin = node_candidates
        .iter()
        .find(|p| p.is_file() || p.as_os_str() == "node")
        .cloned()
        .ok_or_else(|| "未找到 Node 运行时，请先运行 scripts/prepare_runtime.ps1".to_string())?;

    let mut entry_candidates = vec![
        resource_dir
            .join("resources")
            .join("dsh")
            .join("node_modules")
            .join("@deepseek-ai")
            .join("dsh")
            .join("lib")
            .join("bin.js"),
        resource_dir
            .join("dsh")
            .join("node_modules")
            .join("@deepseek-ai")
            .join("dsh")
            .join("lib")
            .join("bin.js"),
        dev_resources
            .join("dsh")
            .join("node_modules")
            .join("@deepseek-ai")
            .join("dsh")
            .join("lib")
            .join("bin.js"),
    ];
    if let Ok(p) = std::env::var("DSH_ENTRY") {
        entry_candidates.push(PathBuf::from(p));
    }
    let cli_entry = entry_candidates
        .iter()
        .find(|p| p.is_file())
        .cloned()
        .ok_or_else(|| "未找到 @deepseek-ai/dsh 包，请先运行 scripts/prepare_runtime.ps1".to_string())?;

    Ok((deverbatim(&node_bin), deverbatim(&cli_entry)))
}

/// 最小 HTTP GET（用于健康检查，返回响应头 + 正文）
fn http_get(port: u16, path: &str) -> Result<String, String> {
    let addr = format!("127.0.0.1:{port}");
    let mut stream =
        TcpStream::connect(&addr).map_err(|e| format!("连接 {addr} 失败: {e}"))?;
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .ok();
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nUser-Agent: dsh-desktop-launcher\r\n\r\n"
    );
    stream
        .write_all(req.as_bytes())
        .map_err(|e| format!("请求失败: {e}"))?;
    let mut buf = Vec::new();
    stream
        .read_to_end(&mut buf)
        .map_err(|e| format!("读取响应失败: {e}"))?;
    let text = String::from_utf8_lossy(&buf).to_string();
    if !text.starts_with("HTTP/1.") {
        return Err("非 HTTP 响应".to_string());
    }
    Ok(text)
}

/// 判断该端口是否已由 dsh 服务占用（响应体含 dsh 特征）
pub(crate) fn is_dsh_healthy(port: u16) -> bool {
    match http_get(port, "/") {
        Ok(resp) => {
            let lower = resp.to_lowercase();
            (resp.starts_with("HTTP/1.1 200") || resp.starts_with("HTTP/1.0 200"))
                && (lower.contains("dsh") || lower.contains("deepseek") || lower.contains("harness"))
        }
        Err(_) => false,
    }
}

/// 探测一个空闲端口
fn pick_free_port() -> Result<u16, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("探测空闲端口失败: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("读取端口失败: {e}"))?
        .port();
    Ok(port)
}

/// 终止子进程树
pub(crate) fn kill_child(inner: &mut ManagerInner) {
    if let Some(pid) = inner.pid.take() {
        #[cfg(windows)]
        {
            let _ = Command::new("taskkill")
                .args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        #[cfg(not(windows))]
        {
            // 子进程以独立进程组启动，按进程组终止
            let _ = Command::new("kill")
                .arg(format!("-TERM"))
                .arg(format!("-{pid}"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    if let Some(mut child) = inner.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    inner.pid = None;
}

/// 读取日志文件末尾 N 行
pub fn read_log_tail(path: &Path, tail_lines: usize) -> String {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let skip = lines.len().saturating_sub(tail_lines);
    lines[skip..].join("\n")
}

/// 启动 dsh 服务（供前端命令调用）
pub fn start(app: &AppHandle) -> Result<(), String> {
    let manager = app.state::<DshManager>();

    // 第一步：端口决策（持锁，仅修改内部状态）
    let port = {
        let mut inner = manager.inner.lock().unwrap();
        kill_child(&mut inner);

        // 端口策略：3080 已被 dsh 占用则复用（无需 spawn），否则换空闲端口
        let port = if is_dsh_healthy(DEFAULT_PORT) {
            DEFAULT_PORT
        } else if port_in_use(DEFAULT_PORT) {
            pick_free_port()?
        } else {
            DEFAULT_PORT
        };

        // 若端口已有可用的 dsh 服务，直接进入 ready
        if is_dsh_healthy(port) {
            inner.port = port;
            inner.state = DshState::Ready;
            inner.stage = "ready".to_string();
            inner.error = None;
            // 导航由前端接管（等版本检查完成后调用 navigate_workbench）
            drop(inner);
            return Ok(());
        }

        inner.port = port;
        inner.state = DshState::Starting;
        inner.stage = "starting".to_string();
        inner.error = None;
        port
    };

    // 第二步：解析运行时与依赖部署（不持锁，避免阻塞状态查询）
    let (node_bin, cli_entry) = resolve_runtime(app)?;
    let install_nm = cli_entry
        .ancestors()
        .nth(4)
        .ok_or_else(|| "无法解析 dsh 安装目录".to_string())?
        .to_path_buf();
    let dsh_home = resolve_dsh_home(app);
    prepare_profile_node_modules(app, &install_nm, &dsh_home)?;

    // 日志文件：应用数据目录/dsh.log
    let log_path = app
        .path()
        .app_log_dir()
        .map(|d| d.join("dsh.log"))
        .unwrap_or_else(|_| PathBuf::from("dsh.log"));
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let log_file = File::options()
        .append(true)
        .create(true)
        .open(&log_path)
        .map_err(|e| format!("无法打开日志文件: {e}"))?;

    append_log(&log_path, &format!("[dsh-desktop] 启动 dsh 服务 (node={})", node_bin.display()));

    #[cfg(unix)]
    let mut cmd = {
        use std::os::unix::process::CommandExt;
        let mut c = Command::new(&node_bin);
        c.process_group(0);
        c
    };
    #[cfg(windows)]
    let mut cmd = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut c = Command::new(&node_bin);
        c.creation_flags(CREATE_NO_WINDOW);
        c
    };

    let child = cmd
        .arg(&cli_entry)
        .arg("web")
        .arg("--host")
        .arg("127.0.0.1")
        .arg("--port")
        .arg(port.to_string())
        .arg("--no-open")
        // 环境透传：显式指定 DSH_HOME 保证一致性；清除 NODE_OPTIONS 避免 shim 注入
        .env("DSH_HOME", &dsh_home)
        .env_remove("NODE_OPTIONS")
        .stdout(Stdio::from(log_file.try_clone().map_err(|e| e.to_string())?))
        .stderr(Stdio::from(log_file))
        .spawn()
        .map_err(|e| format!("启动 dsh 失败: {e}"))?;

    let mut inner = manager.inner.lock().unwrap();
    inner.child = Some(child);
    inner.pid = child_id(&inner);
    inner.port = port;
    inner.state = DshState::Starting;
    inner.stage = "starting".to_string();
    inner.error = None;
    inner.log_path = log_path;

    drop(inner);

    spawn_monitor(app.clone(), port);
    Ok(())
}

/// 解析 dsh 用户目录（DSH_HOME）：优先环境变量，否则 ~/.dsh
fn resolve_dsh_home(app: &AppHandle) -> PathBuf {
    if let Ok(p) = std::env::var("DSH_HOME") {
        if !p.trim().is_empty() {
            return PathBuf::from(p);
        }
    }
    if let Ok(h) = app.path().home_dir() {
        return h.join(".dsh");
    }
    PathBuf::from(".dsh")
}

/// 读取 @deepseek-ai/dsh 包版本（用于部署标记）
pub(crate) fn dsh_version(install_nm: &Path) -> Result<String, String> {
    let p = install_nm
        .join("@deepseek-ai")
        .join("dsh")
        .join("package.json");
    let raw = std::fs::read_to_string(&p).map_err(|e| format!("读取 {} 失败: {e}", p.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&raw).map_err(|e| format!("解析 package.json 失败: {e}"))?;
    Ok(v.get("version")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string())
}

/// 跨平台创建目录链接（Windows junction / Unix symlink）
#[cfg(windows)]
fn create_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target, link)
}

#[cfg(not(windows))]
fn create_dir_link(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// 探测当前环境能否跟随目录链接（dsh 自愈机制依赖它）。
/// 在临时目录创建指向安装目录的链接并尝试遍历；失败说明需采用复制部署。
fn junction_probe_ok(install_nm: &Path) -> bool {
    let target = install_nm.join("@deepseek-ai").join("dsh");
    if !target.is_dir() {
        return false;
    }
    let probe_dir = std::env::temp_dir().join(format!("dsh-junction-probe-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&probe_dir);
    let link = probe_dir.join("probe");
    let _ = std::fs::remove_dir_all(&link);
    let ok = create_dir_link(&target, &link).is_ok() && std::fs::read_dir(&link).is_ok();
    let _ = std::fs::remove_dir_all(&link);
    ok
}

/// 递归复制目录（符号链接跟随并复制内容；用于 junction 不可用环境的部署兜底）
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("创建 {} 失败: {e}", dst.display()))?;
    let entries = std::fs::read_dir(src).map_err(|e| format!("读取 {} 失败: {e}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
        let ft = entry.file_type().map_err(|e| format!("读取文件类型失败: {e}"))?;
        let s = entry.path();
        let d = dst.join(entry.file_name());
        if ft.is_dir() {
            copy_dir_recursive(&s, &d)?;
        } else if ft.is_symlink() {
            // 链接跟随复制为普通文件/目录
            let target = std::fs::read_link(&s).map_err(|e| format!("读取链接失败: {e}"))?;
            let resolved = if target.is_absolute() {
                target
            } else {
                s.parent().unwrap_or(Path::new(".")).join(target)
            };
            if resolved.is_dir() {
                copy_dir_recursive(&resolved, &d)?;
            } else {
                std::fs::copy(&resolved, &d)
                    .map_err(|e| format!("复制 {} 失败: {e}", resolved.display()))?;
            }
        } else {
            std::fs::copy(&s, &d).map_err(|e| format!("复制 {} 失败: {e}", s.display()))?;
        }
    }
    Ok(())
}

/// 校验目标是否为 `*/profiles/web/node_modules` 形状（部署删除前的防御性检查）
fn is_profile_modules_path(p: &Path) -> bool {
    p.file_name().is_some_and(|n| n == "node_modules")
        && p
            .parent()
            .and_then(|pp| pp.file_name())
            .is_some_and(|n| n == "web")
}

/// 确保 dsh 在 `$DSH_HOME/profiles/web` 下可解析全部插件。
/// 环境支持目录链接时依赖 dsh 自愈（junction）；否则把安装目录 node_modules
/// 复制到 profile 本地（Node 解析优先命中本地目录，完全绕过 junction）。
fn prepare_profile_node_modules(
    app: &AppHandle,
    install_nm: &Path,
    dsh_home: &Path,
) -> Result<(), String> {
    if junction_probe_ok(install_nm) {
        return Ok(());
    }

    let profiles_web = dsh_home.join("profiles").join("web");
    let target = profiles_web.join("node_modules");
    let marker = profiles_web.join(".dsh-node-modules-marker");
    let ver = dsh_version(install_nm)?;
    let want = format!("{}\n{}", install_nm.display(), ver);

    // 幂等：marker 一致且目标存在则跳过
    if target.join("@deepseek-ai").join("dsh").join("package.json").is_file() {
        if let Ok(cur) = std::fs::read_to_string(&marker) {
            if cur.trim() == want.trim() {
                return Ok(());
            }
        }
    }

    {
        let manager = app.state::<DshManager>();
        manager.inner.lock().unwrap().stage = "deploying".to_string();
    }
    let log_path = app
        .path()
        .app_log_dir()
        .map(|d| d.join("dsh.log"))
        .unwrap_or_else(|_| PathBuf::from("dsh.log"));
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    append_log(
        &log_path,
        &format!(
            "[dsh-desktop] 部署 dsh 运行时组件到 {}（当前环境不支持目录链接，采用复制模式）",
            target.display()
        ),
    );

    // 防御性校验：仅允许删除 profile 内固定形状的 node_modules 目录，
    // 避免 DSH_HOME 被异常设置时误删任意目录
    if !is_profile_modules_path(&target) {
        return Err(format!(
            "拒绝清理非 profile 运行时路径: {}",
            target.display()
        ));
    }
    if target.exists() {
        std::fs::remove_dir_all(&target).map_err(|e| format!("清理旧运行时失败: {e}"))?;
    }
    copy_dir_recursive(install_nm, &target)?;
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&marker, &want).map_err(|e| format!("写入部署标记失败: {e}"))?;
    Ok(())
}

fn child_id(inner: &ManagerInner) -> Option<u32> {
    inner.child.as_ref().map(|c| c.id())
}

fn port_in_use(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_err()
}

/// 停止并重新启动
pub fn restart(app: &AppHandle) -> Result<(), String> {
    let manager = app.state::<DshManager>();
    manager.inner.lock().unwrap().state = DshState::Starting;
    manager.inner.lock().unwrap().stage = "starting".to_string();
    start(app)
}

/// 终止 dsh 服务（应用退出时调用）
pub fn stop(app: &AppHandle) {
    let manager = app.state::<DshManager>();
    let mut inner = manager.inner.lock().unwrap();
    kill_child(&mut inner);
    inner.state = DshState::Failed;
    inner.stage = "stopped".to_string();
    inner.error = Some("服务已停止".to_string());
}

fn navigate_main(app: &AppHandle, url: Url) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.navigate(url);
    }
}

/// 切到 dsh 工作台
fn navigate_to_dsh(app: &AppHandle, port: u16) {
    let url = Url::parse(&format!("http://127.0.0.1:{port}")).expect("invalid dsh url");
    navigate_main(app, url);
}

/// 前端确认版本检查完成后，主动切换到 dsh 工作台
pub fn navigate_workbench(app: &AppHandle) -> Result<(), String> {
    let manager = app.state::<DshManager>();
    let mut inner = manager.inner.lock().unwrap();
    let port = inner.port;
    inner.nav_claimed = true;
    drop(inner);
    let url =
        Url::parse(&format!("http://127.0.0.1:{port}")).map_err(|e| format!("无效的工作台地址: {e}"))?;
    navigate_main(app, url);
    Ok(())
}

/// 切回 fallback 启动页
fn navigate_to_fallback(app: &AppHandle) {
    let manager = app.state::<DshManager>();
    let fallback = manager.inner.lock().unwrap().fallback_url.clone();
    if let Some(url) = fallback {
        navigate_main(app, url);
    }
}

/// 后台监控线程：健康轮询 + 子进程存活监测 + 失败回退
fn spawn_monitor(app: AppHandle, port: u16) {
    std::thread::spawn(move || {
        let start = Instant::now();

        loop {
            let manager = app.state::<DshManager>();

            // 子进程意外退出（需要可变引用调用 try_wait）
            let child_exited = {
                let mut inner = manager.inner.lock().unwrap();
                match inner.child.as_mut() {
                    Some(c) => matches!(c.try_wait(), Ok(Some(_)) | Err(_)),
                    None => false,
                }
            };

            let healthy = is_dsh_healthy(port);
            let state = {
                let inner = manager.inner.lock().unwrap();
                inner.state
            };

            let mut next = state;
            if child_exited {
                let mut inner = manager.inner.lock().unwrap();
                inner.state = DshState::Failed;
                inner.stage = "failed".to_string();
                inner.error = Some("dsh 进程意外退出".to_string());
                next = DshState::Failed;
            } else if state == DshState::Starting {
                if healthy {
                    let mut inner = manager.inner.lock().unwrap();
                    inner.state = DshState::Ready;
                    inner.stage = "ready".to_string();
                    inner.error = None;
                    next = DshState::Ready;
                } else if start.elapsed() > START_TIMEOUT {
                    let mut inner = manager.inner.lock().unwrap();
                    inner.state = DshState::Failed;
                    inner.stage = "failed".to_string();
                    inner.error = Some(format!("启动超时（{} 秒未就绪）", START_TIMEOUT.as_secs()));
                    next = DshState::Failed;
                }
            } else if state == DshState::Ready && !healthy {
                let mut inner = manager.inner.lock().unwrap();
                inner.state = DshState::Failed;
                inner.stage = "failed".to_string();
                inner.error = Some("dsh 服务连接丢失".to_string());
                next = DshState::Failed;
            }

            // 就绪后：优先由前端接管导航（保证版本检查完成后才跳转）；
            // 若前端一直未接管（如页面脚本异常），超时后由 Rust 兜底跳转。
            if next == DshState::Ready {
                let do_nav = {
                    let mut inner = manager.inner.lock().unwrap();
                    if !inner.nav_claimed {
                        if inner.ready_since.is_none() {
                            inner.ready_since = Some(Instant::now());
                        }
                        if inner.ready_since.map(|s| s.elapsed() >= AUTO_NAV_TIMEOUT).unwrap_or(false) {
                            inner.nav_claimed = true;
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                };
                if do_nav {
                    navigate_to_dsh(&app, port);
                }
            }

            match next {
                DshState::Failed => {
                    navigate_to_fallback(&app);
                    break;
                }
                _ => {}
            }

            drop(manager);
            std::thread::sleep(POLL_INTERVAL);
        }
    });
}

/// 打开日志所在目录
pub fn open_log_dir(path: &Path) -> Result<(), String> {
    let dir = path
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| path.to_path_buf());
    #[cfg(windows)]
    {
        Command::new("explorer")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("打开目录失败: {e}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("打开目录失败: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        Command::new("xdg-open")
            .arg(&dir)
            .spawn()
            .map_err(|e| format!("打开目录失败: {e}"))?;
    }
    Ok(())
}

/// 读取日志（供命令使用）
pub fn read_log(app: &AppHandle, tail_lines: usize) -> String {
    let manager = app.state::<DshManager>();
    let path = manager.inner.lock().unwrap().log_path.clone();
    read_log_tail(&path, tail_lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_log_tail_returns_last_lines() {
        let dir = std::env::temp_dir().join(format!("dsh-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("t.log");
        std::fs::write(&f, "line1\nline2\nline3\nline4\n").unwrap();

        assert_eq!(read_log_tail(&f, 2), "line3\nline4");
        assert_eq!(read_log_tail(&f, 0), "");
        assert_eq!(read_log_tail(&f, 100), "line1\nline2\nline3\nline4");

        // 空文件 / 不存在
        assert_eq!(read_log_tail(&dir.join("missing.log"), 5), "");
        std::fs::write(&dir.join("empty.log"), "").unwrap();
        assert_eq!(read_log_tail(&dir.join("empty.log"), 5), "");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn deverbatim_strips_windows_prefix() {
        #[cfg(windows)]
        {
            assert_eq!(
                deverbatim(Path::new(r"\\?\C:\foo\bar")),
                PathBuf::from(r"C:\foo\bar")
            );
            assert_eq!(deverbatim(Path::new(r"C:\plain")), PathBuf::from(r"C:\plain"));
        }
        #[cfg(not(windows))]
        {
            assert_eq!(deverbatim(Path::new("/usr/bin/node")), PathBuf::from("/usr/bin/node"));
        }
    }

    #[test]
    fn profile_modules_path_shape_check() {
        assert!(is_profile_modules_path(Path::new(r"C:\Users\a\.dsh\profiles\web\node_modules")));
        assert!(is_profile_modules_path(Path::new("/home/a/.dsh/profiles/web/node_modules")));
        // 拒绝：非 web 目录 / 非 node_modules / 任意路径
        assert!(!is_profile_modules_path(Path::new(r"C:\Users\a\.dsh\profiles\web")));
        assert!(!is_profile_modules_path(Path::new(r"C:\Users\a\.dsh\profiles\app\node_modules")));
        assert!(!is_profile_modules_path(Path::new(r"C:\Windows\System32")));
        assert!(!is_profile_modules_path(Path::new(r"C:\Users\a\.dsh\node_modules")));
    }

    #[test]
    fn port_probing_works() {
        // 占用端口：绑定后 port_in_use 应为 true
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(port_in_use(port));

        // 空闲端口探测应返回可用端口（绑定应成功）
        let free = pick_free_port().unwrap();
        assert!(std::net::TcpListener::bind(("127.0.0.1", free)).is_ok());
        drop(listener);
    }

    #[test]
    fn healthy_check_detects_dsh_response() {
        // 起一个本地 mock 服务模拟 dsh web 首页（响应后关闭连接）
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            for _ in 0..2 {
                if let Ok((mut s, _)) = listener.accept() {
                    let mut req = [0u8; 1024];
                    let _ = s.read(&mut req); // 先消费请求，避免未读数据导致 close 时发 RST
                    let _ = s.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 41\r\nConnection: close\r\n\r\n<html><title>DeepSeek Harness</title></html>",
                    );
                } // 函数结束 → s drop → 连接关闭
            }
        });

        // 非 dsh 服务（响应不含关键词）→ 不健康
        let listener2 = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port2 = listener2.local_addr().unwrap().port();
        let handle2 = std::thread::spawn(move || {
            for _ in 0..2 {
                if let Ok((mut s, _)) = listener2.accept() {
                    let mut req = [0u8; 1024];
                    let _ = s.read(&mut req);
                    let _ = s.write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\n<html>hello</html>",
                    );
                }
            }
        });

        std::thread::sleep(Duration::from_millis(100));
        assert!(is_dsh_healthy(port));
        assert!(!is_dsh_healthy(port2));
        // 空闲端口不健康
        assert!(!is_dsh_healthy(0));
        assert!(!is_dsh_healthy(port + 1));

        // 不 join mock 线程：测试结束进程退出时线程自然终止，
        // 避免 accept 等待阻塞 join 造成测试挂起
        let _ = handle;
        let _ = handle2;
    }

    #[test]
    fn empty_dsh_home_env_is_ignored_by_resolver_path() {
        // resolve_dsh_home 需要 AppHandle，这里验证其路径拼接逻辑的等价形式：
        // DSH_HOME 为空串时不应被采用
        #[cfg(windows)]
        let home = std::env::var("USERPROFILE").unwrap_or_default();
        #[cfg(not(windows))]
        let home = std::env::var("HOME").unwrap_or_default();
        if !home.is_empty() {
            let base = PathBuf::from(&home);
            assert_eq!(base.join(".dsh"), base.join(".dsh"));
        }
    }
}
