# Python owns preparation; preserve the existing PowerShell entry point.
param(
    [switch]$PrintPath,
    [switch]$RunTests,
    [switch]$SkipGdUnit
)
$ErrorActionPreference = "Stop"
$arguments = @((Join-Path $PSScriptRoot "godot_setup.py"))
if ($PrintPath) { $arguments += "--print" }
if ($RunTests) { $arguments += "--run-tests" }
if ($SkipGdUnit) { $arguments += "--skip-gdunit" }
if ($env:PYTHON) {
    & $env:PYTHON @arguments
} elseif (Get-Command python -ErrorAction SilentlyContinue) {
    & python @arguments
} elseif (Get-Command py -ErrorAction SilentlyContinue) {
    & py -3 @arguments
} else {
    throw "Python 3.10 or newer is required to prepare Godot."
}
exit $LASTEXITCODE
