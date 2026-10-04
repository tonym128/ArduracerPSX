# Arduracer PSX - AI Image Generation Prompts for Stage Atlas

## Overview
These prompts are designed for generating 24 unique stage illustrations for a **single mega-texture atlas** (megatexture style à la *Rage/Id Tech 5*) that will be streamed from PSX VRAM. Each stage needs to be visually distinct while maintaining a cohesive **PS1-era overhead arcade racer aesthetic**: 32-bit color, 16-bit/256-color CLUT palettes, semi-transparent effects, crisp pixel art with subtle 2.5D depth.

**Technical constraints for the atlas:**
- Target resolution per stage: **512×512** or **1024×1024** (will be downsampled to PSX VRAM pages)
- 8-bit indexed color (256 colors) with per-stage CLUT
- Top-down orthographic view with slight camera tilt for 2.5D feel
- Must tile seamlessly if adjacent in atlas
- Include track surface (tarmac, curbs, off-road), trackside detail, hazards, environmental storytelling

---

## Stage Prompts

### 1. Arduboy Oval
**Theme:** *Beginner-friendly simple oval, homage to Arduboy roots — clean, minimal, iconic*
> **Prompt:** Top-down pixel art illustration of a simple symmetrical oval racing circuit, PS1 32-bit arcade style. Pristine dark grey asphalt tarmac with crisp white racing stripes, bright red-and-white rumble curbs on inside and outside edges. Clean green grass infield with subtle pixel texture, a single grandstand on one straight with tiny pixel spectators. Minimal trackside detail: a few tire stacks, starter lights gantry, timing loop embedded in tarmac. Overhead orthographic view with slight 2.5D camera tilt showing curb height. 256-color palette: asphalt greys, curb red/white, grass greens, grandstand concrete. Clean readable shapes, no clutter — iconic "first track" energy. Crisp pixel art, 512x512.

### 2. Twin Hairpin
**Theme:** *Two dramatic 180° hairpins back-to-back, technical momentum management*
> **Prompt:** Top-down pixel art of a figure-8 style circuit dominated by two massive 180° hairpin turns connected by a short straight, PS1 arcade racer aesthetic. Dark tarmac ribbon winding through rocky desert terrain. Hairpins feature wide red-white curbs with tire marbles (dark rubber streaks) accumulated on apexes. Sand traps outside curbs with blown-sand particle detail. Center of figure-8 has a rocky mesa with sparse desert scrub. Trackside: tire barriers, marshal posts, distant canyon walls fading into haze. Palette: asphalt charcoal, curb red/white, sand beige/ochre, rock browns, sky blue-grey. Dramatic overhead view showing hairpin geometry clearly. 512x512.

### 3. The Serpent
**Theme:** *Long flowing S-curves snaking through landscape, rhythm section mastery*
> **Prompt:** Top-down pixel art of a sinuous snake-like circuit weaving through a lush temperate forest valley, PS1 32-bit style. Tarmac ribbon flows in graceful S-curves between dense pine trees. Alternating curbs on inside of each bend (red-white), grass verge with fallen pine needles texture. Forest floor: dappled light through canopy, ferns, moss on rocks. A shallow stream crosses under track via small culvert (visible as darker tarmac patch). Trackside: wooden guardrails, distance markers, occasional spectator areas carved into treeline. Palette: deep forest greens, asphalt dark grey, curb red/white, tree trunk browns, stream blue. Overhead view with slight tilt showing canopy depth. 512x512.

### 4. Canyon Chicane
**Theme:** *Red rock canyon, tight chicanes carved into cliff walls, elevation drama*
> **Prompt:** Top-down pixel art of a technical canyon circuit carved into red sandstone cliffs, PS1 arcade racer. Narrow tarmac snakes through towering canyon walls with tight left-right chicane sequences. Curbs are weathered concrete (orange-red). Canyon floor: sandy with desert brush, rockfall debris. Dramatic verticality suggested by wall-top overhangs casting pixel shadows on track. Trackside: metal guardrails bolted to rock, warning signs, emergency escape ramp filled with gravel. Distant canyon rim visible at top of frame. Palette: sandstone reds/oranges, asphalt charcoal, curb weathered orange, sand yellows, shadow purples. Strong 2.5D tilt showing wall height. 512x512.

