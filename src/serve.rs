//! `MARTIN_SERVE=<port|1>`: a live control bridge. Boots the show **windowed**, but renders the
//! splats into an **offscreen image** (shown in the window via a 2D blit camera) so screenshots are
//! window-independent — no black-on-unfocused, works over SSH. Serves a newline-delimited JSON
//! command protocol on `127.0.0.1:<port>` (default 7878): drive the camera + clock live and grab
//! screenshots **without reloading the (possibly huge) show**. The slow edit→spawn→reload→blind-frame
//! loop is gone. This is the engine half of "full MCP"; a stdio MCP front (`martin --mcp`) proxies to
//! this port.
//!
//! Protocol — one JSON object per line, one JSON reply per line. Commands:
//! ```text
//! {"cmd":"camera","dist":0.6,"yaw":1.4,"pitch":0.18,"pos":[0,0.1,0]}  # any field optional
//! {"cmd":"seek","t":25.0}            {"cmd":"pause"}   {"cmd":"play"}   {"cmd":"step","dt":0.1}
//! {"cmd":"screenshot","path":"/tmp/m.png"}            # writes the PNG, returns its path
//! {"cmd":"dump_camera"}              # → a ready-to-paste [camera] line for the current pose+time
//! {"cmd":"state"}                    # → current t, paused, camera
//! ```

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use bevy::asset::RenderAssetUsages;
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use serde_json::{Value, json};

use crate::camera::OrbitCam;
use crate::scene::SeqClock;

const DEFAULT_PORT: u16 = 7878;

/// The port `MARTIN_SERVE` asks for (`1`/`on`/empty → the default), or `None` when serving is off.
pub(crate) fn serve_port() -> Option<u16> {
    let v = std::env::var("MARTIN_SERVE").ok()?;
    let v = v.trim();
    if v.is_empty() || v == "1" || v.eq_ignore_ascii_case("on") {
        return Some(DEFAULT_PORT);
    }
    Some(v.parse().unwrap_or(DEFAULT_PORT))
}

/// True when the live control bridge owns the session — the camera flypath + the live auto-exit stand
/// down so the bridge can drive the camera manually and run indefinitely.
pub(crate) fn is_serving() -> bool {
    serve_port().is_some()
}

/// A client command (parsed JSON) + the channel its reply goes back on.
struct Req {
    v: Value,
    reply: Sender<String>,
}

/// The receiver lives behind a `Mutex` only because Bevy resources must be `Sync` (an mpsc `Receiver`
/// isn't); it's drained from the single Bevy thread, so the lock is never contended.
#[derive(Resource)]
struct Inbox(Mutex<Receiver<Req>>);

/// Set by the bridge's `pause`/`play`; consulted by the clock so the show can hold for inspection.
#[derive(Resource, Default)]
pub(crate) struct Paused(pub bool);

/// The offscreen image the splats render into (and the screenshot reads).
#[derive(Resource)]
struct ServeImage(Handle<Image>);

#[derive(Resource, Default)]
struct Attached(bool);

pub(crate) struct ServePlugin;

impl Plugin for ServePlugin {
    fn build(&self, app: &mut App) {
        let Some(port) = serve_port() else { return };
        let (tx, rx) = channel();
        std::thread::spawn(move || accept_loop(port, tx));
        app.insert_resource(Inbox(Mutex::new(rx)))
            .init_resource::<Paused>()
            .init_resource::<Attached>()
            .add_systems(Startup, setup_image)
            .add_systems(Update, (attach_and_blit, drain));
    }
}

/// Bind the port and serve each client on its own thread: read a JSON line, hand it to Bevy, write the
/// reply line back. Runs off the Bevy thread so a blocked client never stalls the render.
fn accept_loop(port: u16, tx: Sender<Req>) {
    let listener = match TcpListener::bind(("127.0.0.1", port)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("serve: cannot bind 127.0.0.1:{port}: {e}");
            return;
        }
    };
    eprintln!("serve: live control bridge on 127.0.0.1:{port}");
    for stream in listener.incoming().flatten() {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let reader = match stream.try_clone() {
                Ok(s) => BufReader::new(s),
                Err(_) => return,
            };
            let mut writer = stream;
            for line in reader.lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let reply = match serde_json::from_str::<Value>(&line) {
                    Ok(v) => {
                        let (rtx, rrx) = channel();
                        if tx.send(Req { v, reply: rtx }).is_err() {
                            break;
                        }
                        match rrx.recv() {
                            Ok(r) => r,
                            Err(_) => break,
                        }
                    }
                    Err(e) => json!({"ok": false, "error": format!("bad json: {e}")}).to_string(),
                };
                if writeln!(writer, "{reply}").is_err() {
                    break;
                }
            }
        });
    }
}

/// Create the window-sized offscreen render target (once, at startup).
fn setup_image(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let size = Extent3d {
        width: 1280,
        height: 720,
        depth_or_array_layers: 1,
    };
    let mut image = Image::new_fill(
        size,
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::all(),
    );
    image.texture_descriptor.usage =
        TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    commands.insert_resource(ServeImage(images.add(image)));
}

