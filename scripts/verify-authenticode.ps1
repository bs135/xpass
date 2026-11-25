# Checks the Authenticode signatures of a release build (v0.7 plan U8).
#
# Every installer, and every .exe inside the NSIS installer (xpass.exe and the
# uninstaller, extracted with 7-Zip), must be signed by the certificate with the
# expected thumbprint and carry a timestamp. The certificate is self-signed, so
# Windows reports the chain as not trusted (UnknownError); that status is expected,
# any other failure (no signature, hash mismatch) is not.
#
# Usage: pwsh scripts/verify-authenticode.ps1 -Thumbprint <sha1> -Path <file or dir> [...]
param(
    [Parameter(Mandatory)] [string] $Thumbprint,
    [Parameter(Mandatory)] [string[]] $Path
)
$ErrorActionPreference = 'Stop'
$expected = $Thumbprint.ToUpperInvariant()

function Test-Signature([string] $File) {
    $sig = Get-AuthenticodeSignature -LiteralPath $File
    $name = Split-Path $File -Leaf
    if ($sig.Status -notin 'Valid', 'UnknownError') {
        throw "$name`: signature status $($sig.Status) ($($sig.StatusMessage))"
    }
    if (-not $sig.SignerCertificate -or $sig.SignerCertificate.Thumbprint -ne $expected) {
        throw "$name`: signed by $($sig.SignerCertificate.Thumbprint), expected $expected"
    }
    if (-not $sig.TimeStamperCertificate) {
        throw "$name`: no timestamp"
    }
    "signed: $name ($($sig.Status), timestamp by $($sig.TimeStamperCertificate.Subject))"
}

$files = foreach ($p in $Path) {
    if (Test-Path -LiteralPath $p -PathType Container) {
        Get-ChildItem -LiteralPath $p -Recurse -File -Include *.exe, *.msi
    } else {
        Get-Item -LiteralPath $p
    }
}
if (-not $files) { throw 'no installers found' }

$sevenZip = (Get-Command 7z -ErrorAction SilentlyContinue).Source
$count = 0
foreach ($file in $files) {
    Test-Signature $file.FullName
    $count++
    if ($file.Name -like '*-setup.exe') {
        if (-not $sevenZip) { throw '7z is needed to check the files inside the NSIS installer' }
        $dir = Join-Path ([IO.Path]::GetTempPath()) ("xpass-nsis-" + [guid]::NewGuid())
        & $sevenZip x -y "-o$dir" $file.FullName '*.exe' -r | Out-Null
        if ($LASTEXITCODE -ne 0) { throw "7z could not extract $($file.Name)" }
        $inner = Get-ChildItem $dir -Recurse -File -Filter *.exe
        foreach ($name in 'xpass.exe', 'uninstall.exe') {
            $match = $inner | Where-Object Name -eq $name | Select-Object -First 1
            if (-not $match) { throw "$($file.Name) contains no $name" }
            Test-Signature $match.FullName
            $count++
        }
        Remove-Item $dir -Recurse -Force
    }
}
"Authenticode OK: $count files signed by $expected"
