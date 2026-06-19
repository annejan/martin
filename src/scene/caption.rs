//! Screen-anchored captions: a title/credit pinned to SCREEN space (a fixed screen fraction) that
//! stays put while the splat camera flies — unlike `text:` which renders as world-space gaussians the
//! camera moves past. Authored in a `.show` `[caption]` track:
//!
//!     [caption]
//!     screentext:deFEEST       in @@intro  out @@drop      at 0.5,0.08  size 72
//!     screentext:VJ annejan    in @@outro  out @@outro+4bar at 0.04,0.9  size 40
//!
//! Built on bevy_ui `Text` (a real UI node — no reinvention). The load-bearing detail: the caption
//! root carries `UiTargetCamera(orbit_cam)` so it composites into whatever the splat camera renders —
//! the offscreen image in a HEADLESS record, the window live — so captions DO bake into recordings
//! (the loader screen, lacking this, does not). Alpha is a pure function of `SeqClock.t` → deterministic.

use bevy::prelude::*;
use bevy::text::Font;
use bevy::ui::UiTargetCamera;

use crate::camera::OrbitCam;
use crate::scene::SeqClock;
use crate::score::Score;

/// Bundled font (same as text.rs — include_bytes, not the asset server, whose root is the .ply folder).
static FONT: &[u8] = include_bytes!("../../assets/font.ttf");

/// One parsed caption: what to show, when (show-clock seconds), where (screen fraction), how big.
#[derive(Clone)]
pub struct CaptionSpec {
    text: String,
    appear: f32, // fade-in start (s); 0 = from the top
    out: f32,    // fade-out start (s); f32::MAX = stays to the end
    fade: f32,   // fade in/out duration (s)
    fx: f32,     // screen x fraction (0..1) of the text's top-left
    fy: f32,     // screen y fraction (0..1); can be >1 or <0 with scroll
    size: f32,   // font size (px)
    scroll: f32, // upward scroll speed (fraction of screen / s); 0 = static
}

/// The parsed `[caption]` track (inserted from main once the score exists).
#[derive(Resource, Default)]
pub struct Captions(pub Vec<CaptionSpec>);

/// Timing carried on each spawned caption text entity, so `animate_captions` ramps its alpha.
#[derive(Component)]
struct CaptionTag {
    appear: f32,
    out: f32,
    fade: f32,
}

/// Carried on the parent Node when a caption scrolls — drives its Y position from the show clock.
#[derive(Component)]
struct CaptionScroll {
    fy_pct: f32, // base screen-y as percentage (0–100)
    speed: f32,  // percentage-points per second, positive = upward
    appear: f32,
}

/// Parse the `[caption]` lines (raw, comment-stripped) into specs. Forgiving: warn + skip a bad line.
/// Grammar: `screentext:<text…>  [in <anchor>] [out <anchor>] [at fx,fy] [size px] [fade s] [scroll s]`.
pub fn parse_captions(lines: &[String], score: &Score) -> Vec<CaptionSpec> {
    let is_kw = |t: &str| matches!(t, "in" | "out" | "at" | "size" | "fade" | "scroll");
    let mut out = Vec::new();
    for raw in lines {
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let toks: Vec<&str> = line.split_whitespace().collect();
        let Some(first) = toks.first().and_then(|h| {
            h.strip_prefix("screentext:")
                .or_else(|| h.strip_prefix("title:"))
        }) else {
            warn!("caption: line must start with screentext:/title: — skipped: {line}");
            continue;
        };
        // text = the head's first word + any following non-keyword tokens (so multi-word captions work).
        let mut text = first.to_string();
        let mut i = 1;
        while i < toks.len() && !is_kw(toks[i]) {
            text.push(' ');
            text.push_str(toks[i]);
            i += 1;
        }
        let (mut appear, mut out_t, mut fade, mut fx, mut fy, mut size, mut scroll) =
            (0.0_f32, f32::MAX, 0.6_f32, 0.5_f32, 0.1_f32, 56.0_f32, 0.0_f32);
        while i < toks.len() {
            let val = toks.get(i + 1).copied().unwrap_or("");
            match toks[i] {
                "in" => appear = score.anchor_seconds(val).unwrap_or(0.0),
                "out" => out_t = score.anchor_seconds(val).unwrap_or(f32::MAX),
                "at" => {
                    let n: Vec<f32> = val
                        .split(',')
                        .filter_map(|x| x.trim().parse().ok())
                        .collect();
                    if let Some(a) = n.first() {
                        fx = a.clamp(0.0, 1.0);
                    }
                    if let Some(b) = n.get(1) {
                        fy = *b; // allow >1 (below-screen) for credit scroll
                    }
                }
                "size" => size = val.parse().unwrap_or(56.0),
                "fade" => fade = val.parse().unwrap_or(0.6),
                "scroll" => scroll = val.parse().unwrap_or(0.0),
                _ => {}
            }
            i += 2;
        }
        if text.is_empty() {
            continue;
        }
        out.push(CaptionSpec {
            text,
            appear,
            out: out_t,
            fade,
            fx,
            fy,
            size,
            scroll,
        });
    }
    out
}

