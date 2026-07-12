# Forge desktop installer. Run: irm https://raw.githubusercontent.com/Adulari/forge/main/install-desktop.ps1 | iex
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$Repo = 'Adulari/forge'; $headers = @{ 'User-Agent' = 'forge-installer' }
function Die($msg) { Write-Error "install-desktop: $msg"; exit 1 }
if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') { Die "unsupported Windows arch: $env:PROCESSOR_ARCHITECTURE" }
$version = $env:FORGE_VERSION
if (-not $version) { try { $version = (Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest" -Headers $headers).tag_name } catch { Die "could not resolve latest release tag: $($_.Exception.Message)" } }
$asset = "Forge-desktop-windows-x86_64.nsis.exe"; $base = "https://github.com/$Repo/releases/download/$version"; $tmp = Join-Path ([IO.Path]::GetTempPath()) ("forge-desktop-" + [Guid]::NewGuid().ToString('N')); New-Item -ItemType Directory -Force $tmp | Out-Null
try {
  $installer = Join-Path $tmp $asset; Write-Host "install-desktop: downloading $asset $version..."
  try { Invoke-WebRequest "$base/$asset" -OutFile $installer -Headers $headers -UseBasicParsing } catch { Die "download failed: $base/$asset ($($_.Exception.Message))" }
  Write-Host 'install-desktop: running installer silently...'
  $p = Start-Process $installer -ArgumentList '/S' -Wait -PassThru
  if ($p.ExitCode -ne 0) { Die "desktop installer exited with $($p.ExitCode)" }
  Write-Host 'install-desktop: Forge desktop installed.'
} finally { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
