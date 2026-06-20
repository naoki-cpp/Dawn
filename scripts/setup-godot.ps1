# setup-godot.ps1
#
# Downloads the pinned Godot editor build (see .godot-version) from the
# official godotengine/godot GitHub releases into .tools/godot/<version>/,
# verifying it against the release's published SHA512 checksum. This keeps
# the Godot binary out of git (like a uv/pyenv-managed toolchain) while
# pinning an exact, reproducible version across machines.
#
# Usage:
#   scripts/setup-godot.ps1            # installs the pinned version if missing
#   scripts/setup-godot.ps1 -PrintPath # prints the resolved binary path only
param(
	[switch]$PrintPath
)

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$version = (Get-Content (Join-Path $repoRoot ".godot-version") -Raw).Trim()
$installDir = Join-Path $repoRoot ".tools/godot/$version"
$asset = "Godot_v${version}_win64.exe.zip"
$exePath = Join-Path $installDir "Godot_v${version}_win64.exe"

if ($PrintPath) {
	Write-Output $exePath
	exit 0
}

if (Test-Path $exePath) {
	Write-Output "Godot $version already installed: $exePath"
	exit 0
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
	Write-Output "Set GODOT_BIN to use it, e.g.:"
	Write-Output "  `$env:GODOT_BIN = `"$exePath`""
}
finally {
	Remove-Item -Recurse -Force $tmpDir -ErrorAction SilentlyContinue
}
