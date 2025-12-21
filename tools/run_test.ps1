# PowerShell script to run cargo test with VS environment (low parallelism)
$vsPath = "E:\vs"
$vcvarsPath = "$vsPath\VC\Auxiliary\Build\vcvars64.bat"

# Create a batch file that sets up environment and runs cargo test
$tempBat = @"
@echo off
call "$vcvarsPath"
set PATH=%PATH%;E:\yw\agiatme\goose\tools\nasm-2.16.03
set CMAKE_GENERATOR=Ninja
cd /d E:\yw\agiatme\goose
cargo test -p goose -j 4 2>&1
"@

$tempBatPath = "E:\yw\agiatme\goose\tools\temp_test.bat"
$tempBat | Out-File -FilePath $tempBatPath -Encoding ASCII

# Run the batch file and capture output
$output = & cmd.exe /c $tempBatPath 2>&1
$output | Out-File -FilePath "E:\yw\agiatme\goose\tools\test_output.txt" -Encoding UTF8

# Output the result
Write-Host "Test output written to test_output.txt"
Write-Host "Exit code: $LASTEXITCODE"
