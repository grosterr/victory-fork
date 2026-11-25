# VIctory, RTX 3060 build (work in progress)

A build of [fxceless/victory-fork](https://github.com/fxceless/victory-fork) for a **mid-range NVIDIA Ampere card (RTX 3060 12 GB)**. That fork is itself built on [VIctory](https://github.com/Marvisak/VIctory) by Marvisak and its contributors, a compatibility layer that runs an Xbox Series title natively on Windows from files the user supplies.

Here I measure what each switch and pull request does on a 12 GB, 32-lane-wave card, fix what breaks, and report findings and fixes back upstream.

**This repository contains no game files, code, assets or captures.** You need your own copy of the game. It is not affiliated with or endorsed by the game's developer or publisher. Licence: GPL-3.0-or-later, as upstream.

## State (2026-11-26)

**About 5.6 frames per second in the first rooms, 6.6 at best in the corridor.** Tested on one machine: RTX 3060 12 GB (driver 621.40), Ryzen 7 5700X, 32 GB RAM, Windows 11, Series X profile.

| What | State on that machine |
|------|-----------------------|
| Boot, menus, loading | work |
| Prologue | cutscene, cell block and corridor render lit and textured: people, hair, faces, posters, sunlight through windows |
| Controls | keyboard and mouse (fxceless#577), gamepad |
| Frame rate in the first rooms | **179 ms a frame (5.6 fps)** steady at ~5,500 draws; GPU ~78 ms, copy-back ~46 ms, preparing ~66 ms. Was 330 ms (3.0 fps) on 2026-11-25 |
| Stability | no device loss in about 20 runs of up to 12 minutes each |
| Shaders | 232 of 25,985 do not translate |

Known problems:
- Rare defects that come and go between launches with the same switches: a blocky outline around silhouettes in sunlight, a posterized frame, black patches on walls. Each switch alone and repeats of the same set came out clean, so they are not tied to one switch; fxceless#1156 may be the cause.
- A strip of blocks from other textures at the bottom of the frame now and then: a known defect of `LEVELS_ON_DEMAND=1` (fxceless#1047).
- A white frame for a moment between the opening cutscene and gameplay (~80 draws dropped for pipeline creation methods not implemented yet).
- No ray-traced light: shaded places are flat; a few bright rectangles of light near bunk beds.
- GPU time does not depend on resolution here (a 1080p display instead of the default 4K changed the frame by 2 %): it goes to per-draw work, the indirect argument clamps and barriers.
- With 32 GB of RAM the system's commit reaches ~55 of 56.5 GB; a page file of 32 GB or more is advised.

## What this branch is

Branch `rtx3060` = the pull request stack fxceless#770 (`d034ac1`) with fxceless main `9c184b3` merged in, plus these pull requests not yet in main:
- fxceless#1057 (nullexxx): `console_insn.rs` and `console_insn_decode.rs`, ported from a GPL-2.0-only project, are gone; VIctory's own crash handler is used. On an AMD processor nothing is lost (the instructions they emulated run natively).
- fxceless#1014 (martindevbr): the card compare hashes a piece with a group of 64 threads.
- fxceless#1112 (KyleS98, fixes from DexNom and other forks): four shader translation fixes.
- fxceless#1044 (Key-rs): DirectStorage workers sized to physical cores (8 here).
- fxceless#1101 (KainanP): the driver watch counts only the Display source's events.

And our own commits:
1. **A fix for `VICTORY_WIDE_TEXTURES=1` with levels on demand** (`704f317`, with a test): a queued texture check ran after the host step that maps its tiles, so some textures came out black or stale. Reported with the fix in [fxceless#768](https://github.com/fxceless/victory-fork/pull/768#issuecomment-7163763533).
2. **A port of Marvisak/VIctory#2230** (`5494ad0`): a key tap between two reads of the current input state is not lost. Ported onto fxceless#577's keyboard and mouse controls, with a test of how the two work together. fxceless#640 does the same and may replace it.
3. **Main's `INDIRECT_GATHER` merged with fxceless#864's clamp shader** in the stack: the gather path uses #864's `base + gi` index and records its dispatches in chunks of 64 with a UAV barrier between them ([fxceless#770](https://github.com/fxceless/victory-fork/pull/770#issuecomment-7173855026)).
4. **One test expectation updated** (`6a53386`): fxceless#854 and #1112 both fix the same voxel visibility shader; with #1112 a lane mask that comes back as two scalar copies is again the compare's mask, which a #854 test had recorded as a limit.

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
set VICTORY_ASYNC_SUBMIT=1
set VICTORY_QUEUE2=1
set VICTORY_INDIRECT_WINDOW=1
set VICTORY_COPY_PLAIN=1
set VICTORY_JOIN_PARTS=1
set VICTORY_BACK_OWN_VIEW=1
set VICTORY_INDIRECT_GATHER=1
target\release\victory.exe run <game_scarlett_master.exe> --execute
```

## What the switches did on an RTX 3060

Medians of steady frames, same save, first rooms; details of every run are kept with the measurements.

| Switch or change | Effect here |
|------------------|-------------|
| `LEVELS_ON_DEMAND=1` | lit rooms and textures; ~3 GB less video memory. Needed on 12 GB |
| `EARLY_TEXTURES_MS=40 TEXTURE_NO_WAIT=1 WIDE_TEXTURES=1` | the first minutes in the world go from ~1.2 to ~2.5 fps; `WIDE_TEXTURES` needs the fix above |
| `QUEUE2=1` | copy-back ~216 → ~65 ms |
| `ASYNC_SUBMIT=1 QUEUE2=1 INDIRECT_WINDOW=1 COPY_PLAIN=1` together | 330 → 243–282 ms (3.0 → 3.5–4.1 fps). No room without walls here (reported in [fxceless#995](https://github.com/fxceless/victory-fork/pull/995#issuecomment-7171479022)); `VICTORY_INDIRECT_WINDOW_CHECK=1` compared 810,000 windows with no difference |
| fxceless main `d510ee4` + `JOIN_PARTS=1 BACK_OWN_VIEW=1 INDIRECT_GATHER=1` | 198–230 ms |
| fxceless#1014 (on by default; `VICTORY_CARD_HASH_WIDE=0` turns it off) | our passes on the card 37.4 → 7.0 ms; frame 221 → 174 ms ([fxceless#1014](https://github.com/fxceless/victory-fork/pull/1014#issuecomment-7172402732)) |
| fxceless main `9c184b3` + #1057, #1112, #1044, #1101 | 179 ms at more draws than before (5,564 against 4,944): the same as without them, within noise; nothing broke |
| loose-LDS fallback of fxceless#799 (on) | posterized picture (bands on walls and skin) and +45 % GPU time; reported in [fxceless#799](https://github.com/fxceless/victory-fork/pull/799#issuecomment-7162619618) |
| `INDIRECT_KEEP=1` (fxceless#780) | device hung after 2 minutes (that pull request is closed) |
| `CARD_FETCH_INFLIGHT=1` + `CARD_REHASH_RUNS=1` + `JOIN_COPIES=1` | copy-back 187 → 44 ms, but GPU 134 → 422 ms: the frame doubled. Leave off on a card this size |

## Credits

- [VIctory](https://github.com/Marvisak/VIctory) by Marvisak and its contributors: the base of everything.
- [fxceless/victory-fork](https://github.com/fxceless/victory-fork) (hyphennn) and its contributors.
- The pull requests merged here on top: nullexxx (#1057), martindevbr (#1014), KyleS98 and DexNom (#1112), Key-rs (#1044), KainanP (#1101).
- The pull request stack fxceless#770 by ExoCalibra, and the authors of the pull requests it carries: PiloteCode (#854), xiaotu030202 (#864), Chipsraine (#868), turbinedogbla-creator (#860), UwUCammie (#857), AscilCH (#764, #765), laptopnotebook (#644), KyleS98 (#715, #768, #798), pug205gti6 (#510), LeonardoS11 (#795), Peppacodes (#812), claude-GTA6 (#577), Lilith133T (#845).
- Measuring, debugging and the changes here were done with [Claude Code](https://claude.com/claude-code); fix commits list their co-authors.

The upstream README follows unchanged in [README-upstream.md](README-upstream.md).
