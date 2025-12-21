# PowerShell script to run cargo build with VS environment (low parallelism)
$vsPath = "E:\vs"
$vcvarsPath = "$vsPath\VC\Auxiliary\Build\vcvars64.bat"

# Create a batch file that sets up environment and runs cargo build
$tempBat = @"
@echo off
call "$vcvarsPath"
set PATH=%PATH%;E:\yw\agiatme\goose\tools\nasm-2.16.03
set CMAKE_GENERATOR=Ninja
cd /d E:\yw\agiatme\goose
cargo build -p goose-server -p goose-cli -j 4 2>&1
"@

$tempBatPath = "E:\yw\agiatme\goose\tools\temp_build.bat"
$tempBat | Out-File -FilePath $tempBatPath -Encoding ASCII

# Run the batch file and capture output
$output = & cmd.exe /c $tempBatPath 2>&1
$output | Out-File -FilePath "E:\yw\agiatme\goose\tools\build_output.txt" -Encoding UTF8

# Output the result
Write-Host "Build output written to build_output.txt"
Write-Host "Exit code: $LASTEXITCODE"
