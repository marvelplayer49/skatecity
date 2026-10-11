# SkateCity

Skate 3's skating in **Batman: Arkham City**. Press a key and Batman drops onto a
board: push, ollie, flip tricks, grind rooftop edges and railings, bail, glide off
buildings with his cape and land back on the board.

The skating is the real Skate 3 simulation: the physics, trick system and animation
run in the skate engine from the
[2010-Rust-Rewrite-Mashup](https://github.com/chasmlol/2010-rust-rewrite-mashup) project, and Batman is posed from the
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
- **The [2010-Rust-Rewrite-Mashup](https://github.com/chasmlol/2010-rust-rewrite-mashup) skate engine** by chasmlol. You need two
  parts of it:
  - its source, placed next to this folder as
    `..\2010-rust-rewrite-mashup-src\2010-rust-rewrite-mashup-main\`, which
    `build.bat` compiles against;
  - its Windows release (`IW4L-Skate-windows-x64.zip`), set up once with your
    Skate 3 data so it makes its `skate-data` folder.
- **Skate 3 for Xbox 360**: your own copy's extracted `default.xex` and its `data`
  folder (not the ISO). The mashup's setup copies out only what skating needs.
- **Modern Warfare 2** (Steam), because the mashup's one-time setup asks for it
  before it sets up Skate 3.
- **Rust**, from [rustup.rs](https://rustup.rs).
- If you use the Split-Screen Mod, turn it off: it crashes alongside SkateCity.
  `splitscreen_on_off.bat` switches it.

The game runs best in DirectX 9 with SkateCity. If it crashes on the menu, set
`AllowD3D11=False` in
`Documents\WB Games\Batman Arkham City GOTY\BmGame\Config\BmEngine.ini`.

## Install, step by step

These steps assume Windows, the Steam copy of the game, and that you're putting
everything in your `Downloads` folder. Other places work too; see the notes in
each step.

### 1. Set up the game

1. Install **Batman: Arkham City Game of the Year Edition** from Steam.
2. Start it once, get past the menus into the game, then quit. This creates the
   game's settings files.
3. If you use the **Split-Screen Mod**, turn it off. It crashes when SkateCity is
   also running. `splitscreen_on_off.bat` in this repo switches it on and off.

### 2. Install Rust

SkateCity's helper is written in Rust and is built on your PC.

1. Go to [rustup.rs](https://rustup.rs) and download `rustup-init.exe`.
2. Run it and accept the defaults. If it offers to install the **Visual Studio C++
   build tools**, say yes: Rust needs them on Windows.
3. Close and reopen any open command windows afterwards.

### 3. Install BmSDK

[BmSDK](https://github.com/Team-BmSDK/BmSDK-AC) lets the game run C# scripts, and
SkateCity's game side is one of those scripts.

1. Open BmSDK's [releases page](https://github.com/Team-BmSDK/BmSDK-AC/releases)
   and download **version 0.21.0 or newer**. Older versions don't run on the
   Steam copy.
2. Install it as its own instructions say. It goes into the game's main folder,
   the one that contains `Binaries` and `BmGame`. On Steam that's
   `C:\Program Files (x86)\Steam\steamapps\common\Batman Arkham City GOTY`.

   Or skip installing it yourself: in step 5, make a `data` folder inside the
   SkateCity folder, put the BmSDK release zips there named `bmsdk0.zip`,
   `bmsdk1.zip`, … and `build.bat` installs them for you.

### 4. Get the Skate 3 engine and your Skate 3 data

The skating comes from chasmlol's
**[2010-Rust-Rewrite-Mashup](https://github.com/chasmlol/2010-rust-rewrite-mashup)** project, which you need in two forms.

**Its source code**, which SkateCity is built against:

1. On [its GitHub page](https://github.com/chasmlol/2010-rust-rewrite-mashup), click **Code** → **Download ZIP**.
2. Make a folder called `2010-rust-rewrite-mashup-src` in `Downloads` and unzip
   the ZIP into it, so this folder exists:
   `Downloads\2010-rust-rewrite-mashup-src\2010-rust-rewrite-mashup-main\`

**Its Windows release, set up with your Skate 3 data**, which SkateCity reads the
skater, animations and physics settings from:

1. You need **Skate 3 for Xbox 360**, extracted from your own copy: its
   `default.xex` with the `data` folder next to it. An ISO file on its own doesn't
   work. The project's
   [Skate 3 notes](https://github.com/chasmlol/2010-rust-rewrite-mashup/blob/main/docs/SKATE.md) show the layout.
2. From the project's [Releases page](https://github.com/chasmlol/2010-rust-rewrite-mashup/releases/latest), download
   `IW4L-Skate-windows-x64.zip`.
3. Make a folder `Downloads\2010-Rust-Rewrite-Mashup-windows-x64\2010-Rust-Rewrite-Mashup`
   and extract the zip into it. Any folder you can write to works (not Program
   Files), but this one needs no extra settings later.
4. Double-click `iw4l.exe` in that folder. It asks for your MW2 folder, then
   whether you have Skate 3: choose **Yes** and select your `default.xex`. It
   copies what skating needs into a `skate-data` folder next to `iw4l.exe`.
5. Close it once the menu opens. You should now have:
   `Downloads\2010-Rust-Rewrite-Mashup-windows-x64\2010-Rust-Rewrite-Mashup\skate-data\assets\`

If you put the release somewhere else, you'll point SkateCity at its
`skate-data\assets` folder in step 8.

### 5. Download SkateCity

1. At the top of this page, click **Code** → **Download ZIP**.
2. Unzip it into `Downloads`, and rename the folder it makes (`skatecity-main`)
   to `SkateCity`.

Your `Downloads` folder should now look like this:

```
Downloads\
├── SkateCity\                              <- this repo
│   ├── build.bat
│   ├── game\
│   └── src\
├── 2010-rust-rewrite-mashup-src\
│   └── 2010-rust-rewrite-mashup-main\      <- engine source
└── 2010-Rust-Rewrite-Mashup-windows-x64\
    └── 2010-Rust-Rewrite-Mashup\
        └── skate-data\assets\              <- your Skate 3 data
```

SkateCity and the engine source must sit side by side like this, because the
build finds the engine through `..\2010-rust-rewrite-mashup-src`.

### 6. If your game isn't in the default Steam folder

Right-click `build.bat` → **Edit** and change the `GAME` line at the top to your
game folder (the one containing `Binaries` and `BmGame`). Save it.

### 7. Build and install

1. Double-click `build.bat` in the SkateCity folder.
2. Wait. The **first build compiles the whole skate engine and can take 5–15
   minutes**; later builds take a minute or two.
3. When it says **Done**, press a key to close it.

`build.bat` builds `SkateCity.exe` and copies it, the game script and the
settings into the game. If it says **BUILD FAILED**, the full error is in
`build.log` in the SkateCity folder.

### 8. Point SkateCity at your Skate 3 data (only if it's not in Downloads)

Open `Binaries\Win32\SkateCity\SkateCity.ini` inside the game folder. Find the
`skate_assets` line, remove the `;` at its start, and give the full path to your
`skate-data\assets` folder. Save it.

### 9. Play

1. Plug in an Xbox-style controller.
2. Start Batman: Arkham City from Steam and load your save.
3. Wait for the message **SkateCity ready** in the lower left. Loading the Skate
   3 data takes a few seconds the first time.
4. Press **K** on the keyboard, or click both sticks in, to get on the board.
   Press it again to get off. The full controls are in [Controls](#controls).

### If the game crashes

- **On the menu or on loading:** switch the game to DirectX 9. Open
  `Documents\WB Games\Batman Arkham City GOTY\BmGame\Config\BmEngine.ini`, find
  `AllowD3D11` and set it to `AllowD3D11=False`.
- **With other mods installed:** turn them off and try again. The Split-Screen
  Mod is known to crash alongside SkateCity.
- **Otherwise:** check the [logs](#logs).

### Updating

Download the new ZIP, replace the files in your SkateCity folder, and run
`build.bat` again. Your settings file in the game folder is kept.

### Uninstalling

Delete these from the game folder:

- `Binaries\Win32\SkateCity\` (the whole folder)
- `BmGame\Scripts\SkateCity.cs`

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

## Credits

- The skate engine is [2010-Rust-Rewrite-Mashup](https://github.com/chasmlol/2010-rust-rewrite-mashup) by chasmlol, a fork of
  [IW4L](https://github.com/vladtrc/iw4L) by vladtrc (Apache License 2.0).
  SkateCity builds against it rather than bundling it.
- [BmSDK](https://github.com/Team-BmSDK/BmSDK-AC) by Team-BmSDK runs the game script.