### 5. Switchback Pass
**Theme:** *Mountain pass with tight switchbacks climbing steep grade, alpine grandeur*
> **Prompt:** Top-down pixel art of a high-alpine mountain pass with tight switchback hairpins climbing a steep slope, PS1 style. Tarmac ribbon switchbacks up mountainside with stone guardrails (grey stacked stone). Curbs: natural stone color blending with rock. Mountainside: pine trees clinging to slopes, scree fields, patches of snow at higher elevations. Hairpins have gravel traps outside. Distant snow-capped peaks in background. Trackside: avalanche barriers, kilometer markers, emergency phones. Palette: mountain greys/browns, pine dark green, snow white, asphalt dark, stone curb grey. Overhead view with strong perspective tilt showing elevation change. 512x512.

### 6. Grand Ring
**Theme:** *Classic high-speed oval/speedway, banking suggested, cathedral of speed*
> **Prompt:** Top-down pixel art of a massive superspeedway oval with subtle banking suggestion, PS1 arcade grandeur. Wide four-lane tarmac oval (dark grey with lane markers), long sweeping turns with high concrete banking (light grey). Inside infield: massive grass field with victory lane, pit lane complex along front straight (pit boxes, gantry, equipment). Grandstands wrap both straights packed with tiny pixel crowds. Giant video screen on backstretch. Trackside: SAFER barriers, catch fencing, flag stands. Palette: asphalt dark, concrete light grey, grass vibrant green, grandstand concrete, crowd colorful speckles. Wide overhead view capturing full oval majesty. 1024x1024.

### 7. Sprint Short
**Theme:** *Tiny technical sprint track, intense short laps, karting feel*
> **Prompt:** Top-down pixel art of a compact sprint circuit packed into a small footprint, PS1 style. Tight tarmac layout with quick esses, a tight hairpin, and a fast sweeper — all visible in one frame. Red-white curbs throughout. Infield: paddock area with team trucks, tire stacks, timing tower. Surrounding: flat grassland with distant treeline. Trackside: tire barriers, marshal posts, start/finish gantry with lights. Palette: asphalt, curb red/white, grass green, paddock concrete, truck colors. Tight overhead framing, every corner readable. 512x512.

### 8. Octagon Speedway
**Theme:** *Eight-sided geometric speedway, unique angular layout, high-speed flow*
> **Prompt:** Top-down pixel art of an octagonal high-speed speedway — eight distinct straight sections connected by eight identical high-radius curves, PS1 style. Symmetrical geometric perfection. Tarmac: dark with white lane lines. Curbs: bold red-white on inside of each corner. Infield: massive grass octagon with central media tower. Grandstands at each straight. Trackside: catch fencing, lighting masts at each corner, flag stations. Palette: asphalt, curb red/white, grass green, concrete, sky blue. Perfect top-down symmetry, satisfying geometry. 512x512.

### 9. Devil's Elbow
**Theme:** *Infamous tight corner complex, treacherous, demands precision*
> **Prompt:** Top-down pixel art of a notorious corner sequence — a decreasing-radius "elbow" turn followed by a blind crest and off-camber exit, set in a gloomy pine forest, PS1 atmosphere. Tarmac: worn dark asphalt with rubber streaks. Curbs: aggressive red-white serrated. Forest: dense dark pines pressing close to track, fallen logs, shadowed undergrowth. The "Elbow" corner has stacked tire barriers and a memorial plaque trackside. Mist/fog particles in low areas. Palette: dark greens, near-black asphalt, curb red/white, fog grey, memorial stone grey. Moody overhead with slight tilt showing forest canopy looming. 512x512.

### 10. Metropolis 10
**Theme:** *Urban street circuit through downtown, skyscrapers, neon, modern city*
> **Prompt:** Top-down pixel art of a city street circuit winding between skyscrapers, PS1 90s arcade racer vibe. Tarmac: city streets with manhole covers, tram lines crossing. Curbs: concrete with yellow-black hazard stripes. Buildings: pixel art skyscrapers with lit windows (yellow/blue squares), neon signs (pink/cyan/red), rooftop AC units. Street furniture: lamp posts, traffic lights, barriers, grandstands erected on sidewalks. Tunnel section under building (darker tarmac, portal visible). Palette: asphalt greys, building greys/blues, neon pink/cyan/red, curb yellow/black, window yellows. Night atmosphere with glow effects. 1024x1024.

