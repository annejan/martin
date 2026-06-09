//! The per-part effect vocabulary: how a part *arrives* (`Transition` + the source cloud it flies
//! in from), how it *persists* while held (`Deform`), and how it *leaves* (`Departure`). Pure data +
//! parsing — no ECS — shared by the morph timeline (`sequence`) and the composition stage (`compose`).

use bevy_gaussian_splatting::Gaussian3d;

use crate::morph::{
    ball_of, condense_of, drop_of, explode_of, fade_of, flatten_of, fold_of, funnel_of, helix_of,
    implode_of, rain_of, shatter_of, swirl_of, zoom_of,
};

pub(crate) const BALL_SHELL: f32 = 0.9; // intro ball-shell radius, in units of the framed radius

/// How a part *arrives*. `Morph` (the default after part 0) flows from the previous part's
/// shape, Morton-paired, with the optional ball-pulse `bulge`. The next group build a source
/// cloud from the part's own shape and morph in from that — the ball is just one of them. The
/// last group are *per-particle* transitions driven by the vendored shader (`transition_mode`
/// uniform): the source is an identity copy and the shader staggers opacity/position per
/// particle (see `SHADER-BLUEPRINT.md`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Transition {
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
}

/// The source cloud a STANDALONE assemble flies in from (compose objects, and seq part 0). Morph/
/// Swarm have no "previous shape" here, so they assemble from a ball; per-particle shader
/// transitions get an identity copy (the shader staggers it). `r` ≈ the content radius.
pub(crate) fn source_cloud(
    tr: Transition,
    shaped: &[Gaussian3d],
    r: f32,
) -> Option<Vec<Gaussian3d>> {
    Some(match tr {
        Transition::Ball | Transition::Morph | Transition::Swarm => ball_of(shaped, r * BALL_SHELL),
        Transition::Fade => fade_of(shaped),
        Transition::Explode => explode_of(shaped, r * 1.6),
        Transition::Implode => implode_of(shaped),
        Transition::Drop => drop_of(shaped, r * 2.5),
        Transition::Rain => rain_of(shaped, r * 3.0),
        Transition::Funnel => funnel_of(shaped, r * 3.0),
        Transition::Shatter => shatter_of(shaped, r * 1.4),
        Transition::Condense => condense_of(shaped, r * 2.2),
        Transition::Swirl => swirl_of(shaped, 2.4, 1.5),
        Transition::Extrude => flatten_of(shaped),
        Transition::Helix => helix_of(shaped, r * 3.0, 4.0),
        Transition::Fold => fold_of(shaped),
        Transition::Zoom => zoom_of(shaped, 7.0),
        _ if tr.shader_uniforms().is_some() => shaped.to_vec(),
        _ => return None,
    })
}

impl Transition {
    pub(crate) fn parse(s: &str) -> Option<Transition> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "morph" => Transition::Morph,
            "swarm" => Transition::Swarm,
            "ball" => Transition::Ball,
            "fade" => Transition::Fade,
            "explode" => Transition::Explode,
            "implode" => Transition::Implode,
            "drop" => Transition::Drop,
            "rain" => Transition::Rain,
            "funnel" | "pour" => Transition::Funnel,
            "shatter" | "shards" => Transition::Shatter,
            "condense" | "fog" | "haze" => Transition::Condense,
            "swirl" => Transition::Swirl,
            "extrude" | "rise" | "pop" => Transition::Extrude,
            "helix" | "dna" | "spiral" => Transition::Helix,
            "fold" | "unfold" => Transition::Fold,
            "zoom" | "telescope" | "warp-in" => Transition::Zoom,
            "typewriter" | "type" => Transition::Typewriter,
            "wipe" => Transition::Wipe,
            "sparkle" => Transition::Sparkle,
            "slither" => Transition::Slither,
            "vortex" => Transition::Vortex,
            "outline" => Transition::Outline,
            "pen" | "penwrite" | "pen-write" | "write" => Transition::PenWrite,
            _ => return None,
        })
    }

    /// Per-particle shader transitions use an identity source cloud (same as the target);
    /// the vendored shader staggers opacity/position. Returns the `(mode, softness, axis)`
    /// uniform triple, or `None` for the data-only / Morph transitions.
    pub(crate) fn shader_uniforms(self) -> Option<(u32, f32, u32)> {
        match self {
            Transition::Typewriter => Some((1, 0.10, 0)),
            Transition::Slither => Some((2, 0.30, 0)),
            Transition::Sparkle => Some((3, 0.40, 0)),
            Transition::Vortex => Some((5, 0.35, 1)),
            Transition::Wipe => Some((6, 0.02, 0)),
            Transition::Outline => Some((7, 0.06, 0)), // filled font → traces outlines
            Transition::PenWrite => Some((7, 0.05, 0)), // single-stroke font → handwriting
            _ => None,
        }
    }
}

