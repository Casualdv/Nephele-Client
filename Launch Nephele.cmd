@echo off
rem Launch Nephele without a console, always from the current source.
rem
rem Builds the three binaries into target\<profile> and starts the launcher from
rem there. The launcher spawns `nephele.exe` and `nephele-prepare.exe` as
rem siblings, so all three are built together and stay in step with the code you
rem just edited - there is nothing to re-copy and no second executable to keep in
rem sync.
rem
rem Usage:  Launch Nephele.cmd            (release build)
rem         Launch Nephele.cmd debug      (debug build: faster to build, slower game)
rem
rem An up-to-date build costs about a second; cargo only recompiles what changed.

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

echo Building (%CARGO_PROFILE%)...
cargo build --profile %CARGO_PROFILE% -p nephele-launcher -p nephele -p nephele-prepare
if errorlevel 1 goto fail

start "" "target\%OUTDIR%\nephele-launcher.exe"
exit /b 0

:fail
echo.
echo Build failed - see the errors above.
pause
exit /b 1
