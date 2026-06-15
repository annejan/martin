//! Beat-reactive visuals: the score's drum hits drive a per-frame pulse envelope that the
//! sequence + compose directors read to thump the cloud (scale), flare the bloom (opacity), and
//! swell any active deform — so the visuals ride Cinder's track, not just the `@@anchor` cues.
//! `MARTIN_BEAT=<scale>` tunes the strength (default 1.0, `0` = off).

use bevy::prelude::*;

use crate::music::ScoreRes;
use crate::scene::SeqClock;
use crate::score::Inst;

/// The hit times (s) for each drum lane, precomputed once, + the user's strength scale.
#[derive(Resource)]
struct BeatTrack {
    kick: Vec<f32>,
    snare: Vec<f32>,
    hat: Vec<f32>,
    intensity: f32,
}

/// The current pulse per lane (1.0 on a hit, decaying to 0), read by the directors each frame.
#[derive(Resource, Default)]
pub(crate) struct Beat {
    pub kick: f32,
    pub snare: f32,
    pub hat: f32,
    pub level: f32,     // section gain (overall dynamics) at the moment
    pub intensity: f32, // MARTIN_BEAT scale (0 = off)
}

impl Beat {
    /// Pack the per-lane pulses for a fullscreen-effect uniform: `x=kick y=snare z=hat w=intensity`
    /// (the layout `bg.wgsl` / `shader_part.wgsl` expect). Shared by the background + interlude layers.
    pub(crate) fn as_vec4(&self) -> Vec4 {
        Vec4::new(self.kick, self.snare, self.hat, self.intensity)
    }
}

/// Decaying envelope: 1.0 at a hit, `exp(-dt/tau)` after — `tau` sets the snap.
fn pulse(times: &[f32], t: f32, tau: f32) -> f32 {
    let i = times.partition_point(|&x| x <= t);
    if i == 0 {
        return 0.0;
    }
    (-(t - times[i - 1]) / tau).exp()
}

fn setup_beat_track(score: Res<ScoreRes>, mut commands: Commands) {
    let s = &score.0;
    let intensity = std::env::var("MARTIN_BEAT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    commands.insert_resource(BeatTrack {
        kick: s.hits(Inst::Kick),
        snare: s.hits(Inst::Snare),
        hat: s.hits(Inst::Hat),
        intensity,
    });
}

fn track_beat(
    track: Option<Res<BeatTrack>>,
    score: Res<ScoreRes>,
    clock: Res<SeqClock>,
    // `[sync]` look-track: when it keyframes `beat`, it overrides the static MARTIN_BEAT intensity.
    sync: Option<Res<crate::sync::SyncTrack>>,
    mut beat: ResMut<Beat>,
) {
    let Some(track) = track else { return };
    let t = clock.t;
    beat.kick = pulse(&track.kick, t, 0.09);
    beat.snare = pulse(&track.snare, t, 0.13);
    beat.hat = pulse(&track.hat, t, 0.05);
    beat.level = score.0.gain_at(t);
    beat.intensity = sync.and_then(|s| s.beat_at(t)).unwrap_or(track.intensity);
}

/// Registered by `ScenePlugin`: own the `Beat` resource + refresh it each frame before the directors.
pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<Beat>()
        .add_systems(Startup, setup_beat_track)
        .add_systems(Update, track_beat);
}
