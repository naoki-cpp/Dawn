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
$exePath = Join-Path $installDir "Godot_v${version}_win64.exe"
$clientDir = Join-Path $repoRoot "client"
$gdUnitDir = Join-Path $clientDir "addons/gdUnit4"

if ($PrintPath) {
	Write-Output $exePath
	exit 0
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
		Invoke-WebRequest -Uri "$baseUrl/$asset" -OutFile $zipPath
		Invoke-WebRequest -Uri "$baseUrl/SHA512-SUMS.txt" -OutFile $sumsPath

		$sumsLine = Select-String -Path $sumsPath -Pattern ([regex]::Escape($asset)) | Select-Object -First 1
		if ($null -eq $sumsLine) {
			throw "Could not find a checksum for $asset in SHA512-SUMS.txt"
		}
		$expectedSum = ($sumsLine.Line -split '\s+')[0].ToLower()
		$actualSum = (Get-FileHash -Path $zipPath -Algorithm SHA512).Hash.ToLower()

		if ($expectedSum -ne $actualSum) {
			throw "SHA512 mismatch for $asset`n  expected: $expectedSum`n  actual:   $actualSum"
		}

		Write-Output "Checksum verified. Extracting ..."
		Expand-Archive -Path $zipPath -DestinationPath $installDir -Force

		Write-Output "Installed: $exePath"
	}
	finally {
		Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
	}
}

function Set-TextIfChanged([string]$Path, [string]$From, [string]$To) {
	if (!(Test-Path $Path)) {
		throw "Required GdUnit4 file is missing: $Path"
	}
	$text = Get-Content -Raw -Path $Path
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
	& $exePath --headless --editor --quit-after 3 --path $clientDir
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
