//! Screen-anchored captions: title/credit pinned to screen space — unlike `text:` which renders
//! as world-space gaussians the camera moves past. Authored in a `.show` `[caption]` track:
//!
//! ```text
//! [caption]
//! screentext:deFEEST           in @@intro  out @@drop        at 0.5,0.08  size 72
//! screentext:VJ annejan        in @@outro  out @@outro+4bar  at 0.04,0.9  size 40
//! screentext:SCENE & CAMP      in @@outro  out @@bar:128     center       size 30
//! screentext:a deFEEST product in @@outro  out @@bar:124     center       size 26  scroll 0.05
//! ```
//!
//! Built on bevy_ui `Text` — the root carries `UiTargetCamera` so captions bake into recordings.
//! Alpha is a pure function of `t` (no randomness, no state) → deterministic seeks & recordings.

use bevy::prelude::*;
use bevy::text::Font;
use bevy::ui::UiTargetCamera;

use crate::camera::OrbitCam;
use crate::scene::SeqClock;
use crate::score::Score;

/// Bundled font.
static FONT: &[u8] = include_bytes!("../../assets/font.ttf");

// ── data ───────────────────────────────────────────────────────────────────

/// One parsed caption.
#[derive(Clone)]
pub struct CaptionSpec {
    pub text: String,
    pub appear: f32,  // fade-in start (show seconds)
    pub out: f32,     // fade-out start (show seconds); f32::MAX = stick to end
    pub fade: f32,    // fade in/out duration (s)
    pub fx: f32,      // screen-x fraction of left edge (used only when !center)
    pub fy: f32,      // screen-y fraction; >1 = below-screen credit scroll
    pub size: f32,    // font size (px)
    pub center: bool, // true → flex-centre text horizontally on screen
    pub scroll: f32,  // upward scroll speed (fraction of screen / s); 0 = static
}

/// The parsed `[caption]` track.
#[derive(Resource, Default)]
pub struct Captions(pub Vec<CaptionSpec>);

// ── ECS components (internal) ──────────────────────────────────────────────

/// Fade timing on each caption text entity.
#[derive(Component)]
struct CaptionTag {
    appear: f32,
    out: f32,
    fade: f32,
}

/// Scroll state on the parent Node — drives Y position from the show clock.
#[derive(Component)]
struct CaptionScroll {
    fy_pct: f32, // base screen-y as percentage (0..100)
    speed: f32,  // percentage-points per second, positive = upward
    appear: f32,
}

// ── parser ─────────────────────────────────────────────────────────────────

/// Parse `[caption]` lines into specs. Forgiving: warn + skip bad lines.
///
/// Grammar (all tokens optional except `screentext:`):
///   `screentext:<text…>  [in <anchor>] [out <anchor>] [at fx,fy] [size px] [fade s] [center] [scroll s]`
pub fn parse_captions(lines: &[String], score: &Score) -> Vec<CaptionSpec> {
    let is_kw = |t: &str| {
        matches!(
            t,
            "in" | "out" | "at" | "size" | "fade" | "center" | "scroll"
        )
    };
    // A trailing-options run: a contiguous block of `keyword [value]` pairs (`center` takes no value)
    // that consumes to the end with no stray token left over. Used to split text from options so a
    // lyric word that happens to BE a keyword (e.g. "beessies in me thee") doesn't truncate the text —
    // "in me thee …" leaves "thee" stray, so it isn't a valid options run and stays part of the text.
    let opts_valid = |s: &[&str]| {
        let mut i = 0;
        while i < s.len() {
            match s[i] {
                "center" => i += 1,
                "in" | "out" | "at" | "size" | "fade" | "scroll" => {
                    if i + 1 >= s.len() {
                        return false; // keyword with no value
                    }
                    i += 2;
                }
                _ => return false, // stray non-keyword token
            }
        }
        !s.is_empty()
    };
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
        // Rebuild the token list with the `screentext:` prefix stripped from the head, then split it
        // into text + options at the FIRST keyword that begins a valid trailing-options run (so a
        // stray keyword word inside the lyric doesn't cut the text short — see `opts_valid`).
        let mut words: Vec<&str> = toks.clone();
        words[0] = first;
        let mut i = words.len();
        for k in 1..words.len() {
            if is_kw(words[k]) && opts_valid(&words[k..]) {
                i = k;
                break;
            }
        }
        let text = words[..i].join(" ");
        let toks = words; // options loop below indexes from `i`
        let (mut appear, mut out_t, mut fade, mut fx, mut fy, mut size, mut center, mut scroll) = (
            0.0_f32,
            f32::MAX,
            0.6_f32,
            0.5_f32,
            0.1_f32,
            56.0_f32,
            false,
            0.0_f32,
        );
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
                "center" => center = true, // boolean flag, no value
                "scroll" => scroll = val.parse().unwrap_or(0.0),
                _ => {}
            }
            // `center` is a value-less flag — advance ONE token, not two, or it eats the next
            // key (e.g. `size`/`scroll`) and desyncs every remaining key=value pair.
            i += if toks[i] == "center" { 1 } else { 2 };
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
            center,
            scroll,
        });
    }
    out
}

