# 从 C# 参考工程同步第三方静态资源（UEditor / layer / layui / jQuery 等）
#
# 用法：
#   .\tools\sync-wwwroot.ps1                # 默认源 = C# 参考工程 wwwroot\static
#   .\tools\sync-wwwroot.ps1 -Source <目录>  # 指定源
#   .\tools\sync-wwwroot.ps1 -Force          # 强制覆盖（默认按「大小 + 修改时间」跳过未变文件）
#
# 说明：这些资产是第三方前端库（825 文件 / 约 24.6 MB），不纳入 Rust 仓库版本管理；
#       `wwwroot/static/` 已在 .gitignore 中。

param(
    [string]$Source = 'G:\Code\Pek.FrameWork\Pek.DsMallUI\Pek.DsMallUI\wwwroot\static',
    [string]$Target = '',
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($Target)) {
    $Target = Join-Path (Split-Path -Parent $PSScriptRoot) 'wwwroot\static'
}

if (-not (Test-Path -LiteralPath $Source)) {
    Write-Host "[sync-wwwroot] 源目录不存在：$Source" -ForegroundColor Red
    Write-Host '  提示：用 -Source 指定 C# 参考工程 wwwroot\static 的实际路径。'
    exit 1
}

New-Item -ItemType Directory -Force -Path $Target | Out-Null
$sourceFull = (Resolve-Path -LiteralPath $Source).Path
$targetFull = (Resolve-Path -LiteralPath $Target).Path

$copied = 0
$skipped = 0
$bytes = 0

Get-ChildItem -LiteralPath $sourceFull -Recurse -File | ForEach-Object {
    $relative = $_.FullName.Substring($sourceFull.Length).TrimStart('\', '/')
    $destination = Join-Path $targetFull $relative
    $dir = Split-Path -Parent $destination
    if (-not (Test-Path -LiteralPath $dir)) {
        New-Item -ItemType Directory -Force -Path $dir | Out-Null
    }

    if (-not $Force -and (Test-Path -LiteralPath $destination)) {
        $existing = Get-Item -LiteralPath $destination
        if ($existing.Length -eq $_.Length -and $existing.LastWriteTimeUtc -ge $_.LastWriteTimeUtc) {
            $script:skipped++
            return
        }
    }

    Copy-Item -LiteralPath $_.FullName -Destination $destination -Force
    (Get-Item -LiteralPath $destination).LastWriteTimeUtc = $_.LastWriteTimeUtc
    $script:copied++
    $script:bytes += $_.Length
}

Write-Host ("[sync-wwwroot] 完成：复制 {0} 个（{1:N1} MB），跳过 {2} 个未变文件" -f $copied, ($bytes / 1MB), $skipped) -ForegroundColor Green
Write-Host "  源：$sourceFull"
Write-Host "  目标：$targetFull"
exit 0
