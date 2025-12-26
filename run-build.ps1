# Load VS environment and build
$vsPath = "E:\vs\VC\Auxiliary\Build\vcvars64.bat"
$env:CMAKE_GENERATOR = "Ninja"
$env:CC = "cl.exe"
$env:CXX = "cl.exe"
$env:AWS_LC_SYS_PREBUILT_NASM = "1"

# Run vcvars64 and then cargo build
cmd /c "`"$vsPath`" && cargo build --release -p agime-cli -p agime-server 2>&1"
