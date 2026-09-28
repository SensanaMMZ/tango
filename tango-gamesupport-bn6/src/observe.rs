//! What a CPU opponent may see of a BN6 battle: [`Bn6Obs`], read from one
//! core every tick and handed on as that core's telemetry
//! [`Detail`](tango_match::telemetry::Detail).
//!
//! A core's reading holds **its own player's** private state (the hand
//! and chip-select cursor, the queued chips) plus what both players see
//! on screen (forms, Beast Out turns, buster charge). It never carries
//! the other player's hand or queue, though the same RAM holds both: a
//! consumer acting for one player reads only that player's core.
//!
//! Every address behind these fields was found and checked with the
//! `bn6_explore` example (tango-session) on BN6 Gregar (US); see
//! `EWRAMOffsets` in [`crate::pvp`] for each one's evidence.

/// The custom gauge's value when full: the chip screen can be opened.
pub const CUSTOM_GAUGE_FULL: u8 = 64;

/// One core's view of a BN6 battle tick.
#[derive(Clone, Debug)]
pub struct Bn6Obs {
    /// The absolute player this core belongs to: whose private state
    /// `chip_select` and `queue` are.
    pub player: usize,
    /// The custom gauge, `0..=CUSTOM_GAUGE_FULL`. Full means L/R opens
    /// the chip screen (for both players at once, in a link battle).
    pub custom_gauge: u8,
    /// The chip screen, while it's open for this player.
    pub chip_select: Option<ChipSelect>,
    /// This player's chips selected and not yet used, next first.
    pub queue: Vec<u16>,
    /// Both players' visible state, by absolute player.
    pub units: [UnitDetail; 2],
    /// The field, `panels[y][x]` for tile `(x + 1, y + 1)`: rows top to
    /// bottom, columns left to right (columns 1-3 are player 0's side at
    /// the start of a battle).
    pub panels: [[Panel; 6]; 3],
    /// Obstacles still standing (HP left), the stage's own included:
    /// RockCube, music boxes, Guardian, bombs... Objects can be placed in
    /// either area, so the panel under one doesn't say who placed it.
    pub obstacles: Vec<Obstacle>,
    /// Probe-only: the raw battle RAM the fields above are read from.
    #[cfg(feature = "ram-probe")]
    pub raw: Option<Vec<u8>>,
}

/// This player's chip screen.
#[derive(Clone, Debug)]
pub struct ChipSelect {
    pub cursor: Cursor,
    /// The hand, in screen order: slots 0-4 are the top row. `None` for a
    /// slot already picked (or never dealt).
    pub hand: [Option<HandChip>; 8],
    /// What's been picked this chip select, in order, as cursor
    /// positions: hand slots, or [`Cursor::BEAST_OUT`] for Beast Out,
    /// which takes one of the five picks.
    pub picked: Vec<u8>,
    /// A form already chosen this chip select. The game allows one: a
    /// Cross greys out Beast Out, and vice versa.
    pub form_picked: Option<FormPick>,
}

/// Where the chip-screen cursor is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cursor {
    /// On hand slot `n`.
    Hand(u8),
    /// On OK: A confirms. START jumps here from anywhere.
    Ok,
    /// On the ★ under OK: A picks Beast Out.
    BeastOut,
    /// On the Cross bar (Up from the hand).
    CrossBar,
    /// In the open Cross list (Up again from the bar; B leaves).
    CrossList,
    /// A position not yet mapped.
    Other(u8),
}

impl Cursor {
    /// The raw cursor positions of OK and the ★ (Beast Out).
    pub const OK: u8 = 10;
    pub const BEAST_OUT: u8 = 11;
}

/// A chip in the hand.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HandChip {
    /// The chip's id, as the ROM's chip table (and the dataview) index it.
    pub id: u16,
    /// Its code: 0-25 for A-Z, 26 for `*`.
    pub code: u8,
}

impl HandChip {
    /// Decode a hand entry: `(code << 9) | id`, `0xFFFF` for none.
    pub fn from_raw(raw: u16) -> Option<Self> {
        (raw != 0xffff).then_some(HandChip {
            id: raw & 0x1ff,
            code: (raw >> 9) as u8,
        })
    }
}

/// One panel of the field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Panel {
    pub kind: PanelKind,
    /// The player whose area it is (changes when an area is stolen).
    pub owner: u8,
}

