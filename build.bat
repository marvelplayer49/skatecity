@echo off
setlocal
cd /d "%~dp0"
rem Where Batman: Arkham City is installed. Change this if yours is elsewhere.
set "GAME=C:\Program Files (x86)\Steam\steamapps\common\Batman Arkham City GOTY"

if exist "%GAME%\Binaries\Win32\BatmanAC.exe" goto :game_found
echo Batman: Arkham City was not found at:
echo "%GAME%"
echo Edit the GAME line at the top of this file and run it again.
goto :fail
:game_found

rem The skate engine comes from the 2010-Rust-Rewrite-Mashup source, next to this folder.
if exist "..\2010-rust-rewrite-mashup-src\2010-rust-rewrite-mashup-main\skate\crates\skate-host\Cargo.toml" goto :engine_found
echo The skate engine source was not found. Put the 2010-Rust-Rewrite-Mashup
echo source next to this folder as:
echo   ..\2010-rust-rewrite-mashup-src\2010-rust-rewrite-mashup-main\
echo See README.md.
goto :fail
:engine_found

where cargo >nul 2>nul
if errorlevel 1 set "PATH=%USERPROFILE%\.cargo\bin;%PATH%"
where cargo >nul 2>nul
if not errorlevel 1 goto :cargo_found
echo Rust is not installed. Get it from https://rustup.rs and run this again.
goto :fail
:cargo_found

rem If SkateRim sits next to this folder, share its build so the skate
rem engine isn't compiled twice.
set "CARGO_TARGET_DIR=%~dp0target"
if exist "..\SkateRim\target" set "CARGO_TARGET_DIR=%~dp0..\SkateRim\target"
if not exist Cargo.lock if exist "..\SkateRim\Cargo.lock" copy /y "..\SkateRim\Cargo.lock" Cargo.lock >nul

echo Building the SkateCity helper. The first build compiles the skate
echo engine and can take several minutes.
echo.
cargo build --release > build.log 2>&1
if not errorlevel 1 goto :built
type build.log
echo.
echo BUILD FAILED. The full output is in build.log.
goto :fail
:built

rem SkateCity needs BmSDK 0.21.0 or later (the first that runs on the Steam
rem copy). If its release zips are in the data folder, they're installed here.
set "BMSDK_DONE=%GAME%\Binaries\Win32\sdk\SkateCity_bmsdk_0.21.0.done"
if exist "%BMSDK_DONE%" goto :bmsdk_done
if not exist "data\bmsdk*.zip" goto :bmsdk_manual
echo Installing BmSDK 0.21.0 from the data folder...
for %%z in ("data\bmsdk*.zip") do call :unpack "%%~fz"
if defined BMSDK_FAIL goto :bmsdk_failed
echo done> "%BMSDK_DONE%"
goto :bmsdk_done
:bmsdk_manual
if exist "%GAME%\Binaries\Win32\sdk" goto :bmsdk_done
echo BmSDK is not installed. Install BmSDK 0.21.0 or later into the game
echo (https://github.com/Team-BmSDK/BmSDK-AC/releases), or put its release
echo zips in the data folder as bmsdk*.zip, then run this again.
goto :fail
:bmsdk_failed
echo Could not install BmSDK. Close the game and run this again.
goto :fail
:bmsdk_done

if not exist "%GAME%\Binaries\Win32\SkateCity" mkdir "%GAME%\Binaries\Win32\SkateCity"
copy /y "%CARGO_TARGET_DIR%\release\SkateCity.exe" "%GAME%\Binaries\Win32\SkateCity\SkateCity.exe" >nul
if not errorlevel 1 goto :copied
echo Could not copy SkateCity.exe into the game. Close the game and run this again.
goto :fail
:copied
if not exist "%GAME%\Binaries\Win32\SkateCity\SkateCity.ini" copy /y "game\SkateCity.ini" "%GAME%\Binaries\Win32\SkateCity\SkateCity.ini" >nul
copy /y "game\SkateCity.cs" "%GAME%\BmGame\Scripts\SkateCity.cs" >nul
if not exist "%GAME%\Binaries\Win32\SkateCity\sounds" mkdir "%GAME%\Binaries\Win32\SkateCity\sounds"
if exist "game\sounds\*.wav" copy /y "game\sounds\*.wav" "%GAME%\Binaries\Win32\SkateCity\sounds\" >nul
rem Only one of SkateCity and SpiderCity is on at a time; this turns SkateCity on.
if exist "switch_mods.bat" call switch_mods.bat skate
echo.
echo Done. Start Batman: Arkham City, load your save, wait for
echo "SkateCity ready", then press K or click both sticks in to skate.
pause
exit /b 0

:unpack
tar -xf %1 -C "%GAME%"
if errorlevel 1 set "BMSDK_FAIL=1"
exit /b 0

:fail
pause
exit /b 1
