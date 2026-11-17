# Installs Notes for the current user: no administrator rights needed.
#
#   install.ps1                 download the latest release and install it
#   install.ps1 -Build          build from this source folder and install that
#   install.ps1 -Hotkey ''      no shortcut key (default: Ctrl+Alt+N)
#   install.ps1 -Uninstall      remove the app and its shortcut (notes are kept)
#
# Afterwards press the shortcut key anywhere, or press Win and type "Notes".
param(
    [switch]$Build,
    [switch]$Uninstall,
    [string]$Hotkey = 'Ctrl+Alt+N'
)

$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:LOCALAPPDATA 'Programs\Notes'
$exe = Join-Path $dir 'notes.exe'
$link = Join-Path ([Environment]::GetFolderPath('Programs')) 'Notes.lnk'

if ($Uninstall) {
    Remove-Item $link -ErrorAction SilentlyContinue
    try {
        if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }
    } catch {
        throw "Could not remove $dir. Close Notes and run this again."
    }
    Write-Host 'Notes is removed. Your notes folder was not touched.'
    return
}

New-Item -ItemType Directory -Force $dir | Out-Null
$new = Join-Path $dir 'notes.new.exe'
if ($Build) {
    & (Join-Path $PSScriptRoot 'build.ps1')
    Copy-Item (Join-Path $PSScriptRoot 'target\release\notes.exe') $new -Force
} else {
    Write-Host 'Downloading Notes...'
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $url = 'https://github.com/WinThiha/gpui-notes/releases/latest/download/notes.exe'
    $ProgressPreference = 'SilentlyContinue'
    Invoke-WebRequest $url -OutFile $new -UseBasicParsing
}
try {
    Move-Item $new $exe -Force
} catch {
    Remove-Item $new -ErrorAction SilentlyContinue
    throw 'Notes is running. Close it and run this again.'
}

$shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($link)
$shortcut.TargetPath = $exe
$shortcut.WorkingDirectory = $dir
$shortcut.Description = 'Notes'
$shortcut.Hotkey = $Hotkey
$shortcut.Save()

Write-Host "Notes is installed. Find it in the Start menu$(if ($Hotkey) { ", or press $Hotkey" })."
