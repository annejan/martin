# martin — art direction & capture recipe 🎥🐕🌿

Everything `martin` shows on screen is a **3D Gaussian splat**. The demo morphs them, frames
them, and makes them glow — but it can only ever be as good as the splats you feed it. This
is the art-direction doc: how to **shoot and prep good source data**, and how to keep a set
looking like it belongs together.

> Splatting is *photography of a volume*. Treat it like a shoot: light, coverage, and a still
> subject matter far more than any slider. Get the capture right and the rest is easy.

---

## 1. Two flavours of splat (pick a vibe, commit to it)

There are two ways to get a `.ply`, and they *look* different. Mixing them in one morph reads
as a mistake, so choose per scene and keep a morph set consistent.

- **TRELLIS / single-image** (browser, one photo → splat): many **small, opaque** splats →
  a crisp, slightly *hard* surface with a charming **PS1 texture-warp** wobble when morphed.
  No data for the unseen sides, though → a **hollow back**. (This is why the demo's camera
  only *sways across the front* instead of orbiting 360° — see `MARTIN_YAW`.)
- **Brush / photo-capture** (local, `pipeline/splat.sh`): **fewer, bigger, semi-transparent**
  splats that blend → a softer, photographic, *volumetric* look that **dissolves** rather than
  warps. A full multi-angle capture is **full 360°**, so you can orbit all the way around.

Brush lets you tune **densification** to hit a gaussian budget — ~250k–500k stays a smooth
60 fps on the iGPU; the full ~1.15M is crisp but ~20 fps (best for offline video).

---

## 2. Shoot the photos / video

**The golden rule: *you* move around the subject; the subject stays still.** You're giving the
computer many overlapping views of one thing so it can solve the 3D shape.

- **Coverage.** Walk a full circle around the subject — ideally *two* circles, one low and one
  higher looking slightly down. For an object, a turntable/lazy-Susan and a fixed camera works
  too (spin *it*, not you). Aim for **40–150 photos** or a slow **30–60 s video** (the pipeline
  pulls frames from video).
- **Overlap.** Each shot overlaps the last by ~70% — small steps, not big jumps. *Shuffle
  sideways*, don't teleport.
- **Light.** Flat, even light wins — an **overcast day outdoors is perfect**. Avoid hard
  shadows, harsh sun, and dappled light through moving leaves.
- **Lock the camera** — fixed exposure, focus, and white balance (see the box below). This is
  the single most-overlooked art-direction step, and it matters a lot.
- **Sharpness.** Blur kills reconstruction. Good light → fast shutter → sharp frames.
- **What breaks it:** anything that *moves or changes* between shots — wind in leaves, a
  wagging tail, ripples, passers-by, your own shadow. Also **shiny / reflective / transparent**
  surfaces (glass, water, chrome, wet noses) and big blank areas (clear sky, a plain wall) —
  the solver needs *texture* to lock onto. A cluttered, textured background actually helps.

- 🐕 **Dogs / animals** move, so they're hard. Best options, in order: a **sleeping / very
  calm** animal; **burst mode**, full circle, fast; or a **toy / figurine / statue** of the
  breed (trivial — put it on a turntable). Get down to eye level, grab a few from above/below.
- 🌿 **Nature / scenes** (a tree, a rock, a garden corner): pick a **calm, overcast, windless**
  moment, lock exposure, arc around it. Don't point at bright sky.

### 🔒 Lock the camera — fixed exposure, focus & white balance

3DGS **bakes whatever it sees into each splat's colour**. If the phone silently re-meters
between frames — auto-exposure brightening as you pan toward shadow, white balance drifting
warmer indoors, autofocus "breathing" — then the *same surface* is a different colour and size
in every shot. COLMAP struggles to match features, and the trained splat comes out muddy,
flickery, and low-contrast. **Lock everything before the first frame:**

- **Exposure.** Use **manual / "Pro" mode** and fix **ISO + shutter** (and aperture if you
  have it). No Pro mode? **Tap-and-hold to lock AE/AF** (the little lock badge) and don't let
  go. Meter on a *mid-tone* part of the subject — never the bright sky.
- **White balance.** Lock it (a fixed Kelvin in Pro mode, or AWB-lock). Drifting WB tints the
  whole model as you walk around it.
- **Focus.** Lock focus on the subject so it doesn't hunt or "breathe". For small objects, stop
  down a little (or step back + zoom) for depth of field so the whole thing stays sharp.
- **Kill the "smart" features.** Auto-HDR, night mode, "scene optimisation" / AI enhance, and
  in-camera sharpening all change appearance per frame — turn them **off**. **Shoot RAW** if
  you can (or top-quality JPEG) and grade *once*, identically, across the whole set.
- **Video.** Lock a **fixed frame rate + shutter**; avoid auto-exposure ramps. Pan slowly and
  smoothly so frames aren't motion-blurred.

> Rule of thumb: if you cover the lens and the preview brightness *changes*, the camera is
> still metering — go find the lock. Consistent light in = clean, punchy splats out.

---

## 3. Turn the photos into a splat (all CUDA-free, on this machine)

```bash
./pipeline/splat-setup.sh                 # once: builds COLMAP (CPU) + Brush (Vulkan)
./pipeline/splat.sh my_dog_video.mp4      # or:  ./pipeline/splat.sh ./my_photos/
VIEWER=1 ./pipeline/splat.sh ./my_photos/ # watch it train live in Brush's window
```

Out comes a `.ply`. Tunables (env): `FPS`, `MAX_SIZE`, `EXPORT_EVERY`, `VIEWER`. The pipeline
is **video | image-dir → ffmpeg frames → COLMAP CPU SfM + undistort → Brush training → .ply**.

No good photo set yet, or only **one** image? Drop it into
**[TRELLIS](https://huggingface.co/spaces/trellis-community/TRELLIS)** in your browser for a
quick single-image splat (mind the hollow-back caveat above).

---

## 4. Tidy it up (browser, free)

Open the `.ply` at **<https://superspl.at/editor>**:

- **Box-select and delete stray "floater" splats** and the background.
- **Recenter** the subject at the origin and **scale** it to a sane size.
- **Export as uncompressed / standard PLY** — the demo's loader rejects SuperSplat's
  *compressed* format (`missing required properties`).

> martin already **auto-crops floaters**: `MARTIN_NORMALIZE` centres each part on its centroid
> and scales by the 90th-percentile radius, so a handful of stray splats won't shrink your
> scene to a distant dot. Cleaning in SuperSplat still helps the *morph* (paired by spatial
> sort) and keeps the gaussian budget honest.

---

## 5. Keep a morph set consistent

For a clean **morph** (sources → target), prep every splat the *same way*:

- same **up-axis** (and orient it with `MARTIN_ROT` if a scene comes in sideways),
- **centred**, similar overall size (the demo normalizes, but start close),
- a **consistent gaussian character** — don't mix a hard TRELLIS object into a soft Brush
  scene mid-sequence.

Mismatched assets blend muddily. Pick one vibe per scene, shoot it well, clean it, and let the
engine fly. 🪩

*— code: annejan · greetings to everyone still rendering on the metal*
