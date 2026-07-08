@echo off
set WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222 --disable-http-cache --disable-cache
echo Launching FileBitch with CDP port 9222...
start "" "%~dp0target\debug\filebitch.exe"
