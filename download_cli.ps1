##############################################################################
# AGIME CLI Install Script for Windows PowerShell
#
# This script downloads the latest stable 'agime' CLI binary from GitHub releases
# and installs it to your system.
#
# Supported OS: Windows
# Supported Architectures: x86_64
#
# Usage:
#   Invoke-WebRequest -Uri "https://github.com/fengrui198609/agime/releases/download/stable/download_cli.ps1" -OutFile "download_cli.ps1"; .\download_cli.ps1
#   Or simply: .\download_cli.ps1
#
# Environment variables:
#   $env:AGIME_BIN_DIR  - Directory to which agime will be installed (default: $env:USERPROFILE\.local\bin)
#   $env:AGIME_VERSION  - Optional: specific version to install (e.g., "v1.0.25"). Can be in the format vX.Y.Z, vX.Y.Z-suffix, or X.Y.Z
#   $env:AGIME_PROVIDER - Optional: provider for agime
#   $env:AGIME_MODEL    - Optional: model for agime
#   $env:CANARY         - Optional: if set to "true", downloads from canary release instead of stable
#   $env:CONFIGURE      - Optional: if set to "false", disables running agime configure interactively
##############################################################################

# Set error action preference to stop on errors
$ErrorActionPreference = "Stop"

# --- 1) Variables ---
# TODO: Update repository to final AGIME GitHub location
$REPO = "fengrui198609/agime"
$OUT_FILE = "agime.exe"

# Set default bin directory if not specified
if (-not $env:AGIME_BIN_DIR) {
    $env:AGIME_BIN_DIR = Join-Path $env:USERPROFILE ".local\bin"
}

# Determine release type
$RELEASE = if ($env:CANARY -eq "true") { "true" } else { "false" }
$CONFIGURE = if ($env:CONFIGURE -eq "false") { "false" } else { "true" }

# Determine release tag
if ($env:AGIME_VERSION) {
    # Validate version format
    if ($env:AGIME_VERSION -notmatch '^v?[0-9]+\.[0-9]+\.[0-9]+(-.*)?$') {
        Write-Error "Invalid version '$env:AGIME_VERSION'. Expected: semver format vX.Y.Z, vX.Y.Z-suffix, or X.Y.Z"
        exit 1
    }
    # Ensure version starts with 'v'
    $RELEASE_TAG = if ($env:AGIME_VERSION.StartsWith("v")) { $env:AGIME_VERSION } else { "v$env:AGIME_VERSION" }
} else {
    # Use canary or stable based on RELEASE variable
    $RELEASE_TAG = if ($RELEASE -eq "true") { "canary" } else { "stable" }
}

# --- 2) Detect Architecture ---
$ARCH = $env:PROCESSOR_ARCHITECTURE
if ($ARCH -eq "AMD64") {
    $ARCH = "x86_64"
} elseif ($ARCH -eq "ARM64") {
    Write-Error "Windows ARM64 is not currently supported."
    exit 1
} else {
    Write-Error "Unsupported architecture '$ARCH'. Only x86_64 is supported on Windows."
    exit 1
}

# --- 3) Build download URL ---
$FILE = "agime-$ARCH-pc-windows-gnu.zip"
$DOWNLOAD_URL = "https://github.com/$REPO/releases/download/$RELEASE_TAG/$FILE"

Write-Host "Downloading $RELEASE_TAG release: $FILE..." -ForegroundColor Green

# --- 4) Download the file ---
try {
    Invoke-WebRequest -Uri $DOWNLOAD_URL -OutFile $FILE -UseBasicParsing
    Write-Host "Download completed successfully." -ForegroundColor Green
} catch {
    Write-Error "Failed to download $DOWNLOAD_URL. Error: $($_.Exception.Message)"
    exit 1
}

# --- 5) Create temporary directory for extraction ---
$TMP_DIR = Join-Path $env:TEMP "agime_install_$(Get-Random)"
try {
    New-Item -ItemType Directory -Path $TMP_DIR -Force | Out-Null
    Write-Host "Created temporary directory: $TMP_DIR" -ForegroundColor Yellow
} catch {
    Write-Error "Could not create temporary extraction directory: $TMP_DIR"
    exit 1
}