/// A panel's surface. Values found by using each chip in the
/// `bn6_explore` lab and checking screenshots.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelKind {
    /// A hole: nothing can stand here until it repairs.
    Broken,
    Normal,
    /// Breaks into a hole once stepped off.
    Cracked,
    Poison,
    Holy,
    Grass,
    Ice,
    /// Conveyor panels from GoingRd / ComingRd, named after the chip.
    /// ComingRd's carried a player standing on it toward the chip's user
    /// (player 1 laid it; player 0 slid from column 1 to 3); GoingRd's
    /// presumably push the other way (unconfirmed: the player on it was
    /// already against the back edge). Both take ~130 ticks to appear.
    GoingRoad,
    ComingRoad,
    /// Not yet mapped (volcano and others).
    Unknown(u8),
}

impl PanelKind {
    pub fn from_raw(raw: u8) -> Self {
        match raw {
            0x01 => PanelKind::Broken,
            0x02 => PanelKind::Normal,
            0x03 => PanelKind::Cracked,
            0x04 => PanelKind::Poison,
            0x05 => PanelKind::Holy,
            0x06 => PanelKind::Grass,
            0x07 => PanelKind::Ice,
            0x0b => PanelKind::GoingRoad,
            0x0c => PanelKind::ComingRoad,
            other => PanelKind::Unknown(other),
        }
    }
}

/// An obstacle on the field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Obstacle {
    pub kind: ObstacleKind,
    pub tile: (u8, u8),
    pub hp: u16,
    pub max_hp: u16,
}

/// What an obstacle is. The game's own kind byte; mapped from the
/// `bn6_explore` lab (LilBolr1 and Fanfare still unseen; the objects
/// AirSpin3 and AirRaid3 leave are unconfirmed).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ObstacleKind {
    RockCube,
    /// The cube some stages start with.
    StageCube,
    Fan,
    Discord,
    Timpani,
    Silence,
    Guardian,
    Sensor,
    BlackBomb,
    TimeBomb,
    Mine,
    VDoll,
    Unknown(u8),
}

impl ObstacleKind {
    pub fn from_raw(raw: u8) -> Self {
        match raw {
            0xd0 => ObstacleKind::RockCube,
            0xd1 => ObstacleKind::StageCube,
            0xd5 => ObstacleKind::BlackBomb,
            0xd7 => ObstacleKind::Fan,
            0xd8 => ObstacleKind::TimeBomb,
            0xda => ObstacleKind::Mine,
            0xde => ObstacleKind::Discord,
            0xdf => ObstacleKind::Timpani,
            0xe0 => ObstacleKind::Silence,
            0xe2 => ObstacleKind::VDoll,
            0xe3 => ObstacleKind::Guardian,
            0xe4 => ObstacleKind::Sensor,
            other => ObstacleKind::Unknown(other),
        }
    }
}

/// Which form was chosen on this chip screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormPick {
    Cross,
    BeastOut,
}

/// One player's state as both players see it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct UnitDetail {
    pub form: Form,
    /// Beast Out turns left (3 at the start of a battle).
    pub beast_turns_left: u8,
    pub charge: Charge,
    /// A barrier or aura, while it has HP left.
    pub barrier: Option<Barrier>,
    /// Ticks of invisibility left (Invisibl), 0 when visible.
    pub invisible_ticks: u16,
    /// The game's status flags for this player (`ObjectFlags1`).
    pub status: Status,
    /// The emotion, raw (`NaviStats.Mood`): 128 in the normal state. The
    /// other values (Full Synchro, Anger, Tired, Exhausted) are still to be
    /// mapped in play.
    pub mood: u8,
}

/// A player's status flags, the game's `ObjectFlags1` word (bits named in
/// the bn6f disassembly, `include/structs/CollisionData.inc`).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Status(pub u32);

