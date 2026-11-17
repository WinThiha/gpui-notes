# Builds target\release\notes.exe.
#
# Use this instead of a bare "cargo build --release" on a PC without the
# Windows SDK: it first builds tools\fxc.rs, a stand-in for the SDK's shader
# compiler, which .cargo\config.toml points the build at.
$ErrorActionPreference = 'Stop'
Set-Location $PSScriptRoot

$fxc = 'target\tools\fxc.exe'
if (-not (Test-Path $fxc) -or (Get-Item $fxc).LastWriteTime -lt (Get-Item 'tools\fxc.rs').LastWriteTime) {
    New-Item -ItemType Directory -Force 'target\tools' | Out-Null
    rustc --edition 2024 -O 'tools\fxc.rs' -o $fxc
    if ($LASTEXITCODE -ne 0) { throw 'could not build tools\fxc.rs' }
}

cargo build --release
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