// ── spawn ──────────────────────────────────────────────────────────────────

/// Spawn caption UI nodes once (guarded by `done`).
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
    let Ok(cam) = cam_q.single() else { return };
    let font = font_h
        .get_or_insert_with(|| {
            fonts.add(Font::try_from_bytes(FONT.to_vec()).expect("caption font"))
        })
        .clone();

    for c in &caps.0 {
        // Build node: flex-centred full-width or left-anchored at fx
        let node = if c.center {
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(0.0),
                right: Val::Percent(0.0),
                top: Val::Percent(c.fy * 100.0),
                display: Display::Flex,
                justify_content: JustifyContent::Center,
                // pad the edges so a long lyric line wraps WITHIN the frame (and the wrapped
                // lines centre, via the child's `Justify::Center`) instead of hugging x=0.
                padding: UiRect::horizontal(Val::Percent(6.0)),
                ..default()
            }
        } else {
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(c.fx * 100.0),
                top: Val::Percent(c.fy * 100.0),
                ..default()
            }
        };

        let mut entity = commands.spawn((node, GlobalZIndex(500), UiTargetCamera(cam)));

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
                TextColor(Color::srgba(0.96, 0.96, 1.0, 0.0)),
                TextLayout::new_with_justify(if c.center {
                    Justify::Center
                } else {
                    Justify::Left
                }),
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

// ── animate ────────────────────────────────────────────────────────────────

/// Ramp caption alpha from the show clock.
fn animate_captions(clock: Res<SeqClock>, mut q: Query<(&CaptionTag, &mut TextColor)>) {
    let t = clock.t;
    for (c, mut col) in &mut q {
        let fin = ((t - c.appear) / c.fade).clamp(0.0, 1.0);
        let fout = ((c.out + c.fade - t) / c.fade).clamp(0.0, 1.0);
        col.0.set_alpha(fin.min(fout));
    }
}

/// Scroll caption Y position upward from base fy.
fn scroll_captions(clock: Res<SeqClock>, mut q: Query<(&CaptionScroll, &mut Node)>) {
    let t = clock.t;
    for (s, mut node) in &mut q {
        let elapsed = (t - s.appear).max(0.0);
        node.top = Val::Percent(s.fy_pct - s.speed * elapsed);
    }
}

// ── plugin ─────────────────────────────────────────────────────────────────

pub(crate) struct CaptionPlugin;

impl Plugin for CaptionPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (spawn_captions, animate_captions, scroll_captions));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::Score;

    fn cap(line: &str) -> CaptionSpec {
        let v = parse_captions(&[line.to_string()], &Score::builtin());
        assert_eq!(v.len(), 1, "expected one caption from: {line}");
        v.into_iter().next().unwrap()
    }

    #[test]
    fn keyword_word_inside_lyric_is_kept_as_text() {
        // a lyric containing the bare keyword "in" must not truncate the caption text — the real
        // options run starts at the LAST "in 50 out 60 …" and the stray "in me thee" stays text.
        let c = cap("screentext:dooie beessies in me thee in 50 out 60 center");
        assert_eq!(c.text, "dooie beessies in me thee");
        assert_eq!(c.appear, 50.0);
        assert_eq!(c.out, 60.0);
        assert!(c.center);
    }

    #[test]
    fn normal_line_splits_text_and_options() {
        let c = cap("screentext:Op de camping in 10 out 20 at 0.5,0.84 size 34 fade 0.3");
        assert_eq!(c.text, "Op de camping");
        assert_eq!(c.appear, 10.0);
        assert_eq!(c.out, 20.0);
        assert_eq!(c.fx, 0.5);
        assert_eq!(c.fy, 0.84);
        assert_eq!(c.size, 34.0);
        assert_eq!(c.fade, 0.3);
    }

    #[test]
    fn text_only_line_keeps_full_text_and_defaults() {
        let c = cap("screentext:Op de camping tet tet");
        assert_eq!(c.text, "Op de camping tet tet");
        assert_eq!(c.appear, 0.0);
        assert_eq!(c.out, f32::MAX);
    }
}
