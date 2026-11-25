# VIctory, RTX 3060 build (work in progress)

A build of [fxceless/victory-fork](https://github.com/fxceless/victory-fork) for a **mid-range NVIDIA Ampere card (RTX 3060 12 GB)**. That fork is itself built on [VIctory](https://github.com/Marvisak/VIctory) by Marvisak and its contributors, a compatibility layer that runs an Xbox Series title natively on Windows from files the user supplies.

Here I measure what each switch and pull request does on a 12 GB, 32-lane-wave card, fix what breaks, and report findings and fixes back upstream.

**This repository contains no game files, code, assets or captures.** You need your own copy of the game. It is not affiliated with or endorsed by the game's developer or publisher. Licence: GPL-3.0-or-later, as upstream.

## State (2026-11-25)

**Not playable yet in the usual sense: about 3 frames per second.** Tested on one machine: RTX 3060 12 GB (driver 621.40), Ryzen 7 5700X, 32 GB RAM, Windows 11, Series X profile.

| What | State on that machine |
|------|-----------------------|
| Boot, menus, loading | work |
| Prologue | cutscene and the cell block render lit and textured: people, hair, posters, sunlight through windows |
| Controls | keyboard and mouse (fxceless#577), gamepad |
| Frame rate in the first rooms | 3.0 fps steady (330 ms a frame; GPU ~207 ms, copy-back ~216 ms) |
| Stability | no device loss in the runs on this branch (6 runs, up to 12 minutes each) |
| Shaders | 232 of 25,985 do not translate |

Known problems:
- Since the paired wave64 form (fxceless#854) about 230 more programs run, and GPU time per frame roughly doubled compared with an older base (114 → 207 ms). The picture has more in it; the frame is slower.
- A white frame for a moment between the opening cutscene and gameplay (~80 draws dropped for pipeline creation methods not implemented yet).
- No ray-traced light: shaded places are flat; a few bright rectangles of light near bunk beds.
- GPU time does not depend on resolution here: telling the game it has a 1080p display (`VICTORY_DISPLAY_MODE=1920x1080`) instead of the default 4K changed the frame by 2 %. The GPU's time goes to per-draw work (indirect argument clamps and barriers), not pixels; `VICTORY_INDIRECT_WINDOW=1` is next.

## What this branch is

Branch `rtx3060` = fxceless main `a253706` merged into the pull request stack fxceless#770 (`d034ac1`), plus two commits:
1. **A fix for `VICTORY_WIDE_TEXTURES=1` with levels on demand** (`704f317`, with a test): a queued texture check ran after the host step that maps its tiles, so some textures came out black or stale. Reported with the fix in [fxceless#768](https://github.com/fxceless/victory-fork/pull/768#issuecomment-7163763533).
2. **A port of Marvisak/VIctory#2230** (`5494ad0`): a key tap between two reads of the current input state is not lost. Ported onto fxceless#577's keyboard and mouse controls, with a test of how the two work together. fxceless#640 does the same and may replace it.

## How it is run here

```
cargo build --release --locked -p victory
set VICTORY_HOST_WAVE=32
set VICTORY_VIEW_LIMIT=999999
target\release\victory.exe translate --warm all --jobs 16
```
(The two variables make the offline cache keys match an NVIDIA card: 32-lane waves and NVIDIA's 1,000,000-view heap.)

```
set VICTORY_LEVELS_ON_DEMAND=1
set VICTORY_EARLY_TEXTURES_MS=40
set VICTORY_TEXTURE_NO_WAIT=1
set VICTORY_WIDE_TEXTURES=1
target\release\victory.exe run <game_scarlett_master.exe> --execute
```

## What the switches did on an RTX 3060

Medians of steady frames, same save, first rooms; details of every run are kept with the measurements.

| Switch or change | Effect here |
|------------------|-------------|
| `LEVELS_ON_DEMAND=1` | lit rooms and textures; ~3 GB less video memory. Needed on 12 GB |
| loose-LDS fallback of fxceless#799 (on) | posterized picture (bands on walls and skin) and +45 % GPU time; reported in [fxceless#799](https://github.com/fxceless/victory-fork/pull/799#issuecomment-7162619618) |
| `INDIRECT_KEEP=1` (fxceless#780) | device hung after 2 minutes (that pull request is closed) |
| `CARD_FETCH_INFLIGHT=1` + `CARD_REHASH_RUNS=1` + `JOIN_COPIES=1` | copy-back 187 → 44 ms, but GPU 134 → 422 ms: the frame doubled. Leave off on a card this size |
| `EARLY_TEXTURES_MS=40 TEXTURE_NO_WAIT=1 WIDE_TEXTURES=1` | the first minutes in the world go from ~1.2 to ~2.5 fps; `WIDE_TEXTURES` needs the fix above |
| fxceless main `f85101b` + the three texture switches, on the older stack | 3.37 → 3.93 fps (with the texture defects the fix above removes) |

## Credits

- [VIctory](https://github.com/Marvisak/VIctory) by Marvisak and its contributors: the base of everything.
- [fxceless/victory-fork](https://github.com/fxceless/victory-fork) (hyphennn) and its contributors.
- The pull request stack fxceless#770 by ExoCalibra, and the authors of the pull requests it carries: PiloteCode (#854), xiaotu030202 (#864), Chipsraine (#868), turbinedogbla-creator (#860), UwUCammie (#857), AscilCH (#764, #765), laptopnotebook (#644), KyleS98 (#715, #768, #798), pug205gti6 (#510), LeonardoS11 (#795), Peppacodes (#812), claude-GTA6 (#577), Lilith133T (#845).
- Measuring, debugging and the changes here were done with [Claude Code](https://claude.com/claude-code); fix commits list their co-authors.

The upstream README follows unchanged in [README-upstream.md](README-upstream.md).
