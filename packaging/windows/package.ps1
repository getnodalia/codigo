<#
.SYNOPSIS
  Build and package Código for Windows.

.DESCRIPTION
  Produces, in $env:DIST (default: dist/release):
    codigo-<version>-windows-<arch>.msi            per-user installer with a setup wizard (WiX v5)
    codigo-<version>-windows-<arch>-portable.zip   codigo.exe and notices

  The MSI installs for the current user only (no administrator rights), by default into
  %LOCALAPPDATA%\Programs\Nodalia\Codigo, with a Start menu shortcut; installing a newer one
  upgrades in place. It needs the WiX v5 CLI (`dotnet tool install -g wix --version 5.0.2`) and its
  UI and Util extensions (see codigo.wxs).

  The zip is portable: unpack it anywhere and start codigo.exe. The binaries link the C runtime
  statically (+crt-static), so nothing else needs installing. They are not signed (Código has no
  code-signing certificate), so SmartScreen warns the first time they run.

.EXAMPLE
  pwsh packaging/windows/package.ps1 -Arch x64
#>
param(
  [ValidateSet('x64', 'arm64')] [string] $Arch = 'x64',
  [switch] $SkipBuild
)
$ErrorActionPreference = 'Stop'
$Root = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path

function Invoke-Native([string] $What, [scriptblock] $Block) {
  Write-Output "==> $What"
  & $Block
  if ($LASTEXITCODE -ne 0) { throw "$What failed with exit code $LASTEXITCODE" }
}

# The version lives in one place: [workspace.package] version in the root Cargo.toml.
$Version = $env:CODIGO_VERSION
if (-not $Version) {
  $inPkg = $false
  foreach ($line in Get-Content (Join-Path $Root 'Cargo.toml')) {
    if ($line -match '^\s*\[') { $inPkg = ($line.Trim() -eq '[workspace.package]'); continue }
    if ($inPkg -and $line -match '^\s*version\s*=\s*"([^"]+)"') { $Version = $Matches[1]; break }
  }
}
if (-not $Version) { throw 'could not read [workspace.package] version from Cargo.toml' }

$Target = switch ($Arch) { 'x64' { 'x86_64-pc-windows-msvc' } 'arm64' { 'aarch64-pc-windows-msvc' } }
$Dist = if ($env:DIST) { $env:DIST } else { Join-Path $Root 'dist\release' }
$TargetDir = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $Root 'target' }
New-Item -ItemType Directory -Force -Path $Dist | Out-Null

Write-Output "Código $Version for Windows $Arch ($Target)"

if (-not $SkipBuild) {
  # Static CRT: no VC++ redistributable needed. Scoped to the target so build scripts are unaffected.
  $flagVar = 'CARGO_TARGET_' + ($Target.ToUpper() -replace '-', '_') + '_RUSTFLAGS'
  [Environment]::SetEnvironmentVariable($flagVar, '-C target-feature=+crt-static')
  # Fail the build (rather than warn) if the icon and version info can't be embedded.
  $env:CODIGO_REQUIRE_WINRES = '1'
  Invoke-Native "cargo build ($Target)" { cargo build --release --locked -p codigo-app --bins --target $Target }
}
$Bin = Join-Path $TargetDir "$Target\release"

# PE headers: the machine must match -Arch and the app must be a window app (subsystem 2, no
# console opens with it).
function Get-PeHeader([string] $Path) {
  $bytes = [System.IO.File]::ReadAllBytes($Path)
  $pe = [BitConverter]::ToInt32($bytes, 0x3C)
  return @{ Machine = [BitConverter]::ToUInt16($bytes, $pe + 4); Subsystem = [BitConverter]::ToUInt16($bytes, $pe + 0x5C) }
}
$Machine = switch ($Arch) { 'x64' { 0x8664 } 'arm64' { 0xAA64 } }
$h = Get-PeHeader (Join-Path $Bin 'codigo.exe')
if ($h.Machine -ne $Machine) { throw "codigo.exe is for machine 0x$('{0:X}' -f $h.Machine), expected 0x$('{0:X}' -f $Machine) ($Arch)" }
if ($h.Subsystem -ne 2) { throw "codigo.exe has PE subsystem $($h.Subsystem), expected 2 (a window app)" }
Write-Output "ok codigo.exe: $Arch, PE subsystem $($h.Subsystem)"

$Name = "codigo-$Version-windows-$Arch-portable"
$Stage = Join-Path $TargetDir "windows-package\$Name"
Remove-Item -Recurse -Force $Stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Stage | Out-Null
Copy-Item (Join-Path $Bin 'codigo.exe') $Stage
foreach ($f in 'README.md', 'LICENSE-MIT', 'LICENSE-APACHE', 'ATTRIBUTION.md') {
  Copy-Item -LiteralPath (Join-Path $Root $f) -Destination $Stage
}

