//! The per-part effect vocabulary: how a part *arrives* (`Entrance` + the source cloud it flies
//! in from), how it *persists* while held (`Deform`), and how it *leaves* (`Departure`). Pure data +
//! parsing — no ECS — shared by the morph timeline (`sequence`) and the composition stage (`compose`).

use bevy_gaussian_splatting::Gaussian3d;

use crate::morph::{
    ball_of, condense_of, disperse_of, drop_of, evaporate_of, explode_of, fade_of, flatten_of,
    fold_of, funnel_of, helix_of, implode_of, rain_of, shatter_of, sink_of, swirl_of, wash_of,
    zoom_of,
};

pub(crate) const BALL_SHELL: f32 = 0.9; // intro ball-shell radius, in units of the framed radius

/// How a part *arrives*. `Morph` (the default after part 0) flows from the previous part's
/// shape, Morton-paired, with the optional ball-pulse `bulge`. The next group build a source
/// cloud from the part's own shape and morph in from that — the ball is just one of them. The
/// last group are *per-particle* transitions driven by the fork shader (`transition_mode`
/// uniform): the source is an identity copy and the shader staggers opacity/position per
/// particle (see `SHADER-BLUEPRINT.md`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Entrance {
    Morph,    // prev shape → this shape (with bulge ball-pulse); the original behaviour
    Swarm,    // like Morph but particles flock/swarm along curled paths between the two scenes
    Ball,     // assemble out of a fuzzy ball shell (default for part 0)
    Fade,     // fade up on the spot (opacity 0 → in)
    Explode,  // gather in from an outward burst
    Implode,  // expand out from a dense point
    Drop,     // fall straight down into place
    Rain,     // fall in from scattered high points (a shower), staggered
    Funnel,   // pour in from a tall narrow column above, fanning out + down
    Shatter,  // re-assemble from ~8 tumbling shards
    Condense, // condense out of a wide faded haze
    Swirl,    // sweep/spiral in around the vertical axis
    Extrude,  // rise out of a flat silhouette into 3D (a logo extruding from its svg into its mesh)
    Helix,    // reel in off a tall spinning column (a DNA/barber-pole assemble)
    Fold,     // unfold sideways out of a vertical seam (like opening a folded sheet)
    Zoom,     // rush in from far — a telescope / hyperspace zoom into place
    // --- per-particle (shader transition_mode) ---
    Typewriter, // reveal left→right as a moving edge (great for text)
    Wipe,       // hard slab reveal across the x axis
    Sparkle,    // random per-particle twinkle-in (HDR bloom flashes)
    Slither,    // staggered lateral sine that settles
    Vortex,     // continuous unwind-rotation about the vertical axis
    Outline, // text traced in outline/pen order — a glowing neon draw-on (filled font); text only
    PenWrite, // text written in pen order on a single-stroke font — true handwriting; text only
    Shockwave, // materialise as an expanding ring sweeping outward from the centre (a kick "blast")
}

/// The source cloud a STANDALONE assemble flies in from (compose objects, and seq part 0). Morph/
/// Swarm have no "previous shape" here, so they assemble from a ball; per-particle shader
/// transitions get an identity copy (the shader staggers it). `r` ≈ the content radius.
pub(crate) fn source_cloud(tr: Entrance, shaped: &[Gaussian3d], r: f32) -> Option<Vec<Gaussian3d>> {
    Some(match tr {
        Entrance::Ball | Entrance::Morph | Entrance::Swarm => ball_of(shaped, r * BALL_SHELL),
        Entrance::Fade => fade_of(shaped),
        Entrance::Explode => explode_of(shaped, r * 1.6),
        Entrance::Implode => implode_of(shaped),
        Entrance::Drop => drop_of(shaped, r * 2.5),
        Entrance::Rain => rain_of(shaped, r * 3.0),
        Entrance::Funnel => funnel_of(shaped, r * 3.0),
        Entrance::Shatter => shatter_of(shaped, r * 1.4),
        Entrance::Condense => condense_of(shaped, r * 2.2),
        Entrance::Swirl => swirl_of(shaped, 2.4, 1.5),
        Entrance::Extrude => flatten_of(shaped),
        Entrance::Helix => helix_of(shaped, r * 3.0, 4.0),
        Entrance::Fold => fold_of(shaped),
        Entrance::Zoom => zoom_of(shaped, 7.0),
        _ if tr.shader_uniforms().is_some() => shaped.to_vec(),
        _ => return None,
    })
}

