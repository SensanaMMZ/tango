//! Print a BN6 save's equipped folder with chip names from the ROM, for
//! planning `bn6_explore` scripts.
//!
//! bn6_folder <save.sav>

fn main() {
    let path = std::env::args().nth(1).expect("usage: bn6_folder <save.sav>");
    let sram = std::fs::read(path).expect("read save");
    let (game, rom) = tango_library::bnlc::scan_steam_roms()
        .into_iter()
        .find(|(g, _)| g.family_and_variant() == ("bn6", 0))
        .expect("BN6 Gregar (US) in the Legacy Collection");
    let save = game.parse_save(&sram).expect("parse save");
    let save = tango_gamesupport_common_dataview::save_ref(save.as_ref());
    let wram = save.as_raw_wram().into_owned();
    let assets = game.load_rom_assets(&rom, &wram, None).expect("rom assets");
    let assets = tango_gamesupport_common_dataview::assets_ref(assets.as_ref());
    let chips = save.view_chips().expect("chips view");
    let folder = chips.equipped_folder_index();
    println!("folder {folder} of {}", chips.num_folders());
    for i in 0..chips.folder_size() {
        let Some(c) = chips.chip(folder, i) else { continue };
        let info = assets.chip(c.id);
        let name = info.as_ref().and_then(|i| i.name()).unwrap_or_default();
        let class = info.as_ref().map(|i| format!("{:?}", i.class())).unwrap_or_default();
        println!("{i:2}  id {:3}  {:?}  {name:<14} {class}", c.id, c.code);
    }
}
