# DeepSeek Harness Desktop

将官方 [DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness)（dsh，Agent 框架）的 Web UI 直接封装为跨平台桌面应用：**双击启动、退出即停**，替代每次手动执行 `npx @deepseek-ai/dsh web` 的命令行流程。

## 功能

- **一键启动**：应用启动即自动拉起 dsh Web 服务，就绪后自动打开内置窗口展示官方 UI
- **内置窗口**：Tauri WebviewWindow 加载 `http://127.0.0.1:3080`，完整保留官方 Web UI 全部能力（对话/任务、插件、会话管理、模型与 API 配置）
- **进程生命周期管理**：退出应用自动终止 dsh 进程及子进程树；stdout/stderr 日志重定向到应用数据目录
- **启动状态反馈**：启动中指示、失败原因展示（含日志查看）与一键重试
- **环境变量透传**：透传 `DEEPSEEK_API_KEY` / `DEEPSEEK_BASE_URL` / `DSH_*` 等环境变量；也可在官方 UI 内直接配置
- **自包含分发**：构建时打包便携 Node.js 运行时与 `@deepseek-ai/dsh` 进应用资源，目标机器无需安装 Node.js/npm
- **端口策略**：默认 3080；被占用时自动探测空闲端口重建服务，或复用既有 dsh 服务

## 技术架构

Tauri 2 壳作为唯一自建可执行程序：启动时 spawn 子进程运行 dsh Web 服务（resources 内便携 node 直接调用 dsh CLI），轮询健康检查，就绪后创建 WebviewWindow 指向本地端口；退出时递归终止进程树。

```
Tauri 壳（spawn/kill、健康轮询、端口策略）
  └─> node <resources>/dsh/.../lib/bin.js web --port 3080 --no-open
        └─> http://127.0.0.1:3080 官方 Web UI
              └─> WebviewWindow 加载
```

### 目录结构

```
├── src/                  # Vue3 fallback 启动页（加载中/失败重试/日志）
│   ├── views/StatusView.vue
│   └── api/tauri.ts      # invoke 封装（状态查询/重启/日志读取）
├── src-tauri/            # Tauri 2 壳（Rust）
│   └── src/dsh.rs        # spawn/kill、健康轮询、端口探测、日志、运行时部署
├── resources/            # [构建时生成] 便携 node + @deepseek-ai/dsh
├── scripts/
│   ├── prepare_runtime.ps1 / .sh   # 下载 Node、安装 dsh 到 resources
│   └── build.ps1 / build.sh        # prepare + 前端构建 + tauri bundle
```

## 开发

前置要求：

- Node.js >= 23（dsh 依赖 `node:zlib` 的 zstd 实验 API）
- Rust（GNU/MSVC toolchain 均可，Windows 建议 GNU + MinGW）
- pnpm（可选，推荐用于安装运行时依赖）

```bash
# 1. 安装前端依赖
npm install

# 2. 准备运行时（下载便携 Node 到 resources/node，安装 dsh 到 resources/dsh）
pnpm run prepare:runtime        # 或 npm run prepare:runtime

# 3. 开发模式（vite + tauri dev）
npm run tauri dev
```

> 注意：若开发机 PATH 中存在 node 影子/包装器（如 `.vite-plus`），构建脚本会自动清除 `NODE_OPTIONS` 注入；请确保 `node` 解析到真实二进制。

## 构建

```bash
# Windows
powershell -ExecutionPolicy Bypass -File scripts/build.ps1

# macOS / Linux
bash scripts/build.sh
```

产物位于 `src-tauri/target/release/bundle/`。

### 运行时部署策略

dsh 启动时需要为 `$DSH_HOME/profiles/web` 解析全部插件。官方机制依赖目录链接（Windows junction / Unix symlink）：

- **环境支持目录链接**（普通 Windows/macOS/Linux）：dsh 自愈，无需额外操作
- **环境不支持目录链接**（如部分云盘/沙箱文件系统，`junction` 遍历报 `UNKNOWN`）：应用自动将 `resources/dsh/node_modules` 复制到 `$DSH_HOME/profiles/web/node_modules`（Node 模块解析优先命中本地目录，幂等部署，带版本标记），随后正常启动

### 环境变量

| 变量 | 说明 |
| --- | --- |
| `DSH_HOME` | dsh 用户数据目录，默认 `~/.dsh`（与官方 CLI 一致） |
| `DSH_NODE_BIN` | 覆盖 Node 可执行文件路径（调试用） |
| `DSH_ENTRY` | 覆盖 dsh CLI 入口 `bin.js` 路径（调试用） |
| `DEEPSEEK_API_KEY` 等 | 原样透传给 dsh 子进程，不落盘、不进日志 |

## 平台说明

- **Windows**：spawn 使用 `CREATE_NO_WINDOW` 隐藏控制台；持久 PTY 后端不支持 Windows agent 为官方限制，不影响 Web UI
- **macOS / Linux**：spawn 设置独立进程组，退出时 `kill -- -pgid` 清理进程树
- **单实例**：接入 `tauri-plugin-single-instance`，避免多开抢端口

## 许可证

本项目仅作为官方 DeepSeek Harness 的桌面启动器封装，业务功能完全由官方 dsh 提供，请遵循 [deepseek-harness](https://github.com/deepseek-ai/deepseek-harness) 的许可证约束。
