# E2E smoke tests for Anvil (PowerShell).
# Verifies basic CLI functionality without requiring an LLM backend.
$ErrorActionPreference = "Stop"

$Anvil = if ($args[0]) { $args[0] } else { "cargo run --quiet --" }
$TmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ("anvil-e2e-" + [guid]::NewGuid().ToString("N").Substring(0,8))
New-Item -ItemType Directory -Path $TmpDir -Force | Out-Null

$pass = 0
$fail = 0

function Check($desc, [scriptblock]$test) {
    try {
        & $test | Out-Null
        Write-Host "  ✓ $desc"
        $script:pass++
    } catch {
        Write-Host "  ✗ $desc"
        $script:fail++
    }
}

function CheckOutput($desc, $pattern, [scriptblock]$cmd) {
    $output = & $cmd 2>&1 | Out-String
    if ($output -match $pattern) {
        Write-Host "  ✓ $desc"
        $script:pass++
    } else {
        Write-Host "  ✗ $desc"
        $script:fail++
    }
}

Write-Host "anvil e2e smoke tests"
Write-Host "====================="

# Version
CheckOutput "anvil --version prints version" "anvil" { Invoke-Expression "$Anvil --version" }

# Help
CheckOutput "anvil --help shows usage" "coding agent" { Invoke-Expression "$Anvil --help" }

# Init
Write-Host ""
Write-Host "init tests (in $TmpDir):"
Check "anvil init creates .anvil/" { Push-Location $TmpDir; Invoke-Expression "$Anvil init"; Pop-Location }
Check ".anvil/config.toml exists" { Test-Path "$TmpDir/.anvil/config.toml" -ErrorAction Stop }
Check ".anvil/context.md exists" { Test-Path "$TmpDir/.anvil/context.md" -ErrorAction Stop }
Check ".anvil/skills/ directory exists" { Test-Path "$TmpDir/.anvil/skills" -ErrorAction Stop }

# Skills count
$skillCount = (Get-ChildItem "$TmpDir/.anvil/skills/*.md" -ErrorAction SilentlyContinue).Count
if ($skillCount -eq 21) {
    Write-Host "  ✓ 21 bundled skills installed"
    $pass++
} else {
    Write-Host "  ✗ expected 21 skills, found $skillCount"
    $fail++
}

# Model profiles
Check ".anvil/models/ directory exists" { Test-Path "$TmpDir/.anvil/models" -ErrorAction Stop }
$profileCount = (Get-ChildItem "$TmpDir/.anvil/models/*.toml" -ErrorAction SilentlyContinue).Count
if ($profileCount -ge 5) {
    Write-Host "  ✓ $profileCount model profiles installed"
    $pass++
} else {
    Write-Host "  ✗ expected ≥5 profiles, found $profileCount"
    $fail++
}

# Idempotent init
Check "re-init is idempotent" { Push-Location $TmpDir; Invoke-Expression "$Anvil init"; Pop-Location }

# Cleanup
Remove-Item -Recurse -Force $TmpDir -ErrorAction SilentlyContinue

Write-Host ""
Write-Host "results: $pass passed, $fail failed"
if ($fail -gt 0) { exit 1 }
