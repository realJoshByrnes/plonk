# usage: powershell tools/hwfuzz/run.ps1 [gen|hw|cmp|all] [extra args for cmp, e.g. --slow --all --filter shld]
param([string]$step = "all", [Parameter(ValueFromRemainingArguments = $true)]$rest)
$root = Split-Path -Parent $PSScriptRoot | Split-Path -Parent
$d = Join-Path $PSScriptRoot "out"
$wd = "/mnt/c/Users/jd/Projects/plonk/tools/hwfuzz"
if ($step -eq "gen" -or $step -eq "all") {
    python (Join-Path $PSScriptRoot "gen.py") $d
}
if ($step -eq "hw" -or $step -eq "all") {
    wsl -d Debian -- bash "$wd/build.sh" "$wd/out"
    Push-Location $d; .\harness.exe cases.bin hw.bin code.bin; Pop-Location
}
if ($step -eq "cmp" -or $step -eq "all") {
    Push-Location $root
    cargo run -q --release -p plonk-cpu --example hwfuzz -- tools/hwfuzz/out @rest
    Pop-Location
}
