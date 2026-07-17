# Forge desktop installer. Run: irm https://raw.githubusercontent.com/Adulari/forge/main/install-desktop.ps1 | iex
$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$Repo = 'Adulari/forge'; $headers = @{ 'User-Agent' = 'forge-installer' }
function Die($msg) { Write-Error "install-desktop: $msg"; exit 1 }
function Get-ResponseText($response) {
  # GitHub release text assets can be Byte[] in PowerShell 7 and String in Windows PowerShell 5.
  if ($response.Content -is [byte[]]) { return [Text.Encoding]::UTF8.GetString($response.Content) }
  return [string]$response.Content
}
if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64') { Die "unsupported Windows arch: $env:PROCESSOR_ARCHITECTURE" }
$version = $env:FORGE_VERSION
if (-not $version) { try { $version = (Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest" -Headers $headers).tag_name } catch { Die "could not resolve latest release tag: $($_.Exception.Message)" } }
$asset = "Forge-desktop-windows-x86_64.nsis.exe"; $base = "https://github.com/$Repo/releases/download/$version"; $tmp = Join-Path ([IO.Path]::GetTempPath()) ("forge-desktop-" + [Guid]::NewGuid().ToString('N')); New-Item -ItemType Directory -Force $tmp | Out-Null
try {
  $installer = Join-Path $tmp $asset; Write-Host "install-desktop: downloading $asset $version..."
  try { Invoke-WebRequest "$base/$asset" -OutFile $installer -Headers $headers -UseBasicParsing } catch { Die "download failed: $base/$asset ($($_.Exception.Message))" }
  try { $sums = Invoke-WebRequest "$base/desktop-checksums.txt" -Headers $headers -UseBasicParsing }
  catch { Die "could not download desktop-checksums.txt for $version ($($_.Exception.Message))" }
  $checksumText = Get-ResponseText $sums
  $line = ($checksumText -split "`r?`n" | Where-Object {
    $parts = $_.Trim() -split '\s+'
    $parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $asset
  } | Select-Object -First 1)
  if (-not $line) { Die "desktop-checksums.txt has no entry for $asset" }
  $want = ($line.Trim() -split '\s+')[0].ToLower()
  if ($want -notmatch '^[0-9a-f]{64}$') { Die "desktop-checksums.txt has an invalid SHA-256 for $asset" }
  $got = (Get-FileHash -Path $installer -Algorithm SHA256).Hash.ToLower()
  if ($want -ne $got) { Die "checksum mismatch for $asset" }
  Write-Host 'install-desktop: checksum ok'
  Write-Host 'install-desktop: running installer silently...'
  $p = Start-Process $installer -ArgumentList '/S' -Wait -PassThru
  if ($p.ExitCode -ne 0) { Die "desktop installer exited with $($p.ExitCode)" }
  Write-Host 'install-desktop: Forge desktop installed.'
} finally { Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue }
