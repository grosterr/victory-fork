# VIctory, RTX 3060 build (work in progress)

A build of [fxceless/victory-fork](https://github.com/fxceless/victory-fork) for a **mid-range NVIDIA Ampere card (RTX 3060 12 GB)**. That fork is itself built on [VIctory](https://github.com/Marvisak/VIctory) by Marvisak and its contributors, a compatibility layer that runs an Xbox Series title natively on Windows from files the user supplies.

Here I measure what each switch and pull request does on a 12 GB, 32-lane-wave card, fix what breaks, and report findings and fixes back upstream.

**This repository contains no game files, code, assets or captures.** You need your own copy of the game. It is not affiliated with or endorsed by the game's developer or publisher. Licence: GPL-3.0-or-later, as upstream.

## State (2026-11-27)

**Series S profile: about 9 frames per second in the first rooms (107–111 ms a frame), 12 at best in the corridor. Series X profile: about 5.3 (180–190 ms).** Tested on one machine: RTX 3060 12 GB (driver 621.40), Ryzen 7 5700X, 32 GB RAM, Windows 11.

| What | State on that machine |
|------|-----------------------|
| Boot, menus, loading | work, in both profiles; in the Series S profile the interface (hints, subtitles) is drawn over the frame (fxceless#1081) |
| Prologue | cutscenes, cell block, corridor and office render lit and textured; faces, mouths, the hero's head |
| Controls | keyboard and mouse (fxceless#577), gamepad |
| Frame rate, Series S | **107–111 ms (9.0–9.3 fps)** at 5,500–6,100 draws: preparing ~55 ms, copies ~28, GPU 10–18, copy-back ~7. The submitting thread is the limit now, not the card |
| Frame rate, Series X | 180–190 ms (5.3–5.6 fps) at ~5,600 draws: preparing ~70, GPU ~80, copy-back ~56 |
| Stability | no device loss with the switches below in the runs of 2026-11-26 (up to 22 minutes each); `DISPATCH_SPREAD=0` lost the device in 2 of 4 loads |
| Shaders | 229 of 26,148 programs do not translate: 195 are one merged ES+GS geometry program (`GS_VoxelizeGeneric`, fxceless#138), 29 the PC FidelityFX library (unused), 5 the ray-traced shadow programs |

Known problems:
- No ray-traced or voxel light: rooms lit that way (the common cell) are dark, shaded places are flat. fxceless#512, #917 (`VICTORY_AMBIENT`, not in this branch yet).
- Background people's faces are sometimes black in cutscenes.
- Blocky patches where sunlight falls on walls, and stair-stepped hair edges, now and then.
- A strip of blocks from other textures at the bottom of the frame now and then: a known defect of `LEVELS_ON_DEMAND=1` (fxceless#1047).
- The game takes 22–24 GB of the PC's memory. With 32 GB of RAM set a page file of 32 GB or more.

## What this branch is

Branch `rtx3060` = the pull request stack fxceless#770 with fxceless main `52ac4ec` merged in, plus these pull requests not yet in main (all new switches off by default):
- **Descriptors and heaps:** fxceless#1293, #1297, #1320, #1324, #1371, #1372, #1373.
- **Copies and copy-back:** fxceless#1032 (`WRITE_PLAIN`), #1066 (`WIDE_MERGE`), #1188 (`BACK_QUIET`), #1274 (`SECTION_MB`), #1295 (`MERGE_MASK`), #1298 (`CARD_REHASH_RUNS`), #1313, #1316 (`CBV_SNAPSHOT_REUSE`), #1336, #1350, #1358 (`COLD_SKIP`), #868 (`SKIP_WC_BACK`, `SKIP_WC_COMPARE`, from the #770 stack).
- **Memory watch and RAM:** fxceless#1286 (`RAM_TRIM`), #1315 (`GUARD_OUTSIDE`), #1365 (`WATCH_RESERVATION`).
- **Pipelines:** fxceless#1312 (`PIPELINE_COMPUTE_WAIT`: lamps and skin right on a cold start), #1226 (`translate --cache --pipelines`).
- **Series S:** fxceless#798 (the title keeps its own upscaler), #1081 (the second present plane blended over the first).
- Kelvaroq's patches `0046` (`FRAMES_AHEAD` with `ASYNC_PRESENT`) and `0061` (`NO_DRAWS`, a diagnostic).

And our own commits:
1. **`VICTORY_SPREAD_LEAN`** ([fxceless#1352](https://github.com/fxceless/victory-fork/pull/1352)): indirect dispatches within 65,535 groups go through the plain dispatch signature; only the rare larger one takes the spread path, under predication. GPU −12.8 ms a frame here; more than half the frame under Proton on an RTX 5080 (BitWuehler).
2. **A 1 MiB floor for `SKIP_WC_BACK` and `SKIP_WC_COMPARE`** (`VICTORY_SKIP_WC_MIN_KB`, default 1024): small write-combining buffers hold counts and visibility lists the title's CPU reads. Without it, `CARD_BUFFERS_MIN_KB=64` gave flashing frames, half the draws and the hero without a head (Proudnoob4's finding in [fxceless#1163](https://github.com/fxceless/victory-fork/issues/1163); [our confirmation](https://github.com/fxceless/victory-fork/issues/1163#issuecomment-7204546502)).
3. **The frame-rate panel in the Series S profile** (it was skipped on the two-plane path).
4. **`VICTORY_SUBMIT_STAGES=1`**: a per-stage profile of the submitting thread, every few seconds (new, not yet used in a measured run).
5. **`VICTORY_RAM_SAFE=0`** (fxceless#1286 enters its emergency mode below 2 GiB free, which is always the case here) and `VICTORY_WRITE_PLAIN=alt`.
6. Test fixes where merged pull requests met each other (#1316 with #860; #1312; main `e4245f3`).
7. Earlier: the `WIDE_TEXTURES` fix with levels on demand, the GameInput key-tap port, `INDIRECT_GATHER` with #864's clamp, #854/#1112 test expectation, main merges (notes in fxceless#770).

## How it is run here

```
cargo build --release --locked -p victory
set VICTORY_HOST_WAVE=32
set VICTORY_VIEW_LIMIT=999999
target\release\victory.exe translate --warm all --jobs 16
```
(The two variables make the offline cache keys match an NVIDIA card: 32-lane waves and NVIDIA's 1,000,000-view heap.)

Switches for both profiles:
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
set VICTORY_CARD_HASH_WIDE=1
set VICTORY_LOOKUP_CACHE=1
set VICTORY_SPREAD_LEAN=1
```

Series X:
```
target\release\victory.exe run <game_scarlett_master.exe> --execute
```

Series S (faster here), add:
```
set VICTORY_SKIP_WC_BACK=1
set VICTORY_SKIP_WC_COMPARE=1
set VICTORY_CARD_BUFFERS_MIN_KB=64
set VICTORY_CARD_BUFFERS_MB=1536
target\release\victory.exe run <game_scarlett_master.exe> --execute --console series-s
```
This is the recipe of fxceless#1268 and #1379, which reach 25–32 fps on an RTX 4080 / 5080 with 720p internal rendering and ray tracing off; this branch has neither of those two yet. Do not use the four Series S switches without the 1 MiB floor above.

## What the switches did on an RTX 3060

Medians of steady frames, same save, first rooms; details of every run are kept with the measurements.

| Switch or change | Effect here |
|------------------|-------------|
| `LEVELS_ON_DEMAND=1` | lit rooms and textures; ~3 GB less video memory. Needed on 12 GB |
| `EARLY_TEXTURES_MS=40 TEXTURE_NO_WAIT=1 WIDE_TEXTURES=1` | the first minutes in the world go from ~1.2 to ~2.5 fps; `WIDE_TEXTURES` needs the fix above |
| `QUEUE2=1` | copy-back ~216 → ~65 ms |
| `ASYNC_SUBMIT=1 QUEUE2=1 INDIRECT_WINDOW=1 COPY_PLAIN=1` together | 330 → 243–282 ms (3.0 → 3.5–4.1 fps). No room without walls here (reported in [fxceless#995](https://github.com/fxceless/victory-fork/pull/995#issuecomment-7171479022)); `VICTORY_INDIRECT_WINDOW_CHECK=1` compared 810,000 windows with no difference |
| fxceless main `d510ee4` + `JOIN_PARTS=1 BACK_OWN_VIEW=1 INDIRECT_GATHER=1` | 198–230 ms |
| fxceless#1014 (`VICTORY_CARD_HASH_WIDE=1`; opt-in since main `6d20eed`) | our passes on the card 37.4 → 7.0 ms; frame 221 → 174 ms ([fxceless#1014](https://github.com/fxceless/victory-fork/pull/1014#issuecomment-7172402732)) |
| fxceless main `9c184b3` + #1057, #1112, #1044, #1101 | 179 ms at more draws than before (5,564 against 4,944): the same as without them, within noise; nothing broke |
| fxceless main `46bf535` (with `CARD_HASH_WIDE=1`) | 183 ms against 178 ms on `9c184b3` in the same session: within noise. `TARGET_VARIANTS` (on by default) made no pipeline in these runs |
| `LOOKUP_CACHE=1` (fxceless#1237) | 92 % of ~180,000 asks a frame answered from the thread's memory; the steady frame the same (177–185 ms: the first rooms are not held by the preparing threads, which run at about half of one processor), the first three minutes 232 → 181 ms with the worst frames about 3.5 times shorter (one run) |
| loose-LDS fallback of fxceless#799 (on) | posterized picture (bands on walls and skin) and +45 % GPU time; reported in [fxceless#799](https://github.com/fxceless/victory-fork/pull/799#issuecomment-7162619618) |
| `INDIRECT_KEEP=1` (fxceless#780) | device hung after 2 minutes (that pull request is closed) |
| `VICTORY_SPREAD_LEAN=1` (fxceless#1352, ours) | GPU 108 → 91 ms, frame 225 → 208 ms in a standing A/B (9 of 9 pairs faster on the GPU) |
| `DISPATCH_SPREAD=0` (main `06702fe`) | GPU −18…22 ms, but 2 of 4 loads lost the device (`DEVICE_HUNG`): keep the default, use `SPREAD_LEAN` |
| Series S profile + `SKIP_WC_BACK SKIP_WC_COMPARE CARD_BUFFERS_MIN_KB=64 CARD_BUFFERS_MB=1536` with the 1 MiB floor | 180–190 → **107–111 ms**: GPU ~80 → 10–18, copy-back ~56 → 7. Without the floor: flashing, half the draws, the hero without a head |
| `CARD_FETCH_INFLIGHT=1` + `CARD_REHASH_RUNS=1` + `JOIN_COPIES=1` | copy-back 187 → 44 ms, but GPU 134 → 422 ms: the frame doubled. Leave off on a card this size |

## Credits

- [VIctory](https://github.com/Marvisak/VIctory) by Marvisak and its contributors: the base of everything.
- [fxceless/victory-fork](https://github.com/fxceless/victory-fork) (hyphennn) and its contributors.
- The pull requests merged here on top: nullexxx (#1057), KyleS98 and DexNom (#1112), Key-rs (#1044), KainanP (#1101); and martindevbr (#1014) and KyleS98 (#1237), now in main.
- The pull request stack fxceless#770 by ExoCalibra, and the authors of the pull requests it carries: PiloteCode (#854), xiaotu030202 (#864), Chipsraine (#868), turbinedogbla-creator (#860), UwUCammie (#857), AscilCH (#764, #765), laptopnotebook (#644), KyleS98 (#715, #768, #798), pug205gti6 (#510), LeonardoS11 (#795), Peppacodes (#812), claude-GTA6 (#577), Lilith133T (#845).
- Pull requests merged since 2026-11-26: Exopus-0x65f7ad0 (#1312), CwWwWwW (#1320, #1324, #1336), Vladimir-Human (#1286, #1293), CuteBooty (#1274), UwUCammie (#1032, #1066), Filip722 (#1313), KainanP (#1295, #1298, #1315), Marukooh (#1350), TG0000 (#1358), spk-c (#1365, #1371, #1372, #1373), ByFxbian (#1316), regiumlepidi (#1226), HenriqueSBueno (#1297), KyleS98 (#1081), Yorivel (#1188); Kelvaroq (patches 0046, 0061).
- The Series S recipe and the size floor: Proudnoob4 (fxceless#1163, #1268) and Zignixx (fxceless#1379).
- Measuring, debugging and the changes here were done with [Claude Code](https://claude.com/claude-code); fix commits list their co-authors.

The upstream README follows unchanged in [README-upstream.md](README-upstream.md).
