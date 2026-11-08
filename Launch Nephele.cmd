@echo off
rem Launch Nephele without cargo or a console.
rem
rem Runs the launcher out of target\<profile>, next to the nephele.exe and
rem nephele-prepare.exe it spawns. Rebuilding with cargo replaces those files in
rem place, so this shortcut keeps working after every edit - there is nothing to
rem re-copy.
rem
rem Usage:  Launch Nephele.cmd            (release build)
rem         Launch Nephele.cmd debug      (debug build, slower game, faster build)

setlocal
cd /d "%~dp0"

set PROFILE=%~1
if "%PROFILE%"=="" set PROFILE=release
if /i "%PROFILE%"=="debug" (
    set CARGO_PROFILE=dev
    set OUTDIR=debug
) else (
    set CARGO_PROFILE=release
    set OUTDIR=release
)

set LAUNCHER=target\%OUTDIR%\nephele-launcher.exe

if exist "%LAUNCHER%" goto run

echo Building the %CARGO_PROFILE% binaries once...
cargo build --profile %CARGO_PROFILE% -p nephele-launcher -p nephele -p nephele-prepare
if errorlevel 1 goto fail

:run
start "" "%LAUNCHER%"
exit /b 0

:fail
echo.
echo Build failed - see the errors above.
pause
exit /b 1
