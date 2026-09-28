//! Sample BN6's random stages: boot a training battle for each of N seeds,
//! read the starting panels and obstacles, and screenshot any stage with a
//! panel kind not yet named, to map the rest of the panel types.
//!
//! bn6_stages <save.sav> <out dir> <count>

use std::collections::BTreeMap;
use std::sync::Arc;

use tango_gamesupport_bn6::observe::{Bn6Obs, PanelKind};
use tango_session::Session;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, save, out, count] = &args[..] else {
        panic!("usage: bn6_stages <save.sav> <out dir> <count>");
    };
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    let sram = std::fs::read(save).expect("read save");
    let (game, rom) = tango_library::bnlc::scan_steam_roms()
        .into_iter()
        .find(|(g, _)| g.family_and_variant() == ("bn6", 0))
        .expect("BN6 Gregar (US) in the Legacy Collection");
    let rom = Arc::new(rom);
    let mut kinds: BTreeMap<String, (u32, u64)> = BTreeMap::new();
    let mut objects: BTreeMap<String, u32> = BTreeMap::new();
    for seed in 0..count.parse::<u64>().unwrap() {
        let mut s = [0u8; 16];
        s[..8].copy_from_slice(&seed.to_le_bytes());
        let (session, mut driver, _audio) = tango_session::training::TrainingSession::new(
            game,
            rom.clone(),
            sram.clone(),
            std::time::SystemTime::UNIX_EPOCH,
            s,
            48000,
        )
        .expect("boot");
        let mut obs = None;
        for _ in 0..600 {
            driver.tick();
            obs = driver
                .last_battle()
                .and_then(|b| b.detail[0].as_ref()?.downcast_ref::<Bn6Obs>().cloned());
            if obs.is_some() {
                break;
            }
        }
        let Some(o) = obs else { continue };
        let mut new = false;
        for p in o.panels.iter().flatten() {
            let k = format!("{:?}", p.kind);
            let e = kinds.entry(k).or_insert((0, seed));
            if e.0 == 0 && matches!(p.kind, PanelKind::Unknown(_)) {
                new = true;
            }
            e.0 += 1;
        }
        for ob in &o.obstacles {
            let e = objects.entry(format!("{:?}", ob.kind)).or_default();
            if *e == 0 && matches!(ob.kind, tango_gamesupport_bn6::observe::ObstacleKind::Unknown(_)) {
                new = true;
                println!(
                    "seed {seed}: new object {:?} at {:?} hp {}/{}",
                    ob.kind, ob.tile, ob.hp, ob.max_hp
                );
            }
            *e += 1;
        }
        if new {
            // Past the battle's opening flash.
            for _ in 0..200 {
                driver.tick();
            }
            let (w, h) = session.frame_size();
            image::save_buffer(
                out.join(format!("seed{seed}.png")),
                &session.frame(),
                w,
                h,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
            let rows: Vec<String> = o
                .panels
                .iter()
                .map(|r| r.iter().map(|p| format!("{:?}", p.kind)).collect::<Vec<_>>().join(" "))
                .collect();
            println!("seed {seed}: new kind\n  {}", rows.join("\n  "));
        }
    }
    println!("panel kinds (tiles seen, first seed): {kinds:?}");
    println!("starting objects: {objects:?}");
}
