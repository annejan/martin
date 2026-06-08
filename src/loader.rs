//! A minimal loading screen (`MARTIN_LOADER=1`, set automatically in a bundled build): a black
//! cover with the logo (`MARTIN_LOGO=<png in the asset root>`) and a slim progress bar that tracks
//! how many splats have loaded, lifted away once the show is built. Off by default — dev runs skip it.

use bevy::prelude::*;
use bevy_gaussian_splatting::PlanarGaussian3d;

use crate::scene::compose::Composition;
use crate::scene::sequence::SeqState;

#[derive(Component)]
struct LoaderRoot;

#[derive(Component)]
struct LoaderFill;

fn spawn_loader(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(22.0),
                ..default()
            },
            BackgroundColor(Color::BLACK),
            GlobalZIndex(1000),
            LoaderRoot,
        ))
        .with_children(|p| {
            if let Ok(logo) = std::env::var("MARTIN_LOGO") {
                p.spawn((
                    Node {
                        width: Val::Px(480.0),
                        height: Val::Auto,
                        ..default()
                    },
                    ImageNode::new(asset_server.load(logo)),
                ));
            }
            // progress track + fill — a thin dim sliver; the logo is the star, and the show flows
            // OUT of it (the loader's logo → the demo's crisp logo mesh → splats).
            p.spawn((
                Node {
                    width: Val::Px(300.0),
                    height: Val::Px(3.0),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.10, 0.10, 0.13)),
            ))
            .with_children(|track| {
                track.spawn((
                    Node {
                        width: Val::Percent(0.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.85, 0.88, 1.0)),
                    LoaderFill,
                ));
            });
        });
}

/// Drive the bar from splat-load progress; lift the cover once the show is built.
fn update_loader(
    mut commands: Commands,
    assets: Res<Assets<PlanarGaussian3d>>,
    state: Option<Res<SeqState>>,
    comp: Option<Res<Composition>>,
    mut fill: Query<&mut Node, With<LoaderFill>>,
    root: Query<Entity, With<LoaderRoot>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let (loaded, total) = state
        .as_ref()
        .map(|s| {
            (
                s.loads.iter().filter(|h| assets.get(*h).is_some()).count(),
                s.loads.len().max(1),
            )
        })
        .unwrap_or((0, 1));
    let pct = (loaded as f32 / total as f32 * 100.0).clamp(0.0, 100.0);
    for mut node in &mut fill {
        node.width = Val::Percent(pct);
    }
    // built = the show is on screen → drop the cover.
    let built = state.map(|s| s.built).unwrap_or(false) || comp.map(|c| c.built).unwrap_or(false);
    if built {
        for e in &root {
            commands.entity(e).despawn();
        }
        *done = true;
    }
}

/// The loading screen — only active when `MARTIN_LOADER` is set (bundled builds set it).
pub(crate) struct LoaderPlugin;

impl Plugin for LoaderPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("MARTIN_LOADER").is_some() {
            app.add_systems(Startup, spawn_loader)
                .add_systems(Update, update_loader);
        }
    }
}