### 11. Forest Expressway
**Theme:** *High-speed flowing circuit through ancient forest, nature cathedral*
> **Prompt:** Top-down pixel art of a wide, fast circuit flowing through an ancient old-growth forest, PS1 style. Broad tarmac ribbon (two lanes each way) sweeping through massive redwood-like trees. Curbs: natural wood-tone. Forest floor: dappled sunlight, ferns, moss, fallen giants. Trees cast long pixel shadows. Trackside: wooden catch fencing, moss-covered stone markers, spectator clearings in trees. A wooden bridge section crosses a ravine. Palette: deep forest greens, bark red-browns, asphalt dark, wood curb tones, sunbeam golds, shadow blues. Grand overhead scale showing tree majesty. 1024x1024.

### 12. Coastal Link
**Theme:** *Cliffside coastal highway, ocean views, sea spray, bridges*
> **Prompt:** Top-down pixel art of a coastal circuit clinging to cliffs above a turquoise ocean, PS1 arcade beauty. Tarmac hugs cliff edge with sheer drops. Curbs: white concrete with salt stains. Ocean: animated pixel waves (foam white), rocky shore below. Cliff face: stratified rock layers, sea birds. Trackside: concrete barriers with view cutouts, lighthouse on headland, coastal road bridge spanning a cove. Distant horizon. Palette: ocean blues/turquoise, cliff greys/tans, asphalt, curb salt-stained white, sky gradient. Breathtaking overhead with cliff verticality. 1024x1024.

### 13. Alpine Drift
**Theme:** *Alpine meadow drifting paradise, wide corners, mountain panorama*
> **Prompt:** Top-down pixel art of a drifting-focused alpine circuit on a high mountain meadow, PS1 style. Wide tarmac with generous runoff, designed for sustained drifts. Curbs: low-profile red-white, drifter-friendly. Meadow: wildflowers (yellow/purple pixels), grazing cows (tiny), streams. Mountains: jagged peaks all around, glacier visible. Trackside: hay bale barriers, cow bells on fence posts, alpine chalets. Palette: meadow greens, flower colors, asphalt, curb red/white, mountain greys/whites, chalet wood. Wide panoramic overhead, sense of altitude. 1024x1024.

### 14. Industrial Yard
**Theme:** *Gritty factory complex, shipping containers, chimneys, night shift atmosphere*
> **Prompt:** Top-down pixel art of a racing circuit weaving through a massive industrial complex at night, PS1 gritty aesthetic. Tarmac: worn industrial road with oil stains, patches. Curbs: steel plate (rust orange) and concrete. Environment: shipping container stacks (colorful rectangles), massive chimneys with pixel smoke, cooling towers, pipe networks, warehouse roofs with vents. Lighting: sodium vapor orange pools, security floodlights, welding sparks. Trackside: jersey barriers, hazard tape, chemical warning signs. Palette: industrial greys/browns, rust orange, sodium orange, container blues/reds/greens, night blues. Moody night overhead. 1024x1024.

