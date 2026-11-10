param(
    [Parameter(Mandatory = $true)]
    [string]$Destination
)

$ErrorActionPreference = 'Stop'
$ffmpegRoot = Join-Path $Destination 'ffmpeg'
$ffmpegExe = Join-Path $ffmpegRoot 'bin\ffmpeg.exe'
if (Test-Path -LiteralPath $ffmpegExe) {
    exit 0
}

$tempRoot = Join-Path $env:TEMP ('Rewind-FFmpeg-' + [guid]::NewGuid().ToString('N'))
$archive = Join-Path $tempRoot 'ffmpeg.zip'
$extractRoot = Join-Path $tempRoot 'extract'
try {
    [void](New-Item -ItemType Directory -Path $extractRoot -Force)
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri 'https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip' -OutFile $archive -UseBasicParsing
    Expand-Archive -LiteralPath $archive -DestinationPath $extractRoot -Force

    $build = Get-ChildItem -LiteralPath $extractRoot -Directory | Select-Object -First 1
    if (-not $build) {
        throw 'FFmpeg archive did not contain a build directory.'
    }
    $bin = Join-Path $build.FullName 'bin'
    if (-not (Test-Path -LiteralPath (Join-Path $bin 'ffmpeg.exe'))) {
        throw 'FFmpeg archive did not contain bin\ffmpeg.exe.'
    }

    [void](New-Item -ItemType Directory -Path $ffmpegRoot -Force)
    Copy-Item -LiteralPath $bin -Destination $ffmpegRoot -Recurse -Force
    if (-not (Test-Path -LiteralPath $ffmpegExe)) {
        throw 'FFmpeg could not be installed.'
    }
}
finally {
    Remove-Item -LiteralPath $tempRoot -Recurse -Force -ErrorAction SilentlyContinue
}
