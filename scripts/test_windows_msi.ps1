param([Parameter(Mandatory = $true)][string]$Msi)
$ErrorActionPreference = 'Stop'
$taskRepository = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$taskMsi = (Resolve-Path -LiteralPath $Msi).Path
$taskOutput = Join-Path $taskRepository '.runtime_data/msi-qa'
New-Item -ItemType Directory -Force -Path $taskOutput | Out-Null
$taskExisting = @(Get-ItemProperty 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/*' -ErrorAction SilentlyContinue | Where-Object DisplayName -eq 'All-in-One Toolbox')
if ($taskExisting.Count) { throw 'This fixture test requires no existing MSI installation of this app. It will not replace an existing installation with the old test fixture.' }
$taskInstaller = New-Object -ComObject WindowsInstaller.Installer
function Read-MsiProperty($Database, [string]$Name) {
    $taskView = $Database.OpenView("SELECT ``Value`` FROM ``Property`` WHERE ``Property`` = '$Name'")
    $taskView.Execute()
    $taskRecord = $taskView.Fetch()
    if ($taskRecord) { return $taskRecord.StringData(1) }
    return $null
}
function Install-Quietly([string]$Package, [string]$Log) {
    $taskProcess = Start-Process -FilePath "$env:SystemRoot/System32/msiexec.exe" -ArgumentList @('/i', "`"$Package`"", '/qn', '/norestart', 'REBOOT=ReallySuppress', '/L*v', "`"$Log`"") -WindowStyle Hidden -Wait -PassThru
    if ($taskProcess.ExitCode -ne 0) { throw "MSI installation failed: $($taskProcess.ExitCode); log: $Log" }
}
$taskDatabase = $taskInstaller.OpenDatabase($taskMsi, 0)
$taskNewProduct = Read-MsiProperty $taskDatabase 'ProductCode'
$taskUpgradeCode = Read-MsiProperty $taskDatabase 'UpgradeCode'
$taskVersion = Read-MsiProperty $taskDatabase 'ProductVersion'
if ($taskUpgradeCode -ne '{A3A2C967-C2D8-50A2-9DB5-8A8A436126BB}') { throw 'Unexpected upgrade identity' }
if (Read-MsiProperty $taskDatabase 'ALLUSERS') { throw 'Expected a per-user MSI' }
$taskDatabase = $null

$taskFixture = Join-Path $taskOutput 'old-version-fixture.msi'
Copy-Item -LiteralPath $taskMsi -Destination $taskFixture
$taskDatabase = $taskInstaller.OpenDatabase($taskFixture, 1)
$taskOldProduct = '{' + ([guid]::NewGuid().ToString().ToUpperInvariant()) + '}'
foreach ($taskQuery in @("UPDATE ``Property`` SET ``Value`` = '0.0.9' WHERE ``Property`` = 'ProductVersion'", "UPDATE ``Property`` SET ``Value`` = '$taskOldProduct' WHERE ``Property`` = 'ProductCode'")) {
    $taskView = $taskDatabase.OpenView($taskQuery)
    $taskView.Execute()
    $taskView.Close()
}
$taskSummary = $taskDatabase.SummaryInformation(1)
$taskSummary.Property(9) = '{' + ([guid]::NewGuid().ToString().ToUpperInvariant()) + '}'
$taskSummary.Persist()
$taskDatabase.Commit()
$taskSummary = $null
$taskDatabase = $null
[GC]::Collect()
[GC]::WaitForPendingFinalizers()

$taskData = Join-Path $env:APPDATA 'com.allinone.desktop'
New-Item -ItemType Directory -Force -Path $taskData | Out-Null
$taskSentinel = Join-Path $taskData ('release-upgrade-qa-' + [guid]::NewGuid().ToString() + '.txt')
$taskSentinelContents = 'Task-owned MSI upgrade data retention fixture'
Set-Content -LiteralPath $taskSentinel -Value $taskSentinelContents -Encoding UTF8
$taskConfig = Join-Path $taskData 'config.json'
$taskBeforeConfig = if (Test-Path -LiteralPath $taskConfig) { (Get-FileHash -LiteralPath $taskConfig).Hash } else { $null }
try {
    Install-Quietly $taskFixture (Join-Path $taskOutput 'install-old.log')
    $taskOldDirectory = (Get-ItemProperty 'HKCU:/Software/dongyan3721/All-in-One Toolbox').InstallDir
    if ($taskInstaller.ProductState($taskOldProduct) -ne 5) { throw 'Old fixture did not register' }
    Install-Quietly $taskMsi (Join-Path $taskOutput 'upgrade-new.log')
    if ($taskInstaller.ProductState($taskOldProduct) -ne -1) { throw 'Old product remains after major upgrade' }
    if ($taskInstaller.ProductState($taskNewProduct) -ne 5) { throw 'New product is not installed' }
    $taskNewDirectory = (Get-ItemProperty 'HKCU:/Software/dongyan3721/All-in-One Toolbox').InstallDir
    if ($taskOldDirectory -ne $taskNewDirectory) { throw 'Installation directory changed' }
    if (-not (Test-Path -LiteralPath (Join-Path $taskNewDirectory 'all-in-one-desktop.exe'))) { throw 'Installed executable missing' }
    if ((Get-Content -LiteralPath $taskSentinel -Raw).Trim() -ne $taskSentinelContents) { throw 'User data sentinel changed' }
    $taskAfterConfig = if (Test-Path -LiteralPath $taskConfig) { (Get-FileHash -LiteralPath $taskConfig).Hash } else { $null }
    if ($taskBeforeConfig -ne $taskAfterConfig) { throw 'Existing user configuration changed during installation' }
    $taskEntries = @(Get-ItemProperty 'HKCU:/Software/Microsoft/Windows/CurrentVersion/Uninstall/*' -ErrorAction SilentlyContinue | Where-Object DisplayName -eq 'All-in-One Toolbox')
    if ($taskEntries.Count -ne 1) { throw 'Expected exactly one installed product entry' }
    $taskReport = [ordered]@{ version = $taskVersion; upgradeCode = $taskUpgradeCode; perUser = $true; oldProductRemoved = $true; newProductInstalled = $true; quietExitCodes = @(0, 0); sameDirectory = $true; existingConfigurationPreserved = $true; dataSentinelPreserved = $true; installationDirectory = $taskNewDirectory }
    $taskReport | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $taskOutput 'result.json') -Encoding UTF8
    $taskReport | ConvertTo-Json
} finally {
    if (Test-Path -LiteralPath $taskSentinel) { Remove-Item -LiteralPath $taskSentinel }
}