/// A *persistent* vertex deform (`^name` token / `MARTIN_DEFORM`). Unlike a `Transition` (which
/// plays once on arrival), this keeps running while the part is **held** — so a `wall:` of text
/// can ripple, billow or curl the whole time it's on screen. Drives the vendored shader's deform
/// uniforms (see SHADER-BLUEPRINT.md); default-off, so an unset part renders plain.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Deform {
    Wave,       // flag-like ripple travelling across x
    Cloth,      // 2D billow (x and y out of phase)
    Ripple,     // concentric radial waves from the centre
    Twist,      // banner curl/uncurl
    Wind,       // gusting sideways sway + spatial turbulence — flutters/streams in the wind
    Turbulence, // a churning 3D field — particles swirl/boil (a turbulent force field)
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
            _ => return None,
        })
    }

    /// The `(mode, amp, freq)` uniform triple for the vendored shader deform.
    pub(crate) fn uniforms(self) -> (u32, f32, f32) {
        match self {
            Deform::Wave => (1, 0.15, 4.0),
            Deform::Cloth => (2, 0.12, 3.5),
            Deform::Ripple => (3, 0.18, 6.0),
            Deform::Twist => (4, 0.5, 2.0), // amp is radians
            Deform::Wind => (5, 0.15, 2.5),
            Deform::Turbulence => (6, 0.12, 3.0),
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
}

impl Departure {
    pub(crate) fn parse(s: &str) -> Option<Departure> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "wash" | "washaway" | "wash-away" => Departure::Wash,
            "disperse" | "dust" | "dissolve" => Departure::Disperse,
            "evaporate" | "rise" => Departure::Evaporate,
            "sink" | "fall" => Departure::Sink,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_parse_names_aliases_and_case() {
        assert_eq!(Transition::parse("fade"), Some(Transition::Fade));
        assert_eq!(
            Transition::parse("  PEN-WRITE "),
            Some(Transition::PenWrite)
        );
        assert_eq!(Transition::parse("pour"), Some(Transition::Funnel)); // alias
        assert_eq!(Transition::parse("pop"), Some(Transition::Extrude)); // alias
        assert_eq!(Transition::parse("dna"), Some(Transition::Helix)); // alias
        assert_eq!(Transition::parse("unfold"), Some(Transition::Fold)); // alias
        assert_eq!(Transition::parse("telescope"), Some(Transition::Zoom)); // alias
        assert_eq!(Transition::parse("nope"), None);
    }

    #[test]
    fn deform_and_departure_parse() {
        assert_eq!(Deform::parse("flag"), Some(Deform::Wave)); // alias
        assert_eq!(Deform::parse("churn"), Some(Deform::Turbulence));
        assert_eq!(Deform::parse("xxx"), None);
        assert_eq!(Departure::parse("dust"), Some(Departure::Disperse));
        assert_eq!(Departure::parse("fall"), Some(Departure::Sink));
        assert_eq!(Departure::parse("gone"), None);
    }

    #[test]
    fn shader_transitions_carry_uniforms_data_ones_dont() {
        assert!(Transition::Typewriter.shader_uniforms().is_some());
        assert!(Transition::PenWrite.shader_uniforms().is_some());
        assert!(Transition::Fade.shader_uniforms().is_none());
        assert!(Transition::Extrude.shader_uniforms().is_none());
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
