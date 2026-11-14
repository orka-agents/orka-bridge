# Throwaway hosted-CI startup reproduction. Not a product check or a lifecycle walk.
param([Parameter(Mandatory)][string]$Executable, [Parameter(Mandatory)][string]$Output)
$ErrorActionPreference = 'Stop'
if ($env:GITHUB_ACTIONS -ne 'true') { throw 'Fresh hosted CI only' }
$Executable = (Resolve-Path $Executable).Path
$Output = [IO.Path]::GetFullPath($Output)
New-Item -ItemType Directory -Force $Output | Out-Null

# Reuse the shipped probe and its original frame/readiness polling bounds without
# executing the lifecycle walk or changing that check's outcome.
$source = Get-Content -Raw (Join-Path $PSScriptRoot 'native-lifecycle-windows.ps1')
$start = $source.IndexOf('Add-Type -AssemblyName')
$end = $source.IndexOf('$Executable = (Resolve-Path')
Invoke-Expression $source.Substring($start, $end - $start)
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseInput($source, [ref]$tokens, [ref]$errors)
if ($errors.Count) { throw 'Lifecycle probe has syntax errors' }
foreach ($name in @('Wait-For', 'Save-BrowserDiagnostics', 'Start-Bridge', 'Capture')) {
    $definition = @($ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq $name }.GetNewClosure(), $true))
    if ($definition.Count -ne 1) { throw "Expected one lifecycle function: $name" }
    Invoke-Expression $definition[0].Extent.Text
}
$data = Join-Path ([Environment]::GetFolderPath('ApplicationData')) 'ai.orka.bridge'
$selected = Join-Path $data 'setup-v1/local-vm'
$vmData = Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'ai.orka.bridge/local-vm-v1'
if (Test-Path $data) { throw 'Expected fresh hosted-CI application profile; will not erase it' }
if (Get-Process -Name orka-bridge,msedgewebview2 -ErrorAction SilentlyContinue) { throw 'App or WebView2 already running' }
$env:BRIDGE_DEV_IMAGE_DIR = Join-Path $env:RUNNER_TEMP "startup-empty-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory $env:BRIDGE_DEV_IMAGE_DIR | Out-Null
$results = @()
for ($launch = 1; $launch -le 10; $launch++) {
    $script:themeOutput = Join-Path $Output "launch-$launch"
    New-Item -ItemType Directory $script:themeOutput | Out-Null
    $browserLog = Join-Path $script:themeOutput 'webview2.log'
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--enable-logging --v=1 --log-file=`"$browserLog`""
    $script:app = $null
    $watch = [Diagnostics.Stopwatch]::StartNew()
    $stalled = $false
    try {
        Start-Bridge
        $frameMs = $watch.ElapsedMilliseconds
        # Setup's ready marker proves controller construction finished. Avoid UIA
        # calls here: this experiment targets pre-setup startup, not navigation.
        Wait-For 'Native setup did not become ready' {
            $script:app.Refresh()
            if ($script:app.HasExited) { throw 'App exited during startup' }
            $stream = [IO.File]::Open($script:startupLog, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
            $reader = [IO.StreamReader]::new($stream)
            try { $reader.ReadToEnd().Contains('bridge-startup: ready') } finally { $reader.Dispose() }
        } | Out-Null
        Save-BrowserDiagnostics 'ready'
        $results += [pscustomobject]@{ launch = $launch; status = 'ready'; frameMs = $frameMs; observedMs = $watch.ElapsedMilliseconds }
    } catch {
        $stalled = $true
        $results += [pscustomobject]@{ launch = $launch; status = 'startup-failure'; observedMs = $watch.ElapsedMilliseconds; error = $_.ToString() }
        Save-BrowserDiagnostics 'failure'
        try { Capture 'failure' $true } catch { Write-Warning $_ }
        if ($script:app -and !$script:app.HasExited) {
            Start-Sleep -Seconds 5
            Save-BrowserDiagnostics 'failure-after-5s'
            Start-Sleep -Seconds 15
            Save-BrowserDiagnostics 'failure-after-20s'
        }
    } finally {
        $results | ConvertTo-Json -AsArray | Set-Content (Join-Path $Output 'results.json')
        if ($script:app -and !$script:app.HasExited) {
            # Diagnostic cleanup, never evidence of a working product quit route.
            & taskkill.exe /PID $script:app.Id /T /F | Out-Null
        }
        # Do not let taskkill's success/failure become the experiment verdict.
        $global:LASTEXITCODE = 0
    }
    Write-Output ($results[-1] | ConvertTo-Json -Compress)
    if ($stalled) { break }
    Wait-For 'WebView2 children did not exit after diagnostic cleanup' {
        if (!(Get-Process -Name orka-bridge,msedgewebview2 -ErrorAction SilentlyContinue)) { return $true }
    } | Out-Null
}
Remove-Item Env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS
"Launches=$($results.Count); startupFailures=$(@($results | Where-Object status -ne 'ready').Count)" | Tee-Object (Join-Path $Output 'summary.txt')