### 15. Nightway Circuit
**Theme:** *Pure night racing, illuminated ribbon through darkness, headlight cones*
> **Prompt:** Top-down pixel art of a circuit that exists only at night — a glowing tarmac ribbon through pitch black, PS1 nocturnal magic. Tarmac: dark asphalt with subtle reflective markers (cat's eyes). Curbs: illuminated with embedded LED strips (red/white glow). Surroundings: pure darkness with only trackside lighting — grandstand floodlights, towering light masts, pit lane neon, marshals with glow sticks. Trees as silhouettes. A tunnel section fully lit. Palette: near-black, asphalt near-black, glow colors (white, amber, red, blue), silhouette darks. Dramatic chiaroscuro overhead. 1024x1024.

### 16. Harbor Slalom
**Theme:** *Port facility slalom between cranes and containers, maritime industry*
> **Prompt:** Top-down pixel art of a tight slalom circuit threaded through a busy container port, PS1 industrial charm. Tarmac: port concrete/asphalt mix. Curbs: port-yellow with black stripes. Environment: towering gantry cranes (blue/red), endless container stacks (colorful rectangles), ship hulls at berth, straddle carriers, rail lines. Water: dark harbor water with ship wake. Trackside: concrete barriers, customs booths, mooring bollards, warning lights. Palette: industrial greys, crane blues/reds, container rainbow, water dark blue, port yellow/black. Overhead showing crane scale. 1024x1024.

### 17. Mountain Gauntlet
**Theme:** *Relentless mountain challenge, elevation changes, no breathing room*
> **Prompt:** Top-down pixel art of a punishing mountain circuit — continuous corners, elevation changes, sheer drops, PS1 intensity. Tarmac: narrow ribbon clinging to mountainside. Curbs: minimal, stone. Rock faces: vertical, textured strata. Guardrails: steel cable. Hairpins stacked vertically. Distant valley floor far below. Clouds clinging to peaks. Trackside: rockfall netting, emergency refuges, kilometer markers. Palette: rock greys, asphalt dark, stone curb, valley greens, cloud whites, safety orange. Strong 2.5D tilt showing terrifying drops. 1024x1024.

### 18. Super Speedway
**Theme:** *Ultimate high-speed oval, massive scale, pure velocity temple*
> **Prompt:** Top-down pixel art of the ultimate superspeedway — massive 2.5-mile tri-oval with steep banking, PS1 scale. Enormous tarmac expanse (4+ lanes), high concrete banking (light grey) on turns. Infield: massive — multiple pit roads, garages, fan zones, lake. Grandstands: colossal, wrapping entire facility. Jumbotrons, lighting towers. Trackside: SAFER barriers, extensive catch fencing, media center. Palette: asphalt, banking concrete, grass, grandstand greys, water blue, sky. Epic overhead scale, sense of speed even static. 1024x1024.

### 19. Endurance Colosseum
**Theme:** *Massive endurance racing circuit, multiple configurations visible, history*
> **Prompt:** Top-down pixel art of a historic endurance racing cathedral — massive 8+ km circuit with multiple layout variations (GP, sprint, club) faintly visible, PS1 reverence. Main tarmac: dark aged asphalt with patches. Curbs: classic red-white, some sections vintage brick. Infield: forests, lakes, classic bridges (Dunlop-style). Grandstands: historic concrete structures with patina. Paddock: vast, historic garages. Trackside: tire walls, catch fencing evolution, marshals huts, timing tower. Palette: aged asphalt, historic curb colors, forest green, concrete patina, sky. Epic scale overhead, layers of history. 1024x1024.

### 20. Championship Final
**Theme:** *Ultimate test, all disciplines combined, grandeur, pressure*
> **Prompt:** Top-down pixel art of the definitive championship circuit — a masterpiece layout combining high-speed straights, technical esses, heavy braking zones, elevation, PS1 final boss energy. Tarmac: pristine dark grey. Curbs: aggressive red-white, varied profiles. Environment: purpose-built facility — grandstands everywhere, pit complex like a city, media center tower, champion's podium prominent. Landscaping: manicured gardens, water features, sculptures. Trackside: premium barriers, VIP suites, fan zones, fireworks prep. Palette: pristine asphalt, vibrant curbs, manicured greens, architectural whites/greys, gold accents. Majestic overhead. 1024x1024.

### 21. Neo Tokyo Expressway
**Theme:** *Futuristic elevated highway through neon megacity, cyberpunk dreams*
> **Prompt:** Top-down pixel art of an elevated expressway circuit soaring through a cyberpunk Neo Tokyo, PS1 32-bit future vision. Tarmac: elevated deck (dark grey with blue glow seams). Curbs: glowing cyan/magenta LED strips. City below: infinite pixel skyscrapers with animated neon (kanji, ads, holograms), flying car traffic trails, holographic billboards. Track: weaves between buildings, through building holes, under video walls. Lighting: intense neon reflection on wet tarmac, volumetric fog. Palette: cyberpunk neons (cyan, magenta, yellow, pink), dark blues/purples, asphalt, glow. Wet-night overhead, rain particles. 1024x1024.

### 22. Canyon Drift Apex
**Theme:** *Technical canyon drift course, narrow, technical, red rock beauty*
> **Prompt:** Top-down pixel art of a precision drift course carved into a narrow red rock canyon, PS1 drift culture. Tarmac: narrow, technical, crowned for drifting. Curbs: minimal, natural stone. Canyon walls: towering red sandstone, striated, close together. Drift zones marked with painted apex cones. Trackside: tire stacks painted with drift team logos, judging towers, smoke residue on walls. Sky: intense blue. Palette: sandstone reds/oranges, asphalt, stone curbs, tire stack colors, sky blue. Intimate overhead, wall proximity felt. 512x512.

### 23. Cyber Circuit 2097
**Theme:** *Ultra-futuristic synthetic circuit, glowing surfaces, anti-gravity vibes*
> **Prompt:** Top-down pixel art of a fully synthetic racing circuit in 2097 — glowing tarmac, force-field boundaries, zero gravity aesthetics, PS1 future fantasy. Tarmac: animated grid lines (Tron-style), color-shifting surface. Curbs: hard-light barriers (cyan/magenta). Environment: floating geometric structures, data streams, orbital ring visible above, particle accelerators. Hazards: oil slicks = data corruption zones (glitch visual), boost pads = accelerator rings. Trackside: holographic crowds, AI drones, energy barriers. Palette: synthwave neons, dark void, grid lines, hard-light colors. Abstract overhead, pure geometry. 1024x1024.

### 24. Monaco GP Classic
**Theme:** *Iconic street circuit, casino, tunnel, harbor, glamour, history*
> **Prompt:** Top-down pixel art of the legendary Monaco street circuit — Casino Square, tunnel, harbor chicane, La Rascasse, PS1 historic grandeur. Tarmac: city streets with tram lines, manhole covers. Curbs: iconic red-white (some sections Armco barriers). Buildings: Hotel de Paris, Casino, yachts in harbor (tiny white pixels), grandstands packed. Tunnel: dark portal with lighting. Marina: white yacht masts. Trackside: Armco, catch fencing, cranes, VIP terraces. Palette: Mediterranean building colors (ochre, cream, terracotta), harbor blue, asphalt, curb red/white, yacht white, sky azure. Glamorous overhead, every landmark recognizable. 1024x1024.

---

## Atlas Composition Notes

### Megatexture Layout Strategy
```
[Row 1]  Tracks 1-6   (512 each)  →  3072×512
[Row 2]  Tracks 7-12  (512 each)  →  3072×512
[Row 3]  Tracks 13-18 (mixed 512/1024) → variable
[Row 4]  Tracks 19-24 (1024 each) →  4096×1024
```
Total atlas approx **4096×3072** → fits in PSX VRAM with paging.

### Per-Stage CLUT Design
- **Tracks 1-10 (Classic):** Earth tones, natural lighting, 16-color sub-palettes per surface type
- **Tracks 11-16 (Nature/Industrial):** Greens, blues, industrial greys, night variants
- **Tracks 17-20 (Super Stages):** High-contrast, saturated, wider palettes
- **Tracks 21-24 (PSX Exclusives):** Neon/cyber palettes, glowing effects, unique CLUTs

### Surface Type Visual Language (consistent across atlas)
| Surface | Visual Key |
|---------|------------|
| Tarmac | Dark charcoal, subtle tire shine, rubber streaks on racing line |
| Curb | Red-white alternating 8px blocks, slight 3D lip on 2.5D tilt |
| Off-Road | Grass: green blades; Gravel: beige stones; Sand: dune texture; Dirt: brown ruts |
| Oil Slick | Iridescent rainbow sheen, semi-transparent additive blend |
| Boost Pad | Pulsing cyan/magenta arrows, particle fountains |
| Barrier | Tire stacks, Armco, concrete, tech barriers — distinct per venue |

### Generation Tips for AI
1. **Consistent camera:** All top-down orthographic with 15-20° forward tilt
2. **Scale reference:** 1 tile = 64 world units = ~4-8 pixels at 512px
3. **No cars:** Clean track only — cars rendered separately at runtime
4. **Seamless edges:** Fade to neutral at tile boundaries for atlas packing
5. **PS1 authencity:** Dithering on gradients, banded lighting, vertex wobble suggestion
6. **CLUT-friendly:** Flat color regions, avoid anti-aliasing between distinct surfaces

---

## Usage
Feed each prompt to your preferred AI image generator (Midjourney, Stable Diffusion, DALL-E 3, etc.) with:
- Aspect ratio: **1:1**
- Style keywords: **pixel art, 32-bit, PS1, overhead, arcade racer, 256 colors, CLUT**
- Negative: **cars, vehicles, drivers, HUD, UI, text, watermark, signature, perspective distortion, fisheye, 3D render, photorealistic**

Batch generate, then manually curate/adjust for atlas packing consistency.