impl Entrance {
    pub(crate) fn parse(s: &str) -> Option<Entrance> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "morph" => Entrance::Morph,
            "swarm" => Entrance::Swarm,
            "ball" => Entrance::Ball,
            "fade" => Entrance::Fade,
            "explode" => Entrance::Explode,
            "implode" => Entrance::Implode,
            "drop" => Entrance::Drop,
            "rain" => Entrance::Rain,
            "funnel" | "pour" => Entrance::Funnel,
            "shatter" | "shards" => Entrance::Shatter,
            "condense" | "fog" | "haze" => Entrance::Condense,
            "swirl" => Entrance::Swirl,
            "extrude" | "rise" | "pop" => Entrance::Extrude,
            "helix" | "dna" | "spiral" => Entrance::Helix,
            "fold" | "unfold" => Entrance::Fold,
            "zoom" | "telescope" | "warp-in" => Entrance::Zoom,
            "typewriter" | "type" => Entrance::Typewriter,
            "wipe" => Entrance::Wipe,
            "sparkle" => Entrance::Sparkle,
            "slither" => Entrance::Slither,
            "vortex" => Entrance::Vortex,
            "outline" => Entrance::Outline,
            "pen" | "penwrite" | "pen-write" | "write" => Entrance::PenWrite,
            "shockwave" | "blast" | "shock" => Entrance::Shockwave,
            _ => return None,
        })
    }

    /// Per-particle shader transitions use an identity source cloud (same as the target);
    /// the fork shader staggers opacity/position. Returns the `(mode, softness, axis)`
    /// uniform triple, or `None` for the data-only / Morph transitions.
    pub(crate) fn shader_uniforms(self) -> Option<(u32, f32, u32)> {
        match self {
            Entrance::Typewriter => Some((1, 0.10, 0)),
            Entrance::Slither => Some((2, 0.30, 0)),
            Entrance::Sparkle => Some((3, 0.40, 0)),
            Entrance::Vortex => Some((5, 0.35, 1)),
            Entrance::Wipe => Some((6, 0.02, 0)),
            Entrance::Outline => Some((7, 0.06, 0)), // filled font → traces outlines
            Entrance::PenWrite => Some((7, 0.05, 0)), // single-stroke font → handwriting
            Entrance::Shockwave => Some((8, 0.18, 0)), // radial blast-front reveal from the centre
            _ => None,
        }
    }
}

/// How a shot's morph factor is **shaped** over time (`ease:name`). The factor runs 0→1 across the
/// `morph` window; this bends it before it becomes `CloudSettings.time`, so the same entrance can
/// drift in gently (`smoothstep`) or LAND on the beat (`snap`/`hold-snap`). Pure scalar — no GPU, no
/// clock, fully deterministic. `Smoothstep` is the default and reproduces the original behaviour
/// bit-for-bit. The single source of the morph curve: the reel (`shot_director`) and the stage
/// (`compose`) both route their factor through `Ease::apply`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Ease {
    #[default]
    Smoothstep, // f²(3-2f) — the classic ease-in-out (default, unchanged)
    Snap,       // cubic ease-in (f³): hangs low, then SLAMS up — lands hard on the beat
    Anticipate, // back-ease-in: winds back (clamped at the source) then whips in
    Stutter,    // stepped: the morph clicks forward in discrete chunks (mechanical, stop-motion)
    HoldSnap,   // holds at the source, then snaps in over the last 20% — the punchiest landing
}

impl Ease {
    pub(crate) fn parse(s: &str) -> Option<Ease> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "smooth" | "smoothstep" | "ease" => Ease::Smoothstep,
            "snap" | "slam" => Ease::Snap,
            "anticipate" | "back" | "wind" => Ease::Anticipate,
            "stutter" | "step" | "click" => Ease::Stutter,
            "hold-snap" | "holdsnap" | "hold" => Ease::HoldSnap,
            _ => return None,
        })
    }

    /// Shape a morph factor `f ∈ [0,1]` into the eased blend value. Output is clamped to `[0,1]` so
    /// `CloudSettings.time` always stays a valid blend (the `Anticipate` backwind would otherwise dip
    /// below 0). `Smoothstep` is identical to the long-standing `f*f*(3-2*f)`.
    pub(crate) fn apply(self, f: f32) -> f32 {
        let f = f.clamp(0.0, 1.0);
        match self {
            Ease::Smoothstep => f * f * (3.0 - 2.0 * f),
            Ease::Snap => f * f * f,
            Ease::Anticipate => {
                const S: f32 = 1.70158; // standard back-ease overshoot constant
                (f * f * ((S + 1.0) * f - S)).clamp(0.0, 1.0)
            }
            Ease::Stutter => {
                const STEPS: f32 = 6.0; // 6 chunks across the morph; reaches exactly 1.0 at f=1
                (f * STEPS).floor() / STEPS
            }
            Ease::HoldSnap => {
                let g = ((f - 0.8) / 0.2).clamp(0.0, 1.0); // 0 until 80%, then 0→1
                g * g * (3.0 - 2.0 * g)
            }
        }
    }
}

