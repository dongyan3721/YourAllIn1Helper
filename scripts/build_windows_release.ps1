param(
    [string]$SigningKey = (Join-Path $env:LOCALAPPDATA 'YourAllIn1Helper/release-keys/updater.key')
)
$ErrorActionPreference = 'Stop'
if (-not (Test-Path -LiteralPath $SigningKey)) { throw "Updater signing key not found: $SigningKey" }
$taskRepository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Push-Location $taskRepository
try {
    $env:TAURI_SIGNING_PRIVATE_KEY = $SigningKey
    $env:CI = 'true'
    $env:CARGO_BUILD_JOBS = '2'
    pnpm --dir src-ui install --frozen-lockfile
    if ($LASTEXITCODE -ne 0) { throw 'Frontend dependency installation failed' }
    pnpm --dir src-ui exec vue-tsc --noEmit
    if ($LASTEXITCODE -ne 0) { throw 'Frontend type checking failed' }
    pnpm tauri build --bundles msi --ci
    if ($LASTEXITCODE -ne 0) { throw 'MSI build or updater signing failed' }
} finally {
    Remove-Item Env:TAURI_SIGNING_PRIVATE_KEY -ErrorAction SilentlyContinue
    Pop-Location
}
