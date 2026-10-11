# SkateCity

Skate 3's skating in **Batman: Arkham City**. Press a key and Batman drops onto a
board: push, ollie, flip tricks, grind rooftop edges and railings, bail, glide off
buildings with his cape and land back on the board.

The skating is the real Skate 3 simulation: the physics, trick system and animation
run in the skate engine from the
[2010-Rust-Rewrite-Mashup](#what-you-need) project, and Batman is posed from the
skater every frame.

> Fan project. Not affiliated with or endorsed by Warner Bros., Rocksteady or
> Electronic Arts. No game files are included: you need your own copy of
> Batman: Arkham City, and Skate 3 data dumped from your own disc.

## Features

- Skate anywhere in Arkham City. The ground around you is scanned as you ride, so
  streets, rooftops, stairs and ramps are all skateable.
- Grind rooftop edges, ledges, curbs, benches and railings. Rails are found
  automatically.
- Skate 3's full trick set: flip tricks, grabs, grinds, slides, manuals, powerslides.
- Front flips and backflips off anything tall enough.
- Glide: press LB in the air to open the cape (Arkham's own glide), and you're put
  back on the board when you land.
- Jump or bail off buildings and ragdoll all the way to the street.
- Trick display: live combo and points, a CLEAN / SKETCHY / BAILED result banner
  and a session score.

## Controls

Skating needs an Xbox-style controller.

| Action | Control |
| --- | --- |
| Start / stop skating | **K**, or click both sticks in |
| Skate | Skate 3's controls (A push, right stick flicks for tricks, LT/RT grabs, ...) |
| Glide (in the air) | **LB**; **LB** again to drop |
| Front flip / backflip (in the air) | Hold **RB**, push the left stick forward / back |

## What you need

- **Batman: Arkham City Game of the Year Edition** (Steam).
- **[BmSDK](https://github.com/Team-BmSDK/BmSDK-AC) 0.21.0 or later**, the C#
  scripting SDK for the game. 0.21.0 is the first version that runs on the Steam
  copy. Install it into the game, or put its release zips in a `data` folder here
  as `bmsdk*.zip` and `build.bat` installs them for you.
- **The 2010-Rust-Rewrite-Mashup skate engine.** You need two parts of it:
  - its source, placed next to this folder as
    `..\2010-rust-rewrite-mashup-src\2010-rust-rewrite-mashup-main\`, which
    `build.bat` compiles against;
  - its release with your Skate 3 data, unzipped in `Downloads` as
    `2010-Rust-Rewrite-Mashup-windows-x64\2010-Rust-Rewrite-Mashup\skate-data\assets`.
    If it's somewhere else, set `skate_assets` in `SkateCity.ini`.
- **Rust**, from [rustup.rs](https://rustup.rs).
- If you use the Split-Screen Mod, turn it off: it crashes alongside SkateCity.
  `splitscreen_on_off.bat` switches it.

The game runs best in DirectX 9 with SkateCity. If it crashes on the menu, set
`AllowD3D11=False` in
`Documents\WB Games\Batman Arkham City GOTY\BmGame\Config\BmEngine.ini`.

## Install

1. Get everything in [What you need](#what-you-need).
2. If the game isn't in the default Steam folder, edit the `GAME` line at the top
   of `build.bat`.
3. Run `build.bat`. It builds the helper (`SkateCity.exe`) and copies it, the game
   script and the settings into the game.
4. Start the game, load your save and wait for **SkateCity ready**, then press **K**.

## Settings

The installed settings file is at
`Binaries\Win32\SkateCity\SkateCity.ini` in the game folder. It covers:

- the toggle key,
- how far around you the ground is scanned,
- how many traces the scan may use per frame (lower it if the game stutters),
- whether to find rails,
- Batman's size relative to the skater,
- the board's materials,
- the trick display.

Restart the game after changing it.

## How it works

Arkham City is a 32-bit game and the skate engine is 64-bit, so they run in two
processes that share memory:

- **`game/SkateCity.cs`** is a BmSDK script that runs inside the game. Every frame
  it does four things:
  - reports Batman's position and skeleton to the helper;
  - casts the ray traces the helper asks for, to measure the ground;
  - applies the reply: where Batman goes, his bone pose, the camera, the board pieces;
  - draws the trick display.
- **`SkateCity.exe`** (the Rust code in `src/`) runs the Skate 3 simulation.
  - `scan.rs` turns the ray results into a height grid and collision for the
    engine, finds grindable edges and railings, and reuses earlier scans so only
    new ground is traced.
  - `rig.rs` retargets the skater's pose onto Batman's skeleton.
  - `mode.rs` runs each frame: starting and stopping, streaming the ground,
    glides, flips, the fall rescue and scoring.
  - `ipc.rs` is the shared-memory layout. It must match the offsets in
    `SkateCity.cs`.

## Logs

If something goes wrong, check these logs in the game's `Binaries\Win32` folder:

- `SkateCity\SkateCity.log` is the helper's log.
- `BmSDK.log` has the game script's messages.