/// A *persistent* vertex deform (`^name` token / `MARTIN_DEFORM`). Unlike a `Entrance` (which
/// plays once on arrival), this keeps running while the part is **held** — so a `wall:` of text
/// can ripple, billow or curl the whole time it's on screen. Drives the fork shader's deform
/// uniforms (see SHADER-BLUEPRINT.md); default-off, so an unset part renders plain.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Deform {
    Wave,       // flag-like ripple travelling across x
    Cloth,      // 2D billow (x and y out of phase)
    Ripple,     // concentric radial waves from the centre
    Twist,      // banner curl/uncurl
    Wind,       // gusting sideways sway + spatial turbulence — flutters/streams in the wind
    Turbulence, // a churning 3D field — particles swirl/boil (a turbulent force field)
    Pulse,      // the whole shape breathes in/out about its centre
    Jitter,     // a fast per-particle shake — nervous, glitchy energy
    Spiral,     // a radial pinwheel — swirls/curls about the vertical axis
}

impl Deform {
    pub(crate) fn parse(s: &str) -> Option<Deform> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "wave" | "flag" => Deform::Wave,
            "cloth" | "billow" => Deform::Cloth,
            "ripple" => Deform::Ripple,
            "twist" | "curl" => Deform::Twist,
            "wind" | "gust" => Deform::Wind,
            "turbulence" | "turb" | "churn" => Deform::Turbulence,
            "pulse" | "breathe" => Deform::Pulse,
            "jitter" | "shake" => Deform::Jitter,
            "spiral" | "pinwheel" => Deform::Spiral,
            _ => return None,
        })
    }

    /// The `(mode, amp, freq)` uniform triple for the fork shader deform.
    pub(crate) fn uniforms(self) -> (u32, f32, f32) {
        match self {
            Deform::Wave => (1, 0.15, 4.0),
            Deform::Cloth => (2, 0.12, 3.5),
            Deform::Ripple => (3, 0.18, 6.0),
            Deform::Twist => (4, 0.5, 2.0), // amp is radians
            Deform::Wind => (5, 0.15, 2.5),
            Deform::Turbulence => (6, 0.12, 3.0),
            Deform::Pulse => (7, 0.10, 1.0), // amp = breathe fraction (±10%)
            Deform::Jitter => (8, 0.04, 1.0), // small per-particle shake
            Deform::Spiral => (9, 0.8, 3.0), // amp ≈ radians, freq = radial
        }
    }
}

/// How a part *leaves* (`out:name`). Where a `~transition` says how a part ARRIVES, this says how it
/// DEPARTS: it morphs to a faded "gone" cloud as a distinct step at the end of its hold (before the
/// next part arrives), so the object dissolves away instead of cross-morphing straight to the next.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Departure {
    Wash,      // flows off sideways and fades — washed away
    Disperse,  // scatters outward in all directions and fades — blown to dust
    Evaporate, // drifts upward and fades — rises away
    Sink,      // falls straight down and fades — drops out the bottom
    Explode, // flung ballistically outward from the centre and fades — a burst (vs Disperse's softer wash)
}

/// The three effect modifiers shared by the reel (`parse_seq`) and the stage (`parse_compose`):
/// `~transition`, `^deform[:amp]`, `tint:mode`. One parser so a new tint mode / `^` syntax change is
/// a one-place edit instead of two.
#[derive(Debug, PartialEq)]
pub(crate) enum FxMod {
    Entrance(Entrance),
    Deform(Deform, Option<f32>), // (deform, optional `:amp` strength)
    Tint(crate::scene::colorize::Tint),
    Ease(Ease), // `ease:name` — shapes the morph curve (snap/anticipate/…)
}

