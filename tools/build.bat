@echo off
rem DeskPad 构建脚本：结果写入 tools\build.log（Kiki 远程读取用）
rem 构建前先结束正在运行的 DeskPad（否则 exe 被占用无法链接），并把它的日志复制到 tools\deskpad.log。
rem 如果存在 tools\req\run.flag，构建成功后自动启动新的 DeskPad。
chcp 65001 >nul
cd /d "%~dp0.."
set LOG=tools\build.log
echo === %date% %time% > %LOG%
taskkill /im DeskPad.exe /f >nul 2>&1
if exist "%LOCALAPPDATA%\DeskPad\log.txt" copy /y "%LOCALAPPDATA%\DeskPad\log.txt" tools\deskpad.log >nul
cargo build --release --workspace >> %LOG% 2>&1
set BUILD_RC=%errorlevel%
echo BUILD_EXIT=%BUILD_RC% >> %LOG%
if "%BUILD_RC%"=="0" (
  if not exist dist mkdir dist
  copy /y target\release\HN-DeskPad-Setup.exe dist\HN-DeskPad-Setup.exe >nul
)
cargo test --workspace >> %LOG% 2>&1
echo TEST_EXIT=%errorlevel% >> %LOG%
cargo clippy --workspace --release -- -D warnings >> %LOG% 2>&1
echo CLIPPY_EXIT=%errorlevel% >> %LOG%
if exist tools\req\shot.flag (
  del tools\req\shot.flag
  if "%BUILD_RC%"=="0" (
    target\release\DeskPad.exe --render-test tools\shot.bmp
    echo SHOT_EXIT=!errorlevel! >> %LOG%
    target\release\DeskPad.exe --render-test tools\shot-q.bmp --query wx
    copy /y "%LOCALAPPDATA%\DeskPad\log.txt" tools\deskpad.log >nul
  )
)
if exist tools\req\run.flag (
  del tools\req\run.flag
  if "%BUILD_RC%"=="0" (
    start "" target\release\DeskPad.exe
    echo STARTED DeskPad >> %LOG%
  )
)
echo === done %time% >> %LOG%
