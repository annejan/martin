$ErrorActionPreference = 'Stop'

function Invoke-Step {
  param(
    [Parameter(Mandatory = $true)][string]$Exe,
    [Parameter(ValueFromRemainingArguments = $true)][string[]]$Args
  )

  & $Exe @Args
  if ($LASTEXITCODE -ne 0) {
    throw "$Exe failed with exit code $LASTEXITCODE"
  }
}

if (Test-Path Env:RUSTFLAGS) {
  Remove-Item Env:RUSTFLAGS
}
if (Test-Path Env:CARGO_ENCODED_RUSTFLAGS) {
  Remove-Item Env:CARGO_ENCODED_RUSTFLAGS
}
if (Test-Path Env:RUSTDOCFLAGS) {
  Remove-Item Env:RUSTDOCFLAGS
}

Write-Host 'Building wasm (web feature set)...'
Invoke-Step cargo build --target wasm32-unknown-unknown --release --no-default-features --features web

$wasmPath = Join-Path 'target/wasm32-unknown-unknown/release' 'bevy_gaussian_splatting.wasm'
if (-not (Test-Path $wasmPath)) {
  throw "wasm output not found at $wasmPath"
}

if (-not (Get-Command wasm-bindgen -ErrorAction SilentlyContinue)) {
  throw 'wasm-bindgen not found on PATH'
}

Write-Host 'Generating wasm bindings...'
Invoke-Step wasm-bindgen --out-dir ./www/out --target web $wasmPath

Write-Host 'Rendering example thumbnails from manifest...'
$env:RENDER_EXAMPLE_THUMBNAILS = '1'
$env:THUMBNAIL_SORT_MODE = 'std'
try {
  Invoke-Step cargo test --test headless_examples render_example_thumbnails -- --nocapture
} finally {
  Remove-Item Env:THUMBNAIL_SORT_MODE -ErrorAction SilentlyContinue
  Remove-Item Env:RENDER_EXAMPLE_THUMBNAILS -ErrorAction SilentlyContinue
}

if ($env:THUMBNAIL_SCENE_CACHE_CLEANUP -eq '1') {
  $sceneCache = Join-Path 'assets' '.thumbnail_cache'
  if (Test-Path $sceneCache) {
    Write-Host "Cleaning thumbnail scene cache..."
    Remove-Item $sceneCache -Recurse -Force
  }
}

Write-Host 'www build complete.'