/// Parse one whitespace token as a shared fx modifier. `None` = not one of these sigils (keep the
/// token); `Some(Ok(m))` = parsed; `Some(Err(what))` = a recognised sigil with a bad value — the
/// caller warns (with its own `seq:`/`compose:` prefix) and consumes it.
pub(crate) fn parse_fx_modifier(tok: &str) -> Option<Result<FxMod, String>> {
    if let Some(t) = tok.strip_prefix('~') {
        return Some(
            Entrance::parse(t)
                .map(FxMod::Entrance)
                .ok_or_else(|| format!("unknown transition '~{t}'")),
        );
    }
    if let Some(d) = tok.strip_prefix('^') {
        // `^name` or `^name:amp` — the optional amp scales the deform strength (bad amp → 1.0).
        let (name, amp) = d.split_once(':').map_or((d, None), |(n, a)| (n, Some(a)));
        return Some(match Deform::parse(name) {
            Some(de) => Ok(FxMod::Deform(de, amp.and_then(|a| a.parse().ok()))),
            None => Err(format!("unknown deform '^{d}'")),
        });
    }
    if let Some(tn) = tok.strip_prefix("tint:") {
        return Some(
            crate::scene::colorize::Tint::parse(tn)
                .map(FxMod::Tint)
                .ok_or_else(|| format!("unknown tint 'tint:{tn}'")),
        );
    }
    if let Some(en) = tok.strip_prefix("ease:") {
        return Some(
            Ease::parse(en)
                .map(FxMod::Ease)
                .ok_or_else(|| format!("unknown ease 'ease:{en}'")),
        );
    }
    None
}

