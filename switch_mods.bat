@echo off
setlocal
rem Keeps SpiderCity and SkateCity apart: only one of them is on at a time.
rem The other is parked in BmGame\ModsOff. Close the game first.
rem   switch_mods.bat          asks which one
rem   switch_mods.bat spider   SpiderCity on, SkateCity off
rem   switch_mods.bat skate    SkateCity on, SpiderCity off
set "GAME=C:\Program Files (x86)\Steam\steamapps\common\Batman Arkham City GOTY"
set "OFF=%GAME%\BmGame\ModsOff"
set "SKATE=%GAME%\BmGame\Scripts\SkateCity.cs"
set "SPIDER=%GAME%\BmGame\Mods\SpiderCity"

if not exist "%GAME%\Binaries\Win32\BatmanAC.exe" (
  echo Batman: Arkham City was not found at "%GAME%".
  echo Edit the GAME line at the top of this file.
  goto :end
)
if not exist "%OFF%" mkdir "%OFF%"

set "PICK=%~1"
if /i "%PICK%"=="spider" goto :spider
if /i "%PICK%"=="skate" goto :skate
echo Which mod should be on?
echo   1 = SpiderCity
echo   2 = SkateCity
choice /c 12 /n /m "Press 1 or 2: "
if errorlevel 2 goto :skate

:spider
if exist "%SKATE%" move /y "%SKATE%" "%OFF%\SkateCity.cs" >nul
if not exist "%SPIDER%" if exist "%OFF%\SpiderCity" move "%OFF%\SpiderCity" "%SPIDER%" >nul
echo SpiderCity is on. SkateCity is off.
goto :end

:skate
if exist "%SPIDER%" (
  if exist "%OFF%\SpiderCity" rmdir /s /q "%OFF%\SpiderCity"
  move "%SPIDER%" "%OFF%\SpiderCity" >nul
)
if not exist "%SKATE%" if exist "%OFF%\SkateCity.cs" move /y "%OFF%\SkateCity.cs" "%SKATE%" >nul
echo SkateCity is on. SpiderCity is off.

:end
if "%~1"=="" pause
