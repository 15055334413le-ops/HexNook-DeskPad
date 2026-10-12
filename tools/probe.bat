@echo off
rem 运行一组只读的 probe 验收命令（不启动任何应用），结果写入 tools\probe.log
chcp 65001 >nul
setlocal enabledelayedexpansion
cd /d "%~dp0.."
set LOG=tools\probe.log
rem 自检：tools\req\toggle.flag 存在时，再启动一次 DeskPad（二次启动 = 开/关已运行的启动台），
rem 等 3 秒后把日志复制到 tools\deskpad.log，不跑 probe。
if exist tools\req\toggle.flag (
  del tools\req\toggle.flag
  start "" target\release\DeskPad.exe
  timeout /t 3 /nobreak >nul
  copy /y "%LOCALAPPDATA%\DeskPad\log.txt" tools\deskpad.log >nul
  echo TOGGLED %time% > %LOG%
  exit /b 0
)
set P=target\release\probe.exe
echo === %date% %time% > %LOG%
for %%c in (settings apps recent dirs "search wx" "search ps" "search 微信" "search vsc" icons) do (
  echo. >> %LOG%
  echo ##### probe %%~c >> %LOG%
  %P% %%~c >> %LOG% 2>&1
  echo EXIT=!errorlevel! >> %LOG%
)
echo === done %time% >> %LOG%
