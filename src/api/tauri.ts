import { invoke } from "@tauri-apps/api/core";

export type DshState = "starting" | "ready" | "failed";

export interface StatusInfo {
  state: DshState;
  /** 当前阶段：deploying / starting / ready / failed / stopped */
  stage?: string | null;
  pid?: number | null;
  port?: number | null;
  log_path?: string | null;
  error?: string | null;
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
