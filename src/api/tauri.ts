import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type DshState = "starting" | "ready" | "failed";

export interface StatusInfo {
  state: DshState;
  /** 当前阶段：deploying / starting / ready / failed / stopped */
  stage?: string | null;
  pid?: number | null;
  port?: number | null;
  log_path?: string | null;
  error?: string | null;
  /** 是否正在执行 dsh 包自更新 */
  updating?: boolean;
}

/** dsh 版本检查结果 */
export interface UpdateInfo {
  /** 当前部署版本 */
  current: string;
  /** npm 上的最新版本 */
  latest: string;
  has_update: boolean;
  /** 检查失败原因（离线等） */
  error: string | null;
}

/** 更新进度事件负载 */
export interface UpdateEventPayload {
  kind: "started" | "success" | "error";
  message: string;
}

/** 检查 @deepseek-ai/dsh 是否有新版本 */
export async function checkUpdate(): Promise<UpdateInfo> {
  if (!isTauri()) {
    return { current: "-", latest: "-", has_update: false, error: null };
  }
  return await invoke<UpdateInfo>("check_update");
}

/** 手动触发更新（后台执行，完成后自动重启服务） */
export async function updateDsh(): Promise<void> {
  if (!isTauri()) return;
  await invoke("update_dsh");
}

/** 监听更新进度事件 */
export function onDshUpdate(cb: (payload: UpdateEventPayload) => void): Promise<UnlistenFn> {
  return listen<UpdateEventPayload>("dsh-update", (e) => cb(e.payload));
}

/** 查询 dsh 服务与 sidecar 状态 */
export async function getStatus(): Promise<StatusInfo> {
  if (!isTauri()) {
    return { state: "starting", pid: null, port: null };
  }
  return await invoke<StatusInfo>("get_status");
}

/** 重新启动 dsh 服务 */
export async function restartDsh(): Promise<void> {
  if (!isTauri()) return;
  await invoke("restart_dsh");
}

/** 版本检查完成后切换到 dsh 工作台 */
export async function navigateWorkbench(): Promise<void> {
  if (!isTauri()) return;
  await invoke("navigate_workbench");
}

/** 读取日志尾部若干行（用于失败诊断） */
export async function readLog(tailLines = 200): Promise<string> {
  if (!isTauri()) return "（浏览器预览模式：无日志）";
  return await invoke<string>("read_log", { tailLines });
}

/** 打开日志所在目录 */
export async function openLogDir(): Promise<void> {
  if (!isTauri()) return;
  await invoke("open_log_dir");
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
