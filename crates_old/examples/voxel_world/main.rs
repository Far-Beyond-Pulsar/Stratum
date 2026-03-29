//! `voxel_world` — Infinite procedurally-generated multi-biome voxel world.
//!
//! This example showcases the Stratum prefab system with:
//! * 7 biomes: Plains, Forest, Taiga, Desert, Swamp, Mountains, Jungle
//! * 7 tree types: Oak, Pine, Birch, Willow, Jungle, Cactus, Dead
//! * 7 house types: Cottage, Manor, Tower House, Market Stall, Barn, Desert House, Taiga Cabin
//! * Village infrastructure: Wells, Fountains, Lampposts
//! * Towers: Ruined Tower, Watchtower, Lighthouse
//! * Waterways: Aqueduct Pillars, Aqueduct Spans, Docks
//! * Water system at sea level with frozen ice in taiga biomes
//!
//! ## Controls
//! WASD fly | Space/Shift up/down | Mouse drag look (click to grab) | Tab mode | Esc exit
//!
//! ## Pass toggles (number keys)
//! 1 depth_prepass | 2 gbuffer | 3 deferred_lighting | 4 shadow | 5 geometry
//! 6 transparent | 7 sky | 8 radiance_cascades | 9 billboards | 0 ssao
//! P probes | R RC bounds | L portal

mod blocks;
mod noise;
mod biomes;
mod terrain;
mod materials;
mod camera;
mod chunks;
mod prefabs;
mod generation;
mod voxel_grid;
mod player;
mod app;

fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let no_fs = std::env::args().any(|a| a == "--no-fs");

    log::info!(
        "Multi-biome voxel world — chunk {}m, load radius {} → {} chunks max",
        terrain::CHUNK_SIZE as i32,
        terrain::LOAD_RADIUS,
        (terrain::LOAD_RADIUS * 2 + 1).pow(2) * terrain::MAX_Y_CHUNKS,
    );
    log::info!("Biomes: Plains, Forest, Taiga, Desert, Swamp, Mountains, Jungle");
    log::info!("WASD fly | Space/Shift up/down | Mouse look (click) | Tab mode | Esc exit");
    if no_fs {
        log::info!("Flag enabled: --no-fs (chunk filesystem reads/writes disabled)");
    }

    winit::event_loop::EventLoop::new().expect("event loop")
        .run_app(&mut app::App::new(no_fs))
        .expect("run_app failed");
}
