//! Live audio: monitor Cinder's synth while flying (the recorder muxes the WAV instead).
//!
//! The synth is **streamed**: `audio::stream::produce` renders the track in time-ordered segments
//! on a background thread, pushing each into a shared `StreamBuf`; a `StreamAudio` asset decodes
//! that growing buffer for bevy_audio. Playback (and the show clock, via `AudioGate`) starts as
//! soon as a short lead buffer (~2 s) is ready — about a second after launch — instead of waiting
//! for the whole track, and the producer (≈7× realtime) stays well ahead. Picture + music leave
//! together at t=0, so the score's `@@` anchors stay sample-locked. `MARTIN_MUSIC=<wav>` plays a
//! pre-rendered track and skips the producer entirely; the loader covers the brief lead-in.

use std::sync::Arc;

use bevy::audio::{AddAudioSource, AudioPlayer, AudioSource, Decodable, PlaybackSettings};
use bevy::prelude::*;

use crate::audio::stream::{StreamBuf, StreamDecoder};
use crate::scene::SeqClock;
use crate::scene::compose::Composition;
use crate::scene::sequence::SeqState;
use crate::{audio, score};

/// The loaded score (`MARTIN_SCORE` file or built-in), shared for live-audio rendering.
#[derive(Resource, Clone)]
pub(crate) struct ScoreRes(pub std::sync::Arc<score::Score>);

/// Sync gate: while live audio is wanted but the lead buffer isn't ready, the show clock
/// (`advance_seq_clock`) holds at 0 — starting the picture without the music would put every `@@`
/// anchor out of sync. Not inserted when muted/recording (no gate → no hold).
#[derive(Resource, Default)]
pub(crate) struct AudioGate {
    pub ready: bool,
}

/// A bevy-audio asset that decodes the growing `StreamBuf` (interleaved stereo f32 @ SAMPLE_RATE).
#[derive(Asset, TypePath, Clone)]
struct StreamAudio(Arc<StreamBuf>);

impl Decodable for StreamAudio {
    type DecoderItem = f32;
    type Decoder = StreamDecoder;
    fn decoder(&self) -> StreamDecoder {
        StreamDecoder::new(self.0.clone())
    }
}

/// What the live audio is playing: either a streamed render (the producer fills `buf`) or a
/// pre-rendered WAV (`MARTIN_MUSIC`). `entity`/`prev_t` track the spawned player + clock for restart.
#[derive(Resource)]
struct Music {
    stream: Option<Arc<StreamBuf>>, // streamed render (None when MARTIN_MUSIC is used)
    wav: Option<Handle<AudioSource>>, // pre-rendered WAV handle (MARTIN_MUSIC path)
    stream_handle: Option<Handle<StreamAudio>>,
    entity: Option<Entity>,
    prev_t: f32,
}

/// Startup: kick off the streamed synth render (unless recording / screenshotting / muted — the
/// recorder muxes the WAV separately), or load the pre-rendered `MARTIN_MUSIC` WAV.
fn spawn_synth(mut commands: Commands, asset_server: Res<AssetServer>, score_res: Res<ScoreRes>) {
    let want_audio = std::env::var("MARTIN_RECORD").is_err()
        && std::env::var("MARTIN_SHOT").is_err()
        && std::env::var("MARTIN_MUTE").is_err();
    if !want_audio {
        return;
    }
    commands.insert_resource(AudioGate::default());
    if let Ok(path) = std::env::var("MARTIN_MUSIC") {
        // Pre-rendered (bundled) audio — load instantly so it plays in sync, no synth render.
        let handle = asset_server.load::<AudioSource>(path.clone());
        commands.insert_resource(Music {
            stream: None,
            wav: Some(handle),
            stream_handle: None,
            entity: None,
            prev_t: 0.0,
        });
        info!("live audio: playing pre-rendered track ({path})");
        return;
    }
    // Stream the synth: render in time-ordered segments on a background thread, pushing each into a
    // shared buffer the StreamAudio asset reads. Playback starts at the lead buffer (see music_director).
    let sr = audio::SAMPLE_RATE as f32;
    let total_frames = (score_res.0.demo_len() * sr).ceil() as usize;
    let buf = Arc::new(StreamBuf::new(total_frames));
    let producer_buf = buf.clone();
    let score = score_res.0.clone();
    std::thread::spawn(move || {
        audio::stream::produce(&score, |chunk| producer_buf.push(chunk));
    });
    commands.insert_resource(Music {
        stream: Some(buf),
        wav: None,
        stream_handle: None,
        entity: None,
        prev_t: 0.0,
    });
    info!(
        "live audio: streaming Cinder's synth — playback starts at the lead buffer (~1s) \
         (MARTIN_MUTE=1 to skip, MARTIN_MUSIC=<wav> for a pre-rendered track)"
    );
}

/// Spawn the player once it's ready + the show is built, and open the gate so the clock starts in
/// sync. For the stream that's when the lead buffer is filled; for a WAV, when the asset has loaded.
/// Restart on a clock reset (Space).
#[allow(clippy::too_many_arguments)]
fn music_director(
    mut commands: Commands,
    music: Option<ResMut<Music>>,
    gate: Option<ResMut<AudioGate>>,
    state: Option<Res<SeqState>>,
    comp: Option<Res<Composition>>,
    clock: Res<SeqClock>,
    asset_server: Res<AssetServer>,
    mut stream_assets: ResMut<Assets<StreamAudio>>,
) {
    let Some(mut music) = music else { return };

    // clock jumped backwards (Space restart) → despawn so it respawns from the top, resynced.
    if clock.t + 0.05 < music.prev_t {
        if let Some(e) = music.entity.take() {
            commands.entity(e).despawn();
        }
    }
    music.prev_t = clock.t;

    if music.entity.is_some() {
        return; // already playing
    }

    // Clone the source handles first so we can mutate `music` below without aliasing it.
    let stream_buf = music.stream.clone();
    let wav = music.wav.clone();
    // ready? stream → lead buffer filled (or the whole short track done); WAV → asset loaded.
    let ready = if let Some(buf) = &stream_buf {
        let lead = audio::STREAM_LEAD_FRAMES.min(buf.total_frames());
        buf.finalized_frames() >= lead
    } else if let Some(h) = &wav {
        asset_server.is_loaded_with_dependencies(h.id())
    } else {
        false
    };
    let built = state.map(|s| s.built).unwrap_or(false) || comp.map(|c| c.built).unwrap_or(false);
    if !ready || !built {
        return;
    }

    let entity = if let Some(buf) = &stream_buf {
        let h = music
            .stream_handle
            .get_or_insert_with(|| stream_assets.add(StreamAudio(buf.clone())))
            .clone();
        commands
            .spawn((AudioPlayer(h), PlaybackSettings::ONCE))
            .id()
    } else if let Some(h) = &wav {
        commands
            .spawn((AudioPlayer(h.clone()), PlaybackSettings::ONCE))
            .id()
    } else {
        return;
    };
    music.entity = Some(entity);
    if let Some(mut gate) = gate {
        gate.ready = true; // release the show clock — picture + music start together
    }
    info!("live audio: playback started — show clock released");
}

/// Streamed (or pre-rendered) live playback, gated to start in sync with the show.
pub(crate) struct MusicPlugin;

impl Plugin for MusicPlugin {
    fn build(&self, app: &mut App) {
        // add_audio_source registers the StreamAudio asset + its playback systems.
        app.add_audio_source::<StreamAudio>()
            .add_systems(Startup, spawn_synth)
            .add_systems(Update, music_director);
    }
}