impl Departure {
    pub(crate) fn parse(s: &str) -> Option<Departure> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "wash" | "washaway" | "wash-away" => Departure::Wash,
            "disperse" | "dust" | "dissolve" => Departure::Disperse,
            "evaporate" | "rise" => Departure::Evaporate,
            "sink" | "fall" => Departure::Sink,
            "explode" | "burst" => Departure::Explode,
            _ => return None,
        })
    }

    /// The faded, displaced cloud a part morphs *to* as it leaves — the exit's sibling of
    /// `source_cloud` (the arrival). `r` is the part's framed radius.
    pub(crate) fn out_cloud(self, shaped: &[Gaussian3d], r: f32) -> Vec<Gaussian3d> {
        match self {
            Departure::Wash => wash_of(shaped, r * 2.5),
            Departure::Disperse => disperse_of(shaped, r * 1.8),
            Departure::Evaporate => evaporate_of(shaped, r * 3.0),
            Departure::Sink => sink_of(shaped, r * 3.0),
            Departure::Explode => explode_of(shaped, r * 2.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::colorize::Tint;

    #[test]
    fn parse_fx_modifier_covers_all_three_sigils() {
        // recognized + parsed
        assert_eq!(
            parse_fx_modifier("~morph"),
            Some(Ok(FxMod::Entrance(Entrance::Morph)))
        );
        assert_eq!(
            parse_fx_modifier("^wave"),
            Some(Ok(FxMod::Deform(Deform::Wave, None)))
        );
        assert_eq!(
            parse_fx_modifier("^wave:0.5"),
            Some(Ok(FxMod::Deform(Deform::Wave, Some(0.5))))
        );
        assert_eq!(
            parse_fx_modifier("^wave:nope"), // bad amp → None (deform still applies)
            Some(Ok(FxMod::Deform(Deform::Wave, None)))
        );
        assert_eq!(
            parse_fx_modifier("tint:fry"),
            Some(Ok(FxMod::Tint(Tint::Fry)))
        );
        assert_eq!(
            parse_fx_modifier("ease:snap"),
            Some(Ok(FxMod::Ease(Ease::Snap)))
        );
        // recognized sigil, bad value → Err (caller warns + consumes)
        assert!(matches!(parse_fx_modifier("~bogus"), Some(Err(_))));
        assert!(matches!(parse_fx_modifier("^bogus"), Some(Err(_))));
        assert!(matches!(parse_fx_modifier("tint:bogus"), Some(Err(_))));
        assert!(matches!(parse_fx_modifier("ease:bogus"), Some(Err(_))));
        // not an fx token → None (the caller keeps it for the head/placement)
        assert_eq!(parse_fx_modifier("splat:x.ply"), None);
        assert_eq!(parse_fx_modifier("@5,3,0"), None);
        assert_eq!(parse_fx_modifier("flock:5"), None);
    }

    #[test]
    fn transition_parse_names_aliases_and_case() {
        assert_eq!(Entrance::parse("fade"), Some(Entrance::Fade));
        assert_eq!(Entrance::parse("  PEN-WRITE "), Some(Entrance::PenWrite));
        assert_eq!(Entrance::parse("pour"), Some(Entrance::Funnel)); // alias
        assert_eq!(Entrance::parse("pop"), Some(Entrance::Extrude)); // alias
        assert_eq!(Entrance::parse("dna"), Some(Entrance::Helix)); // alias
        assert_eq!(Entrance::parse("unfold"), Some(Entrance::Fold)); // alias
        assert_eq!(Entrance::parse("telescope"), Some(Entrance::Zoom)); // alias
        assert_eq!(Entrance::parse("blast"), Some(Entrance::Shockwave)); // alias
        assert_eq!(Entrance::parse("nope"), None);
    }

    #[test]
    fn ease_curves_endpoints_and_shape() {
        // every curve pins the endpoints (a morph always completes), default = old smoothstep.
        for e in [
            Ease::Smoothstep,
            Ease::Snap,
            Ease::Anticipate,
            Ease::Stutter,
            Ease::HoldSnap,
        ] {
            assert!(e.apply(0.0).abs() < 1e-6, "{e:?} f=0");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-6, "{e:?} f=1");
            // output always a valid blend factor (Anticipate's backwind is clamped).
            for i in 0..=10 {
                let v = e.apply(i as f32 / 10.0);
                assert!((0.0..=1.0).contains(&v), "{e:?} out of range: {v}");
            }
        }
        assert_eq!(Ease::default(), Ease::Smoothstep);
        assert_eq!(Ease::Smoothstep.apply(0.5), 0.5); // smoothstep(0.5)=0.5
        assert!(Ease::Snap.apply(0.5) < 0.5); // ease-in hangs low at the midpoint
        assert_eq!(Ease::HoldSnap.apply(0.5), 0.0); // still at the source before 80%
        assert_eq!(Ease::parse("slam"), Some(Ease::Snap)); // alias
        assert_eq!(Ease::parse("hold-snap"), Some(Ease::HoldSnap));
        assert_eq!(Ease::parse("nope"), None);
    }

    #[test]
    fn deform_and_departure_parse() {
        assert_eq!(Deform::parse("flag"), Some(Deform::Wave)); // alias
        assert_eq!(Deform::parse("churn"), Some(Deform::Turbulence));
        assert_eq!(Deform::parse("breathe"), Some(Deform::Pulse)); // alias
        assert_eq!(Deform::parse("pinwheel"), Some(Deform::Spiral)); // alias
        assert_eq!(Deform::parse("xxx"), None);
        assert_eq!(Departure::parse("dust"), Some(Departure::Disperse));
        assert_eq!(Departure::parse("fall"), Some(Departure::Sink));
        assert_eq!(Departure::parse("burst"), Some(Departure::Explode)); // alias
        assert_eq!(Departure::parse("EXPLODE"), Some(Departure::Explode));
        assert_eq!(Departure::parse("gone"), None);
    }

    #[test]
    fn shader_transitions_carry_uniforms_data_ones_dont() {
        assert!(Entrance::Typewriter.shader_uniforms().is_some());
        assert!(Entrance::PenWrite.shader_uniforms().is_some());
        assert_eq!(Entrance::Shockwave.shader_uniforms(), Some((8, 0.18, 0))); // radial blast, mode 8
        assert!(Entrance::Fade.shader_uniforms().is_none());
        assert!(Entrance::Extrude.shader_uniforms().is_none());
    }

    #[test]
    fn every_deform_has_distinct_nonzero_mode() {
        let modes: Vec<u32> = [
            Deform::Wave,
            Deform::Cloth,
            Deform::Ripple,
            Deform::Twist,
            Deform::Wind,
            Deform::Turbulence,
            Deform::Pulse,
            Deform::Jitter,
            Deform::Spiral,
        ]
        .iter()
        .map(|d| d.uniforms().0)
        .collect();
        assert!(modes.iter().all(|&m| m != 0)); // 0 = "off" in the shader
        let mut sorted = modes.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), modes.len()); // all distinct
    }
}
