@echo off
setlocal
set "MOD=C:\Program Files (x86)\Steam\steamapps\common\Batman Arkham City GOTY\BmGame\Mods\SplitScreenMod-main"
if exist "%MOD%\mod.toml" goto :turn_off
if exist "%MOD%\mod.toml.off" goto :turn_on
echo The Split-Screen Mod was not found.
goto :end
:turn_off
ren "%MOD%\mod.toml" mod.toml.off
echo Split-Screen Mod is now OFF. Run this again to turn it back on.
goto :end
:turn_on
ren "%MOD%\mod.toml.off" mod.toml
echo Split-Screen Mod is now ON. Run this again to turn it off.
:end
pause
