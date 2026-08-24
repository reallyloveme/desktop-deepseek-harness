# 下载便携 Node 运行时并安装 @deepseek-ai/dsh 到 resources/
# 用法: powershell -ExecutionPolicy Bypass -File scripts/prepare_runtime.ps1
# 注意: dsh 需要 Node >= 23（依赖 node:zlib 的 zstd 实验 API），请勿降低版本
param(
    [string]$NodeVersion = "v23.11.1",
    [string]$DshVersion = "0.1.1-rc.2"   # dsh 为 developer preview，锁定精确版本
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
$ResDir = Join-Path $Root "resources"
$NodeDir = Join-Path $ResDir "node"
$DshDir = Join-Path $ResDir "dsh"

# 保证使用真实 Node（绕过可能的 .vite-plus 影子）
$RealNode = "C:\nvm4w\nodejs\node.exe"
if (-not (Test-Path $RealNode)) { $RealNode = "node" }

# 清除 CodeBuddy shim 注入，避免 npm/pnpm 异常
$env:NODE_OPTIONS = $null

Write-Host "==> 准备 resources 目录"
New-Item -ItemType Directory -Force -Path $ResDir | Out-Null

# 版本变更检测：resources/node/.node-version 与目标版本不一致时重新下载
$versionMark = Join-Path $NodeDir ".node-version"
$needNode = $true
if ((Test-Path (Join-Path $NodeDir "node.exe")) -and (Test-Path $versionMark)) {
    $current = (Get-Content $versionMark -Raw).Trim()
    if ($current -eq $NodeVersion) { $needNode = $false }
}

if ($needNode) {
    Write-Host "==> 下载便携 Node $NodeVersion (win-x64)"
    if (Test-Path $NodeDir) { Remove-Item $NodeDir -Recurse -Force }
    $zip = Join-Path $env:TEMP "node-$NodeVersion-win-x64.zip"
    Invoke-WebRequest -Uri "https://nodejs.org/dist/$NodeVersion/node-$NodeVersion-win-x64.zip" -OutFile $zip
    New-Item -ItemType Directory -Force -Path $NodeDir | Out-Null
    Expand-Archive -Path $zip -DestinationPath $NodeDir -Force
    # node.exe 位于解压后的一级子目录中，将其提升到 NodeDir
    $nested = Get-ChildItem $NodeDir -Directory | Select-Object -First 1
    if ($nested -and -not (Test-Path (Join-Path $NodeDir "node.exe"))) {
        Get-ChildItem $nested.FullName | Move-Item -Destination $NodeDir -Force
        Remove-Item $nested.FullName -Recurse -Force
    }
    Remove-Item $zip -Force
    Set-Content -Path $versionMark -Value $NodeVersion -NoNewline
} else {
    Write-Host "==> 便携 Node 已存在 ($NodeVersion)，跳过下载"
}

Write-Host "==> 安装 @deepseek-ai/dsh@$DshVersion 到 resources/dsh"
New-Item -ItemType Directory -Force -Path $DshDir | Out-Null
$pkg = "@deepseek-ai/dsh@$DshVersion"
if ($DshVersion -eq "") { $pkg = "@deepseek-ai/dsh" }

# 优先使用 pnpm（hoisted 布局，规避 E 盘 junction 遍历问题）；否则回退 npm
$pnpm = Get-Command pnpm -ErrorAction SilentlyContinue
if ($pnpm) {
    Write-Host "==> 使用 pnpm (hoisted) 安装"
    & $pnpm.Source install --prefix $DshDir --config.node-linker=hoisted --no-frozen-lockfile $pkg
    if ($LASTEXITCODE -ne 0) { throw "pnpm install 失败" }
} else {
    $npmCmd = Join-Path $NodeDir "npm.cmd"
    if (-not (Test-Path $npmCmd)) { $npmCmd = "npm.cmd" }
    Write-Host "==> 使用 npm 安装"
    $env:NODE_OPTIONS = "--max-old-space-size=8192"
    & $npmCmd install --prefix $DshDir --no-fund --no-audit $pkg
    if ($LASTEXITCODE -ne 0) { throw "npm install 失败" }
}

Write-Host "==> 完成"
Write-Host "  Node: $(Join-Path $NodeDir 'node.exe')"
Write-Host "  dsh : $(Join-Path $DshDir 'node_modules')"
