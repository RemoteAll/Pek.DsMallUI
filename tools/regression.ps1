# 一键回归：编译 → 单元/文档测试 → 起服务冒烟 → 汇总
#
# 用法：
#   .\tools\regression.ps1              # 完整回归
#   .\tools\regression.ps1 -SkipSmoke   # 只编译 + 测试
#
# 退出码：0 = 全部通过；1 = 有失败（失败项在报告末尾列出）

param(
    [switch]$SkipSmoke,
    [int]$MinTests = 84
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$artifacts = Join-Path $root 'artifacts\regression'
New-Item -ItemType Directory -Force -Path $artifacts | Out-Null
$report = Join-Path $artifacts ('report-{0:yyyyMMdd-HHmmss}.txt' -f (Get-Date))

$script:failures = New-Object System.Collections.Generic.List[string]
$script:passed = 0
$script:known = 0
$script:skipped = 0

function Write-Log {
    param([string]$Text, [string]$Level = 'INFO')
    $line = '[{0}] {1,-5} {2}' -f (Get-Date -Format 'HH:mm:ss'), $Level, $Text
    Write-Host $line
    Add-Content -LiteralPath $report -Value $line -Encoding UTF8
}

function Add-Failure {
    param([string]$Text)
    $script:failures.Add($Text) | Out-Null
    Write-Log $Text 'FAIL'
}

function Add-Pass {
    param([string]$Text)
    $script:passed++
    Write-Log $Text 'PASS'
}

function Add-Skip {
    param([string]$Text)
    $script:skipped++
    Write-Log $Text 'SKIP'
}

# 外部命令封装：临时关闭 Stop（外部程序写 stderr 会被当成终止错误），取回退出码与输出
function Invoke-Native {
    param(
        [string]$File,
        [string[]]$Arguments,
        [string]$Label
    )
    $previous = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    try {
        $lines = & $File @Arguments 2>&1 | ForEach-Object { "$_" }
        $code = $LASTEXITCODE
    }
    finally {
        $ErrorActionPreference = $previous
    }
    return [pscustomobject]@{ Label = $Label; ExitCode = $code; Output = @($lines) }
}

Write-Log ("回归开始：{0}" -f $root)
Write-Log ("报告文件：{0}" -f $report)

# ————— ① 编译检查 —————
$check = Invoke-Native -File 'cargo' -Arguments @('check', '--all-targets') -Label 'cargo check'
$check.Output | Select-Object -Last 5 | ForEach-Object { Write-Log "  $_" }
if ($check.ExitCode -eq 0) {
    Add-Pass 'cargo check --all-targets'
}
else {
    Add-Failure ("cargo check 失败（exit {0}）" -f $check.ExitCode)
    $check.Output | Select-Object -Last 40 | ForEach-Object { Write-Log "  $_" }
}

# ————— ② 单元 + 文档测试 —————
$test = Invoke-Native -File 'cargo' -Arguments @('test') -Label 'cargo test'
$test.Output | Where-Object { $_ -match 'test result|running \d+ tests|FAILED|panicked' } | ForEach-Object { Write-Log "  $_" }

$totalPassed = 0
foreach ($line in $test.Output) {
    if ($line -match 'test result: ok\.\s+(\d+) passed') {
        $totalPassed += [int]$Matches[1]
    }
}
if ($test.ExitCode -eq 0) {
    Add-Pass ("cargo test：{0} 项通过（基线 {1}）" -f $totalPassed, $MinTests)
    if ($totalPassed -lt $MinTests) {
        Add-Failure ("测试数量下降：{0} < 基线 {1}（新增用例后请抬高 -MinTests）" -f $totalPassed, $MinTests)
    }
}
else {
    Add-Failure ("cargo test 失败（exit {0}）" -f $test.ExitCode)
    $test.Output | Select-Object -Last 60 | ForEach-Object { Write-Log "  $_" }
}

# ————— ③ 冒烟：起示例服务并打真实请求 —————
if ($SkipSmoke) {
    Add-Skip '冒烟测试（-SkipSmoke）'
}
else {
    $build = Invoke-Native -File 'cargo' -Arguments @('build', '--example', 'admin_server') -Label 'cargo build example'
    if ($build.ExitCode -ne 0) {
        Add-Failure ("示例编译失败（exit {0}）" -f $build.ExitCode)
        $build.Output | Select-Object -Last 30 | ForEach-Object { Write-Log "  $_" }
    }
    else {
        Add-Pass 'cargo build --example admin_server'
        $exe = Join-Path $root 'target\debug\examples\admin_server.exe'

        # 取一个空闲端口（避免与开发中的服务冲突）
        $listener = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
        $listener.Start()
        $port = ([System.Net.IPEndPoint]$listener.LocalEndpoint).Port
        $listener.Stop()

        $outFile = Join-Path $artifacts ("smoke-{0}.log" -f $port)
        $process = $null
        try {
            $process = Start-Process -FilePath $exe -ArgumentList $port -WorkingDirectory $root -PassThru -RedirectStandardOutput $outFile -RedirectStandardError ($outFile + '.err')
            $base = "http://127.0.0.1:$port"
            # 示例默认区域为 Biz（可用第二个参数改成 Admin）；冒烟按安装值验证
            $areaSegment = 'biz'

            $ready = $false
            for ($i = 0; $i -lt 40; $i++) {
                Start-Sleep -Milliseconds 250
                try {
                    $probe = Invoke-WebRequest -Uri "$base/fileuploader?action=config" -UseBasicParsing -TimeoutSec 3
                    if ($probe.StatusCode -eq 200) { $ready = $true; break }
                }
                catch { }
            }

            if (-not $ready) {
                Add-Failure '冒烟：服务 10 秒内未就绪'
            }
            else {
                # ① 后台首页 + 设置页（区域名由示例默认值 Biz 决定）
                $homePage = Invoke-WebRequest -Uri "$base/$areaSegment/home/index" -UseBasicParsing -TimeoutSec 5
                if ($homePage.StatusCode -eq 200) {
                    Add-Pass ('冒烟 /{0}/home/index（{1} 字节）' -f $areaSegment, $homePage.Content.Length)
                }
                else {
                    Add-Failure ('冒烟 /{0}/home/index 异常：HTTP {1}' -f $areaSegment, $homePage.StatusCode)
                }

                # 旧默认区域不应再挂载（区域可配置的隔离性）
                $legacy = $null
                try {
                    $legacy = Invoke-WebRequest -Uri "$base/admin/home/index" -UseBasicParsing -TimeoutSec 5
                }
                catch {
                    $legacy = $_.Exception.Response
                }
                if ($null -ne $legacy -and [int]$legacy.StatusCode -eq 404) {
                    Add-Pass '冒烟 /admin/home/index 已隔离（404）'
                }
                else {
                    Add-Failure '冒烟 /admin/home/index 不应可访问'
                }

                $page = Invoke-WebRequest -Uri "$base/$areaSegment/config/index" -UseBasicParsing -TimeoutSec 5
                if ($page.StatusCode -eq 200 -and $page.Content.Contains('<form')) {
                    Add-Pass ('冒烟 /{0}/config/index（{1} 字节）' -f $areaSegment, $page.Content.Length)
                }
                else {
                    Add-Failure ('冒烟 /{0}/config/index 异常：HTTP {1}' -f $areaSegment, $page.StatusCode)
                }

                # ② UEditor 配置
                $config = Invoke-WebRequest -Uri "$base/fileuploader?action=config" -UseBasicParsing -TimeoutSec 5
                if ($config.Content.Contains('imageActionName')) {
                    Add-Pass '冒烟 /fileuploader?action=config'
                }
                else {
                    Add-Failure '冒烟 /fileuploader?action=config 未返回配置'
                }

                # ③ 登录（错误口令应 200 并提示；正确口令应 302）
                $bad = Invoke-WebRequest -Uri "$base/$areaSegment/login/login" -Method Post -Body @{ UserName = 'admin'; Password = 'wrong' } -UseBasicParsing -TimeoutSec 5
                if ($bad.StatusCode -eq 200) {
                    Add-Pass '冒烟 登录失败路径'
                }
                else {
                    Add-Failure ('冒烟 登录失败路径异常：HTTP {0}' -f $bad.StatusCode)
                }

                # ④ 上传（需要 curl.exe；缺失时跳过并说明原因）
                if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
                    $tempPng = Join-Path $artifacts 'smoke.png'
                    [IO.File]::WriteAllBytes($tempPng, [byte[]](0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x01, 0x02))
                    $upload = & curl.exe -s -F "upfile=@$tempPng;type=image/png" "$base/fileuploader?action=uploadimage"
                    if ($upload -match '"state":"SUCCESS"') {
                        Add-Pass '冒烟 UEditor 图片上传'
                    }
                    else {
                        Add-Failure ("冒烟 UEditor 图片上传失败：{0}" -f $upload)
                    }
                }
                else {
                    Add-Skip '冒烟 UEditor 上传（未找到 curl.exe）'
                }
            }
        }
        catch {
            Add-Failure ("冒烟异常：{0}" -f $_.Exception.Message)
        }
        finally {
            if ($null -ne $process -and -not $process.HasExited) {
                Stop-Process -Id $process.Id -Force
            }
            # 冒烟产生的上传文件不留在工作区
            $uploadDir = Join-Path $root 'wwwroot\upload'
            if (Test-Path -LiteralPath $uploadDir) {
                Remove-Item -LiteralPath $uploadDir -Recurse -Force -ErrorAction SilentlyContinue
            }
        }
    }
}

# ————— 汇总 —————
Write-Log ('汇总：通过 {0}，跳过 {1}，失败 {2}' -f $passed, $skipped, $failures.Count)
if ($failures.Count -gt 0) {
    foreach ($failure in $failures) {
        Write-Log ("  失败项：{0}" -f $failure) 'FAIL'
    }
    Write-Log '回归结果：FAILED' 'FAIL'
    exit 1
}

Write-Log '回归结果：PASSED'
exit 0
