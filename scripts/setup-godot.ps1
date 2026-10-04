# setup-godot.ps1
#
# Downloads the pinned Godot editor build (see .godot-version), prepares the
# local GdUnit4 addon for the pinned Godot version, and warms the project's
# import/script-class cache so CLI tests can run on a fresh checkout.
#
# Usage:
#   scripts/setup-godot.ps1            # installs Godot and prepares GdUnit4
#   scripts/setup-godot.ps1 -RunTests  # prepares the environment, then runs GdUnit4
#   scripts/setup-godot.ps1 -PrintPath # prints the resolved binary path only
param(
	[switch]$PrintPath,
	[switch]$RunTests,
	[switch]$SkipGdUnit
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$version = (Get-Content (Join-Path $repoRoot ".godot-version") -Raw).Trim()
$installDir = Join-Path $repoRoot ".tools/godot/$version"
$asset = "Godot_v${version}_win64.exe.zip"
$exePath = Join-Path $installDir "Godot_v${version}_win64_console.exe"
$clientDir = Join-Path $repoRoot "client"
$gdUnitDir = Join-Path $clientDir "addons/gdUnit4"

if ($PrintPath) {
	Write-Output $exePath
	exit 0
}

function Receive-Download([string]$Uri, [string]$Path) {
	if (Get-Command curl.exe -ErrorAction SilentlyContinue) {
		& curl.exe --fail --silent --show-error --location --connect-timeout 20 --max-time 180 $Uri --output $Path
		if ($LASTEXITCODE -ne 0) {
			throw "Download failed: $Uri (exit code $LASTEXITCODE)"
		}
	}
	else {
		$ProgressPreference = "SilentlyContinue"
		Invoke-WebRequest -UseBasicParsing -TimeoutSec 180 -Uri $Uri -OutFile $Path
	}
}

function Install-Godot {
	if (Test-Path $exePath) {
		Write-Output "Godot $version already installed: $exePath"
		return
	}

	New-Item -ItemType Directory -Force -Path $installDir | Out-Null
	$tmpDir = Join-Path ([System.IO.Path]::GetTempPath()) ([System.Guid]::NewGuid())
	New-Item -ItemType Directory -Force -Path $tmpDir | Out-Null

	try {
		$baseUrl = "https://github.com/godotengine/godot/releases/download/$version"
		$zipPath = Join-Path $tmpDir $asset
		$sumsPath = Join-Path $tmpDir "SHA512-SUMS.txt"

		Write-Output "Downloading $asset ($version) from godotengine/godot releases ..."
		Receive-Download -Uri "$baseUrl/$asset" -Path $zipPath
		Receive-Download -Uri "$baseUrl/SHA512-SUMS.txt" -Path $sumsPath

		$sumsLines = @(Get-Content -Encoding UTF8 $sumsPath | Where-Object { ($_ -split '\s+')[1] -eq $asset })
		if ($sumsLines.Count -ne 1) {
			throw "Expected exactly one checksum for $asset in SHA512-SUMS.txt"
		}
		$expectedSum = ($sumsLines[0] -split '\s+')[0].ToLowerInvariant()
		$stream = [System.IO.File]::OpenRead($zipPath)
		$hasher = [System.Security.Cryptography.SHA512]::Create()
		try {
			$actualSum = [BitConverter]::ToString($hasher.ComputeHash($stream)).Replace("-", "").ToLowerInvariant()
		}
		finally {
			$hasher.Dispose()
			$stream.Dispose()
		}

		if ($expectedSum -ne $actualSum) {
			throw "SHA512 mismatch for $asset`n  expected: $expectedSum`n  actual:   $actualSum"
		}

		Write-Output "Checksum verified. Extracting ..."
		Expand-Archive -Path $zipPath -DestinationPath $installDir -Force
		if (!(Test-Path -LiteralPath $exePath)) {
			throw "Downloaded archive is missing the console executable: $exePath"
		}

		Write-Output "Installed: $exePath"
	}
	finally {
		$tempRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath()).TrimEnd('\') + '\'
		if (![System.IO.Path]::GetFullPath($tmpDir).StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) {
			throw "Temporary directory is outside the expected temp root: $tmpDir"
		}
		Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
	}
}

function Set-TextIfChanged([string]$Path, [string]$From, [string]$To) {
	if (!(Test-Path $Path)) {
		throw "Required GdUnit4 file is missing: $Path"
	}
	$text = (Get-Content -Raw -Encoding UTF8 -Path $Path).Replace("`r`n", "`n")
	$updated = $text.Replace($From, $To)
	if ($updated -ne $text) {
		Set-Content -Path $Path -Value $updated -NoNewline -Encoding UTF8
		Write-Output "Patched: $Path"
	}
}

function Initialize-GdUnit {
	if ($SkipGdUnit) {
		return
	}
	if (!(Test-Path (Join-Path $gdUnitDir "runtest.cmd"))) {
		throw "GdUnit4 is not installed under client/addons/gdUnit4. Install it from Godot AssetLib, then rerun this script."
	}

	Set-TextIfChanged `
		-Path (Join-Path $gdUnitDir "src/core/GdUnitFileAccess.gd") `
		-From "return file.get_as_text(true)" `
		-To "return file.get_as_text()"

	Set-TextIfChanged `
		-Path (Join-Path $gdUnitDir "plugin.gd") `
		-From 'ProjectSettings.get_setting("debug/gdscript/warnings/exclude_addons")' `
		-To 'ProjectSettings.get_setting("debug/gdscript/warnings/exclude_addons", false)'

	Set-TextIfChanged `
		-Path (Join-Path $gdUnitDir "src/monitor/GodotGdErrorMonitor.gd") `
		-From "var _eof: int`n" `
		-To "var _eof: int = 0`n"
	Set-TextIfChanged `
		-Path (Join-Path $gdUnitDir "src/monitor/GodotGdErrorMonitor.gd") `
		-From "func collect_full_logs() -> PackedStringArray:`n`tawait (Engine.get_main_loop() as SceneTree).process_frame`n`tawait (Engine.get_main_loop() as SceneTree).physics_frame`n`n`tvar file := FileAccess.open(_godot_log_file, FileAccess.READ)`n`tfile.seek(_eof)" `
		-To "func collect_full_logs() -> PackedStringArray:`n`tawait (Engine.get_main_loop() as SceneTree).process_frame`n`tawait (Engine.get_main_loop() as SceneTree).physics_frame`n`n`tvar file := FileAccess.open(_godot_log_file, FileAccess.READ)`n`tif file == null:`n`t`treturn PackedStringArray()`n`tfile.seek(_eof)"
	Set-TextIfChanged `
		-Path (Join-Path $gdUnitDir "src/monitor/GodotGdErrorMonitor.gd") `
		-From "func _collect_log_entries(force_collect_reports: bool) -> Array[ErrorLogEntry]:`n`tvar file := FileAccess.open(_godot_log_file, FileAccess.READ)`n`tfile.seek(_eof)" `
		-To "func _collect_log_entries(force_collect_reports: bool) -> Array[ErrorLogEntry]:`n`tvar file := FileAccess.open(_godot_log_file, FileAccess.READ)`n`tif file == null:`n`t`treturn []`n`tfile.seek(_eof)"

	$logDir = Join-Path $clientDir ".godot-test-logs"
	New-Item -ItemType Directory -Force -Path $logDir | Out-Null

	Set-TextIfChanged `
		-Path (Join-Path $gdUnitDir "runtest.cmd") `
		-From '"!godot_binary!" --path . -s -d res://addons/gdUnit4/bin/GdUnitCmdTool.gd !filtered_args!' `
		-To '"!godot_binary!" --log-file .godot-test-logs\gdunit.log --path . -s -d res://addons/gdUnit4/bin/GdUnitCmdTool.gd !filtered_args!'

	Set-TextIfChanged `
		-Path (Join-Path $gdUnitDir "runtest.cmd") `
		-From '"!godot_binary!" --headless --path . --quiet -s res://addons/gdUnit4/bin/GdUnitCopyLog.gd !filtered_args! > nul' `
		-To '"!godot_binary!" --headless --log-file .godot-test-logs\gdunit-copy.log --path . --quiet -s res://addons/gdUnit4/bin/GdUnitCopyLog.gd !filtered_args! > nul'

	Set-TextIfChanged `
		-Path (Join-Path $gdUnitDir "runtest.sh") `
		-From '"$godot_binary" --path . -s -d res://addons/gdUnit4/bin/GdUnitCmdTool.gd $filtered_args' `
		-To '"$godot_binary" --log-file .godot-test-logs/gdunit.log --path . -s -d res://addons/gdUnit4/bin/GdUnitCmdTool.gd $filtered_args'

	Set-TextIfChanged `
		-Path (Join-Path $gdUnitDir "runtest.sh") `
		-From '"$godot_binary" --headless --path . --quiet -s res://addons/gdUnit4/bin/GdUnitCopyLog.gd $filtered_args > /dev/null' `
		-To '"$godot_binary" --headless --log-file .godot-test-logs/gdunit-copy.log --path . --quiet -s res://addons/gdUnit4/bin/GdUnitCopyLog.gd $filtered_args > /dev/null'

	Write-Output "Importing Godot project and warming script-class cache ..."
	& $exePath --headless --editor --import --path $clientDir --log-file (Join-Path $logDir "import.log")
	$godotExitCode = if ($null -eq $LASTEXITCODE) { 0 } else { [int]$LASTEXITCODE }
	if ($godotExitCode -ne 0) {
		throw "Godot project import failed with exit code $godotExitCode"
	}

	if ($RunTests) {
		Write-Output "Running GdUnit4 tests ..."
		Push-Location $clientDir
		try {
			& (Join-Path $gdUnitDir "runtest.cmd") --godot_binary $exePath -a test
			if ($LASTEXITCODE -ne 0) {
				throw "GdUnit4 tests failed with exit code $LASTEXITCODE"
			}
		}
		finally {
			Pop-Location
		}
	}
}

Install-Godot
Initialize-GdUnit

Write-Output "Godot test environment is ready."
Write-Output "Godot binary: $exePath"
Write-Output "Run tests with:"
Write-Output "  scripts/setup-godot.ps1 -RunTests"