impl Status {
    const NAMES: &'static [(u32, &'static str)] = &[
        (0x0000_0002, "invisible"),
        (0x0000_0400, "flinching"),
        (0x0000_0800, "paralyzed"),
        (0x0000_2000, "blind"),
        (0x0000_4000, "immobilized"),
        (0x0000_8000, "confused"),
        (0x0001_0000, "frozen"),
        (0x0002_0000, "super armor"),
        (0x0004_0000, "undershirt"),
        (0x0020_0000, "anger"),
        (0x8000_0000, "bubbled"),
    ];

    pub fn paralyzed(self) -> bool {
        self.0 & 0x0800 != 0
    }
    pub fn flinching(self) -> bool {
        self.0 & 0x0400 != 0
    }
    pub fn anger(self) -> bool {
        self.0 & 0x0020_0000 != 0
    }
    pub fn super_armor(self) -> bool {
        self.0 & 0x0002_0000 != 0
    }

    /// The set flags' names, for display.
    pub fn names(self) -> Vec<&'static str> {
        Self::NAMES
            .iter()
            .filter(|(bit, _)| self.0 & bit != 0)
            .map(|(_, n)| *n)
            .collect()
    }
}

/// A barrier or aura on a player. Traps (the Anti- chips, ElemTrap) are
/// deliberately not read: they're hidden from the opponent, and a bot
/// knows its own from having set them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Barrier {
    pub kind: BarrierKind,
    /// HP left (a BubbleWrap reads 1; it pops on any hit and returns).
    pub hp: u8,
    /// For an aura, the damage an attack must reach to break it.
    pub threshold: u8,
}

/// Which barrier; values from the `bn6_explore` duel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BarrierKind {
    Barrier,
    Barrier100,
    Barrier200,
    BubbleWrap,
    LifeAura,
    Unknown(u8),
}

impl BarrierKind {
    pub fn from_raw(raw: u8) -> Self {
        match raw {
            0x01 => BarrierKind::Barrier,
            0x05 => BarrierKind::Barrier100,
            0x07 => BarrierKind::Barrier200,
            0x08 => BarrierKind::BubbleWrap,
            0x09 => BarrierKind::LifeAura,
            other => BarrierKind::Unknown(other),
        }
    }
}

/// A player's form, the game's `TF_` transformation enum (named in the
/// bn6f disassembly, `constants/constants.inc`). Crosses are numbered 1-10:
/// Heat, Elec, Slash, Erase, Charge (Gregar), Spout, Tomahawk, Tengu,
/// Ground, Dust (Falzar).
///
/// A Cross Beast is a Cross and Beast Out together. It takes two chip
/// selects, in either order, and while it lasts each later chip select
/// may switch the Cross, until Beast Out runs out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    Normal,
    Cross(u8),
    BeastOut,
    CrossBeast(u8),
    /// After Beast Out runs out (23 on Gregar, confirmed in play; 24 on
    /// Falzar).
    BeastOver,
    /// A value not yet mapped (Falzar's crosses and Beast Over, mods).
    Unknown(u8),
}

impl Form {
    /// Decode the game's form byte: 0 (or 255) normal, 1-10 a Cross, 11-12
    /// Beast Out (Gregar, Falzar), 12 + n Cross Beast with Cross n, 23-24
    /// Beast Over.
    pub fn from_raw(raw: u8) -> Self {
        match raw {
            0 | 0xff => Form::Normal,
            1..=10 => Form::Cross(raw),
            11 | 12 => Form::BeastOut,
            13..=22 => Form::CrossBeast(raw - 12),
            23 | 24 => Form::BeastOver,
            _ => Form::Unknown(raw),
        }
    }
}

/// The buster's charge.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Charge {
    None,
    /// B held; the number of ticks it has been held (caps at 90).
    Charging(u8),
    /// Fully charged: releasing B fires a charge shot.
    Full,
}

