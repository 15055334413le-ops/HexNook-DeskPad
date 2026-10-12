@echo off
rem DeskPad 构建监听：只执行本目录下固定的 build.bat / probe.bat，结果写入 tools\*.log。
rem 关闭这个窗口即停止。
chcp 65001 >nul
cd /d "%~dp0.."
title DeskPad 构建监听（关闭窗口即停止）
echo DeskPad 构建监听已启动，只会运行 tools\build.bat 和 tools\probe.bat。
:loop
if exist tools\req\build.flag (
  del tools\req\build.flag
  echo [%time%] 开始构建...
  call tools\build.bat
  echo [%time%] 构建结束，见 tools\build.log
)
if exist tools\req\probe.flag (
  del tools\req\probe.flag
  echo [%time%] 运行 probe...
  call tools\probe.bat
  echo [%time%] probe 结束，见 tools\probe.log
)
timeout /t 2 /nobreak >nul
goto loop
