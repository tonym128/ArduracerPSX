# Frame-time profiling — October 2026

## Method

- Guest profiler: `game/src/prof.rs`, reading PSoXide's guest cycle counter and reporting 30-frame windows.
- Diagnostic build: `make profile-disc PROFILE_FEATURES=profile-qoi,profile-all-tracks` for NTSC and add `,profile-pal` for PAL. `profile-all-tracks` spends 90 measured frames on each of the 40 circuits; the final circuit continues until the emulator stop point.
- Emulator: PSoXide headless frontend at revision `390fd19`, launched with `--stop-at-poll 3690 --steps 5000000000 --dump-guest-profile`.
- The profile build disables whole-program LTO because the LTO build fails to link profiling code with `out of range PC16 fixup`. `make exe` still builds the shipping LTO executable. Consequently, these figures describe the diagnostic build and identify bottlenecks; they are not a final shipping-build performance claim.
- All track texture files and `LEVELS.BIN` were present on the profiling disc. The build pins QOI preference; the runtime can still use the authored fallback codec for a circuit.
- Budgets: NTSC 564,480 cycles per VBlank (60 Hz); PAL 680,438 cycles per VBlank (50 Hz). Both runs used one VBlank per selected frame. The PAL profiler budget was corrected to match its actual cadence.

Each course has at least three 30-frame windows (90 frames) in each mode. The last course has additional windows because the headless stop is poll-based. The values below are each course's largest single frame among its sampled windows, not a mean.

## Summary

| Mode | Courses | Windows | Frames over budget | Worst course/frame | Budget | Peak |
|---|---:|---:|---:|---|---:|---:|
| NTSC | 40 | 122 | 3,660 / 3,660 | Thunder Basin, 1,521,683 cycles | 564,480 | 269% |
| PAL | 40 | 122 | 3,660 / 3,660 | Onyx Run, 1,385,931 cycles | 680,438 | 203% |

The profiler counted 12,780 missed VBlank periods during the NTSC run and 9,359 during PAL. No sampled course met its selected frame budget in this diagnostic build.

## Per-course worst sampled frame

| # | Course | NTSC cycles (% budget) | PAL cycles (% budget) |
|---:|---|---:|---:|
| 0 | Cascade Falls | 1,277,754 (226%) | 1,235,659 (181%) |
| 1 | Copper Gorge | 1,300,287 (230%) | 1,308,744 (192%) |
| 2 | Longbow | 1,259,652 (223%) | 1,221,281 (179%) |
| 3 | Amber Mesa | 1,195,783 (211%) | 1,236,487 (181%) |
| 4 | Emerald Glen | 1,246,955 (220%) | 1,275,898 (187%) |
| 5 | Sunburst Valley | 1,283,768 (227%) | 1,237,091 (181%) |
| 6 | Whispering Pines | 1,330,609 (235%) | 1,233,960 (181%) |
| 7 | Ironstone Quarry | 1,336,864 (236%) | 1,282,371 (188%) |
| 8 | Silverstone Mist | 1,296,454 (229%) | 1,278,505 (187%) |
| 9 | Falcon Crest | 1,322,546 (234%) | 1,316,036 (193%) |
| 10 | Willow Creek | 1,367,097 (242%) | 1,362,469 (200%) |
| 11 | Cobalt Run | 1,317,095 (233%) | 1,263,511 (185%) |
| 12 | Dragon's Spine | 1,279,255 (226%) | 1,235,503 (181%) |
| 13 | Redwood Pass | 1,252,937 (221%) | 1,211,076 (177%) |
| 14 | Thunder Basin | 1,521,683 (269%) | 1,271,093 (186%) |
| 15 | Canyon Mirage | 1,277,026 (226%) | 1,235,619 (181%) |
| 16 | Apex Predator | 1,260,212 (223%) | 1,221,996 (179%) |
| 17 | Viper's Tail | 1,307,272 (231%) | 1,261,541 (185%) |
| 18 | Frostpeak Summit | 1,170,191 (207%) | 1,215,733 (178%) |
| 19 | Golden Horizon | 1,185,973 (210%) | 1,224,309 (179%) |
| 20 | Phantom Ravine | 1,283,979 (227%) | 1,237,591 (181%) |
| 21 | Frost Cascade | 1,508,590 (267%) | 1,340,963 (197%) |
| 22 | Lunar Gorge | 1,307,986 (231%) | 1,336,350 (196%) |
| 23 | Onyx Run | 1,334,433 (236%) | 1,385,931 (203%) |
| 24 | Titan Circuit | 1,365,979 (241%) | 1,337,381 (196%) |
| 25 | Metro 8 | 1,144,842 (202%) | 1,217,257 (178%) |
| 26 | Speedway Boulevard | 1,262,916 (223%) | 1,229,197 (180%) |
| 27 | Serpent Canyon | 1,259,716 (223%) | 1,277,942 (187%) |
| 28 | Alleyway Chicanes | 1,155,498 (204%) | 1,285,751 (188%) |
| 29 | Oil Rig Slipway | 1,179,479 (208%) | 1,215,244 (178%) |
| 30 | Apex Arena | 1,266,400 (224%) | 1,225,390 (180%) |
| 31 | Cyclone Carousel | 1,266,921 (224%) | 1,268,718 (186%) |
| 32 | Neon Grid Sprint | 1,258,855 (223%) | 1,225,505 (180%) |
| 33 | Devil's Hairpin | 1,254,268 (222%) | 1,269,757 (186%) |
| 34 | Turbo Raceway 500 | 1,393,404 (246%) | 1,230,889 (180%) |
| 35 | Industrial Slalom | 1,460,101 (258%) | 1,350,595 (198%) |
| 36 | Quad Loop Arena | 1,293,500 (229%) | 1,258,650 (184%) |
| 37 | Mirage Dunes Drift | 1,168,967 (207%) | 1,213,100 (178%) |
| 38 | Razorblade Chicanes | 1,267,699 (224%) | 1,221,341 (179%) |
| 39 | Trackmania Stadium Master | 1,437,460 (254%) | 1,329,332 (195%) |

## Where the time goes

Across the sampled windows, mean `render:track` work was about 621k cycles on NTSC and 611k on PAL. Its nested `track:quads` stage averaged 526k and 517k cycles respectively, so quad emission is the first renderer target. Those nested rows are diagnostic breakdowns and must not be added to their parent. Other major top-level costs were the HUD (about 99k cycles in both modes), AI simulation (about 96k NTSC / 71k PAL), and car rendering (about 53k / 46k). The high per-frame misses show the game needs a substantial performance pass before its 60 Hz NTSC / 50 Hz PAL target is credible.

## Raw run logs

The captured profiler output used for this report is available in the working environment at `/tmp/trackit-profile-ntsc.log` and `/tmp/trackit-profile-pal.log`; the logs are not checked into the repository because they contain every stage row for every window. Recreate them with the build and launch commands above.
