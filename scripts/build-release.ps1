# Builds a single, portable ttask.exe for Windows x64.
#
# - Release profile: LTO, one codegen unit, stripped (see Cargo.toml).
# - The C runtime is linked statically (+crt-static), so the exe does not
#   need vcruntime140.dll; SQLCipher and OpenSSL are compiled in as well.
#
# Output: dist\ttask-<version>-x86_64-pc-windows-msvc.exe plus a .sha256 file.
# Requires Rust (MSVC toolchain), Visual Studio build tools, Strawberry Perl.

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

if (-not $env:OPENSSL_SRC_PERL) {
    $strawberry = 'C:\Strawberry\perl\bin\perl.exe'
    if (Test-Path $strawberry) { $env:OPENSSL_SRC_PERL = $strawberry }
    else { throw 'Set OPENSSL_SRC_PERL to a native Windows perl (winget install StrawberryPerl.StrawberryPerl)' }
}

$target = 'x86_64-pc-windows-msvc'
# A separate target dir keeps the static-CRT build from invalidating the dev build.
$targetDir = Join-Path $root 'target\portable'
$flags = '["-C", "target-feature=+crt-static", "-C", "link-arg=/IGNORE:4099"]'

cargo build --release --locked -p ttask --target $target --target-dir $targetDir `
    --config "target.$target.rustflags=$flags"
if ($LASTEXITCODE -ne 0) { throw "cargo build failed ($LASTEXITCODE)" }

$version = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).packages |
    Where-Object name -eq 'ttask' | Select-Object -ExpandProperty version
$built = Join-Path $targetDir "$target\release\ttask.exe"
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Force $dist | Out-Null
$out = Join-Path $dist "ttask-$version-$target.exe"
Copy-Item $built $out -Force

$hash = (Get-FileHash $out -Algorithm SHA256).Hash.ToLower()
"$hash  $(Split-Path -Leaf $out)" | Set-Content -Encoding ascii "$out.sha256"

$size = [math]::Round((Get-Item $out).Length / 1MB, 1)
Write-Host "Built $out ($size MB)"
Write-Host "SHA256 $hash"