# ---- MSI ---------------------------------------------------------------------------------------
# The setup wizard's art, drawn here (System.Drawing) from the app icon so the MSI shows Código
# rather than WiX's stock bitmaps. Both must be 24-bit BMPs:
#   - dialog, 493x312: the Welcome and Finish pages' backdrop. Their text sits on the right, so
#     that stays white; the left panel is Código's teal with the icon and the name.
#   - banner, 493x58: across the top of the inner pages. The page title is drawn over its left
#     side, so that stays white; the right end is a teal cell with the icon.
function New-InstallerArt([string] $BannerOut, [string] $DialogOut) {
  Add-Type -AssemblyName System.Drawing
  $teal = [System.Drawing.ColorTranslator]::FromHtml('#0B9A80')
  $icon = [System.Drawing.Image]::FromFile((Join-Path $Root 'assets\icons\hicolor\256x256\apps\dev.danielmiguelt.Codigo.png'))
  $font = New-Object System.Drawing.Font 'Segoe UI Semibold', 22, ([System.Drawing.FontStyle]::Regular), ([System.Drawing.GraphicsUnit]::Pixel)
  try {
    foreach ($spec in @(@($BannerOut, 493, 58), @($DialogOut, 493, 312))) {
      $bmp = New-Object System.Drawing.Bitmap $spec[1], $spec[2], ([System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
      $g = [System.Drawing.Graphics]::FromImage($bmp)
      $panel = New-Object System.Drawing.SolidBrush $teal
      try {
        $g.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
        $g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
        $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
        $g.TextRenderingHint = [System.Drawing.Text.TextRenderingHint]::AntiAliasGridFit
        $g.Clear([System.Drawing.Color]::White)
        if ($spec[2] -eq 58) {
          $g.FillRectangle($panel, 405, 0, 88, 58)
          $g.DrawImage($icon, 431, 11, 36, 36)
        } else {
          $g.FillRectangle($panel, 0, 0, 164, 312)
          $g.DrawImage($icon, 22, 80, 120, 120)
          $white = [System.Drawing.Brushes]::White
          $g.DrawString('Código', $font, $white, 22, 212, [System.Drawing.StringFormat]::GenericTypographic)
        }
      } finally { $panel.Dispose(); $g.Dispose() }
      $bmp.Save($spec[0], [System.Drawing.Imaging.ImageFormat]::Bmp)
      $bmp.Dispose()
    }
  } finally { $font.Dispose(); $icon.Dispose() }
}
# Outside $Stage: the zip below packs that whole folder.
$Work = Join-Path $TargetDir 'windows-package\msi-work'
Remove-Item -Recurse -Force $Work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path $Work | Out-Null
$Banner = Join-Path $Work 'installer-banner.bmp'
$Dialog = Join-Path $Work 'installer-dialog.bmp'
New-InstallerArt $Banner $Dialog

# MSI ProductVersion is numeric (major.minor.build); a pre-release tag is dropped there.
$MsiVersion = ($Version -split '[-+]')[0]
$Msi = Join-Path $Dist "codigo-$Version-windows-$Arch.msi"
Invoke-Native 'wix build' {
  wix build (Join-Path $PSScriptRoot 'codigo.wxs') -arch $Arch `
    -ext WixToolset.UI.wixext -ext WixToolset.Util.wixext `
    -culture en-US -loc (Join-Path $PSScriptRoot 'codigo.en-us.wxl') `
    -d "Version=$MsiVersion" -d "DisplayVersion=$Version" -d "BinDir=$Stage" `
    -d "IconPath=$(Join-Path $Root 'assets\icons\codigo.ico')" `
    -d "BannerBmp=$Banner" -d "DialogBmp=$Dialog" `
    -o $Msi
}
Invoke-Native 'MSI icon validation (ICE50)' {
  wix msi validate $Msi -ice ICE50 -intermediateFolder (Join-Path $Work 'msi-validation')
}
# wix writes its debug symbols (.wixpdb) next to the MSI; keep them out of the release assets.
Remove-Item -Force -ErrorAction SilentlyContinue ([IO.Path]::ChangeExtension($Msi, '.wixpdb'))
Write-Output "wrote $Msi"

# ---- portable zip ------------------------------------------------------------------------------
$Zip = Join-Path $Dist "$Name.zip"
Remove-Item -Force $Zip -ErrorAction SilentlyContinue
Compress-Archive -Path $Stage -DestinationPath $Zip
Write-Output "wrote $Zip"
Get-Item $Zip | Format-Table Name, Length