# --- 6) Extract the archive ---
Write-Host "Extracting $FILE to temporary directory..." -ForegroundColor Green
try {
    Expand-Archive -Path $FILE -DestinationPath $TMP_DIR -Force
    Write-Host "Extraction completed successfully." -ForegroundColor Green
} catch {
    Write-Error "Failed to extract $FILE. Error: $($_.Exception.Message)"
    Remove-Item -Path $TMP_DIR -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

# Clean up the downloaded archive
Remove-Item -Path $FILE -Force

# --- 7) Determine extraction directory ---
$EXTRACT_DIR = $TMP_DIR
if (Test-Path (Join-Path $TMP_DIR "agime-package")) {
    Write-Host "Found agime-package subdirectory, using that as extraction directory" -ForegroundColor Yellow
    $EXTRACT_DIR = Join-Path $TMP_DIR "agime-package"
}

# --- 8) Create bin directory if it doesn't exist ---
if (-not (Test-Path $env:AGIME_BIN_DIR)) {
    Write-Host "Creating directory: $env:AGIME_BIN_DIR" -ForegroundColor Yellow
    try {
        New-Item -ItemType Directory -Path $env:AGIME_BIN_DIR -Force | Out-Null
    } catch {
        Write-Error "Could not create directory: $env:AGIME_BIN_DIR"
        Remove-Item -Path $TMP_DIR -Recurse -Force -ErrorAction SilentlyContinue
        exit 1
    }
}

# --- 9) Install agime binary ---
$SOURCE_AGIME = Join-Path $EXTRACT_DIR "agime.exe"
$DEST_AGIME = Join-Path $env:AGIME_BIN_DIR $OUT_FILE

if (Test-Path $SOURCE_AGIME) {
    Write-Host "Moving agime to $DEST_AGIME" -ForegroundColor Green
    try {
        # Remove existing file if it exists to avoid conflicts
        if (Test-Path $DEST_AGIME) {
            Remove-Item -Path $DEST_AGIME -Force
        }
        Move-Item -Path $SOURCE_AGIME -Destination $DEST_AGIME -Force
    } catch {
        Write-Error "Failed to move agime.exe to $DEST_AGIME. Error: $($_.Exception.Message)"
        Remove-Item -Path $TMP_DIR -Recurse -Force -ErrorAction SilentlyContinue
        exit 1
    }
} else {
    Write-Error "agime.exe not found in extracted files"
    Remove-Item -Path $TMP_DIR -Recurse -Force -ErrorAction SilentlyContinue
    exit 1
}

# --- 10) Copy Windows runtime DLLs if they exist ---
$DLL_FILES = Get-ChildItem -Path $EXTRACT_DIR -Filter "*.dll" -ErrorAction SilentlyContinue
foreach ($dll in $DLL_FILES) {
    $DEST_DLL = Join-Path $env:AGIME_BIN_DIR $dll.Name
    Write-Host "Moving Windows runtime DLL: $($dll.Name)" -ForegroundColor Green
    try {
        # Remove existing file if it exists to avoid conflicts
        if (Test-Path $DEST_DLL) {
            Remove-Item -Path $DEST_DLL -Force
        }
        Move-Item -Path $dll.FullName -Destination $DEST_DLL -Force
    } catch {
        Write-Warning "Failed to move $($dll.Name): $($_.Exception.Message)"
    }
}

# --- 11) Clean up temporary directory ---
try {
    Remove-Item -Path $TMP_DIR -Recurse -Force
    Write-Host "Cleaned up temporary directory." -ForegroundColor Yellow
} catch {
    Write-Warning "Could not clean up temporary directory: $TMP_DIR"
}

# --- 12) Configure agime (Optional) ---
if ($CONFIGURE -eq "true") {
    Write-Host ""
    Write-Host "Configuring agime" -ForegroundColor Green
    Write-Host ""
    try {
        & $DEST_AGIME configure
    } catch {
        Write-Warning "Failed to run agime configure. You may need to run it manually later."
    }
} else {
    Write-Host "Skipping 'agime configure', you may need to run this manually later" -ForegroundColor Yellow
}

# --- 13) Check PATH and give instructions if needed ---
$CURRENT_PATH = $env:PATH
if ($CURRENT_PATH -notlike "*$env:AGIME_BIN_DIR*") {
    Write-Host ""
    Write-Host "Warning: agime installed, but $env:AGIME_BIN_DIR is not in your PATH." -ForegroundColor Yellow
    Write-Host "To add it to your PATH permanently, run the following command as Administrator:" -ForegroundColor Yellow
    Write-Host "    [Environment]::SetEnvironmentVariable('PATH', `$env:PATH + ';$env:AGIME_BIN_DIR', 'Machine')" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "Or add it to your user PATH (no admin required):" -ForegroundColor Yellow
    Write-Host "    [Environment]::SetEnvironmentVariable('PATH', `$env:PATH + ';$env:AGIME_BIN_DIR', 'User')" -ForegroundColor Cyan
    Write-Host ""
    Write-Host "For this session only, you can run:" -ForegroundColor Yellow
    Write-Host "    `$env:PATH += ';$env:AGIME_BIN_DIR'" -ForegroundColor Cyan
    Write-Host ""
}

Write-Host "agime CLI installation completed successfully!" -ForegroundColor Green
Write-Host "agime is installed at: $DEST_AGIME" -ForegroundColor Green
