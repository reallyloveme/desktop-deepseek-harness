# 一键构建：准备运行时 -> 编译前端 -> Tauri bundle
param(
    [switch]$SkipPrepare
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot

if (-not $SkipPrepare) {
    & (Join-Path $PSScriptRoot "prepare_runtime.ps1")
    if ($LASTEXITCODE -ne 0) { throw "prepare_runtime 失败" }
}

Push-Location $Root
try {
    Write-Host "==> 编译前端"
    & npm run build
    if ($LASTEXITCODE -ne 0) { throw "前端构建失败" }

    Write-Host "==> Tauri bundle"
    & npm run tauri -- build
    if ($LASTEXITCODE -ne 0) { throw "Tauri 构建失败" }
} finally {
    Pop-Location
}
Write-Host "==> 构建完成"
