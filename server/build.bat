@echo off
rem Compila o fh2-server. Usa o toolchain GNU do Rust (traz o proprio linker, nao precisa do Visual Studio).
where cargo >nul 2>nul || (
  echo Rust nao encontrado. Instale com:  winget install Rustlang.Rustup
  echo Depois rode:  rustup toolchain install stable-x86_64-pc-windows-gnu
  pause & exit /b 1 )
cd /d "%~dp0"
rustup toolchain list | findstr /c:"stable-x86_64-pc-windows-gnu" >nul || rustup toolchain install stable-x86_64-pc-windows-gnu
cargo +stable-x86_64-pc-windows-gnu build --release || ( pause & exit /b 1 )
echo OK: target\release\fh2-server.exe
pause