/// Once the splat camera exists: point it at the offscreen image, and add a 2D camera + a fullscreen
/// `ImageNode` so the window *shows* that image (the live view) while screenshots read it directly.
fn attach_and_blit(
    mut commands: Commands,
    img: Option<Res<ServeImage>>,
    cams: Query<Entity, With<OrbitCam>>,
    mut done: ResMut<Attached>,
) {
    if done.0 {
        return;
    }
    let Some(img) = img else { return };
    let mut attached = false;
    for e in &cams {
        commands.entity(e).insert(RenderTarget::Image(ImageRenderTarget {
            handle: img.0.clone(),
            scale_factor: 1.0,
        }));
        attached = true;
    }
    if attached {
        // a cheap 2D camera composites the offscreen render into the window (no second splat pass).
        commands.spawn(Camera2d);
        commands.spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            ImageNode::new(img.0.clone()),
        ));
        done.0 = true;
    }
}

/// Drain queued client commands each frame, apply them, and reply.
fn drain(
    inbox: Res<Inbox>,
    img: Option<Res<ServeImage>>,
    mut cams: Query<&mut OrbitCam>,
    mut clock: ResMut<SeqClock>,
    mut paused: ResMut<Paused>,
    mut commands: Commands,
) {
    let Ok(rx) = inbox.0.lock() else { return };
    while let Ok(req) = rx.try_recv() {
        let reply = handle(
            &req.v,
            img.as_deref(),
            &mut cams,
            &mut clock,
            &mut paused,
            &mut commands,
        );
        let _ = req.reply.send(reply.to_string());
    }
}

/// One pose, as JSON, for `state`/`dump_camera`/`camera` replies.
fn cam_json(c: &OrbitCam) -> Value {
    json!({
        "dist": c.dist, "yaw": c.yaw, "pitch": c.pitch,
        "pos": [c.target.x, c.target.y, c.target.z],
    })
}

fn handle(
    v: &Value,
    img: Option<&ServeImage>,
    cams: &mut Query<&mut OrbitCam>,
    clock: &mut SeqClock,
    paused: &mut Paused,
    commands: &mut Commands,
) -> Value {
    let cmd = v.get("cmd").and_then(Value::as_str).unwrap_or("");
    let num = |k: &str| v.get(k).and_then(Value::as_f64).map(|x| x as f32);
    match cmd {
        "camera" => {
            for mut c in cams.iter_mut() {
                if let Some(d) = num("dist") {
                    c.dist = d;
                }
                if let Some(y) = num("yaw") {
                    c.yaw = y;
                }
                if let Some(p) = num("pitch") {
                    c.pitch = p;
                }
                if let Some(p) = v.get("pos").and_then(Value::as_array) {
                    let g = |i: usize| p.get(i).and_then(Value::as_f64).unwrap_or(0.0) as f32;
                    c.target = Vec3::new(g(0), g(1), g(2));
                }
            }
            let cam = cams.iter().next().map(cam_json).unwrap_or(Value::Null);
            json!({"ok": true, "camera": cam})
        }
        "seek" => {
            if let Some(t) = num("t") {
                clock.t = t.max(0.0);
            }
            json!({"ok": true, "t": clock.t})
        }
        "pause" => {
            paused.0 = true;
            json!({"ok": true, "paused": true})
        }
        "play" => {
            paused.0 = false;
            json!({"ok": true, "paused": false})
        }
        "step" => {
            clock.t = (clock.t + num("dt").unwrap_or(0.1)).max(0.0);
            json!({"ok": true, "t": clock.t})
        }
        "screenshot" => {
            let path = v
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or("/tmp/martin_serve.png")
                .to_string();
            match img {
                Some(img) => {
                    commands
                        .spawn(Screenshot::image(img.0.clone()))
                        .observe(save_to_disk(path.clone()));
                    // the PNG lands a frame or two later (GPU readback); the caller should briefly wait.
                    json!({"ok": true, "path": path, "note": "written shortly after this reply"})
                }
                None => json!({"ok": false, "error": "no render target yet"}),
            }
        }
        "dump_camera" => {
            let line = cams
                .iter()
                .next()
                .map(|c| {
                    format!(
                        "t={:.1}   pos={:.2},{:.2},{:.2}   dist={:.2}   yaw={:.3}   pitch={:.3}",
                        clock.t, c.target.x, c.target.y, c.target.z, c.dist, c.yaw, c.pitch
                    )
                })
                .unwrap_or_default();
            json!({"ok": true, "line": line})
        }
        "state" => {
            let cam = cams.iter().next().map(cam_json).unwrap_or(Value::Null);
            json!({"ok": true, "t": clock.t, "paused": paused.0, "camera": cam})
        }
        other => json!({"ok": false, "error": format!("unknown cmd '{other}'")}),
    }
}