impl tango_match::telemetry::GameDetail for Bn6Obs {
    fn summary(&self) -> String {
        let mut lines = vec![format!("gauge {}/{CUSTOM_GAUGE_FULL}", self.custom_gauge)];
        if !self.queue.is_empty() {
            lines[0] += &format!(" · queued {:?}", self.queue);
        }
        for (p, u) in self.units.iter().enumerate() {
            let me = if p == self.player { " (self)" } else { "" };
            let mut extra = String::new();
            if let Some(b) = u.barrier {
                extra += &format!(" · {:?} {}", b.kind, b.hp);
            }
            if u.invisible_ticks > 0 {
                extra += &format!(" · invisible {}", u.invisible_ticks);
            }
            if u.mood != 128 {
                extra += &format!(" · mood {}", u.mood);
            }
            let flags = u.status.names();
            if !flags.is_empty() {
                extra += &format!(" · [{}]", flags.join(", "));
            }
            lines.push(format!(
                "P{}{me}: {} · beast {} left · {}{extra}",
                p + 1,
                u.form,
                u.beast_turns_left,
                u.charge
            ));
        }
        let mark = |p: &Panel| match p.kind {
            PanelKind::Broken => 'x',
            PanelKind::Normal => '.',
            PanelKind::Cracked => '%',
            PanelKind::Poison => 'P',
            PanelKind::Holy => 'H',
            PanelKind::Grass => 'G',
            PanelKind::Ice => 'I',
            PanelKind::GoingRoad => '>',
            PanelKind::ComingRoad => '<',
            PanelKind::Unknown(_) => '?',
        };
        for row in &self.panels {
            let (a, b): (String, String) = (row[..3].iter().map(mark).collect(), row[3..].iter().map(mark).collect());
            lines.push(format!("field {a}|{b}"));
        }
        if !self.obstacles.is_empty() {
            let o: Vec<String> = self
                .obstacles
                .iter()
                .map(|o| format!("{:?}@{:?} {}/{}", o.kind, o.tile, o.hp, o.max_hp))
                .collect();
            lines.push(format!("objects {}", o.join(", ")));
        }
        if let Some(cs) = &self.chip_select {
            let form = match cs.form_picked {
                Some(FormPick::Cross) => " · Cross picked",
                Some(FormPick::BeastOut) => " · Beast Out picked",
                None => "",
            };
            lines.push(format!("chip select: {:?} · picks {:?}{form}", cs.cursor, cs.picked));
            let hand: Vec<String> = cs
                .hand
                .iter()
                .map(|c| c.map_or("--".to_string(), |c| c.to_string()))
                .collect();
            lines.push(format!("hand {}", hand.join(" ")));
        }
        lines.join("\n")
    }
}

impl std::fmt::Display for HandChip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let code = match self.code {
            0..=25 => (b'A' + self.code) as char,
            26 => '*',
            _ => '?',
        };
        write!(f, "{}{code}", self.id)
    }
}

impl std::fmt::Display for Form {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Form::Normal => write!(f, "normal"),
            Form::Cross(n) => write!(f, "Cross #{n}"),
            Form::BeastOut => write!(f, "Beast Out"),
            Form::CrossBeast(n) => write!(f, "Cross Beast #{n}"),
            Form::BeastOver => write!(f, "Beast Over"),
            Form::Unknown(v) => write!(f, "form {v}?"),
        }
    }
}

impl std::fmt::Display for Charge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Charge::None => write!(f, "no charge"),
            Charge::Charging(t) => write!(f, "charging {t}"),
            Charge::Full => write!(f, "charged"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hand_entries_decode_as_seen_in_ram() {
        // Vulcan1 *, the first chip of the template save's opening hand.
        assert_eq!(HandChip::from_raw(0x3405), Some(HandChip { id: 5, code: 26 }));
        assert_eq!(HandChip::from_raw(0xffff), None);
    }

    #[test]
    fn panels_and_obstacles_decode_as_seen_in_ram() {
        assert_eq!(PanelKind::from_raw(0x07), PanelKind::Ice);
        assert_eq!(PanelKind::from_raw(0x0b), PanelKind::GoingRoad);
        assert_eq!(PanelKind::from_raw(0x42), PanelKind::Unknown(0x42));
        assert_eq!(ObstacleKind::from_raw(0xd0), ObstacleKind::RockCube);
        assert_eq!(ObstacleKind::from_raw(0x99), ObstacleKind::Unknown(0x99));
    }

    #[test]
    fn forms_decode_as_seen_in_ram() {
        assert_eq!(Form::from_raw(0), Form::Normal);
        assert_eq!(Form::from_raw(255), Form::Normal);
        assert_eq!(Form::from_raw(4), Form::Cross(4));
        assert_eq!(Form::from_raw(11), Form::BeastOut);
        // Heat then Beast Out, and Beast Out then Elec.
        assert_eq!(Form::from_raw(13), Form::CrossBeast(1));
        assert_eq!(Form::from_raw(14), Form::CrossBeast(2));
        assert_eq!(Form::from_raw(23), Form::BeastOver);
        assert_eq!(Form::from_raw(12), Form::BeastOut);
        assert_eq!(Form::from_raw(22), Form::CrossBeast(10));
        assert_eq!(Form::from_raw(25), Form::Unknown(25));
    }
}