/// Spawn the caption UI nodes ONCE, after the OrbitCam exists (Update + a `done` guard — the camera is
/// a Startup spawn, so a Startup-system query could race it). The font is registered once into Assets.
fn spawn_captions(
    mut commands: Commands,
    caps: Res<Captions>,
    cam_q: Query<Entity, With<OrbitCam>>,
    mut fonts: ResMut<Assets<Font>>,
    mut font_h: Local<Option<Handle<Font>>>,
    mut done: Local<bool>,
) {
    if *done || caps.0.is_empty() {
        return;
    }
    let Ok(cam) = cam_q.single() else { return }; // wait until the orbit camera is up
    let font = font_h
        .get_or_insert_with(|| {
            fonts.add(Font::try_from_bytes(FONT.to_vec()).expect("caption font"))
        })
        .clone();
    for c in &caps.0 {
        let mut entity = commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(c.fx * 100.0),
                top: Val::Percent(c.fy * 100.0),
                ..default()
            },
            GlobalZIndex(500), // under the loader (1000), over the splat render
            UiTargetCamera(cam), // composite into the splat camera → records headless
        ));
        if c.scroll != 0.0 {
            entity.insert(CaptionScroll {
                fy_pct: c.fy * 100.0,
                speed: c.scroll * 100.0,
                appear: c.appear,
            });
        }
        entity.with_children(|p| {
                p.spawn((
                    Text::new(c.text.clone()),
                    TextFont {
                        font: font.clone(),
                        font_size: c.size,
                        ..default()
                    },
                    TextColor(Color::srgba(0.96, 0.96, 1.0, 0.0)), // starts invisible; animate fades in
                    CaptionTag {
                        appear: c.appear,
                        out: c.out,
                        fade: c.fade,
                    },
                ));
            });
    }
    *done = true;
}

/// Ramp each caption's alpha from the show clock: smooth in over `fade` from `appear`, hold, smooth
/// out over `fade` ending `fade` after `out`. Pure function of `SeqClock.t` → deterministic (records
/// + seeks identically), the same in/out shape the compose stage uses.
fn animate_captions(clock: Res<SeqClock>, mut q: Query<(&CaptionTag, &mut TextColor)>) {
    let t = clock.t;
    for (c, mut col) in &mut q {
        let fin = ((t - c.appear) / c.fade).clamp(0.0, 1.0);
        let fout = ((c.out + c.fade - t) / c.fade).clamp(0.0, 1.0);
        col.0.set_alpha(fin.min(fout));
    }
}

/// Drift a caption's Y position upward from its base `fy` at `scroll` speed. The offset starts
/// at `appear` so the text scrolls in from below (fy ≥ 1.0) or scrolls away upward.
fn scroll_captions(clock: Res<SeqClock>, mut q: Query<(&CaptionScroll, &mut Node)>) {
    let t = clock.t;
    for (s, mut node) in &mut q {
        let elapsed = (t - s.appear).max(0.0);
        node.top = Val::Percent(s.fy_pct - s.speed * elapsed);
    }
}

/// The screen-anchored caption layer. Resource is inserted by `main` (it needs the parsed score).
pub(crate) struct CaptionPlugin;

impl Plugin for CaptionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (spawn_captions, animate_captions, scroll_captions));
    }
}
