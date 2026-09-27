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
}

/// A player's form. Crosses are numbered by their place in the Cross
/// list, from 1 (on Gregar: Heat, Elec, Slash, Erase, Charge).
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
    /// After Beast Out runs out. 23 on Gregar (reported in play); Falzar's
    /// value is unchecked.
    BeastOver,
    /// A value not yet mapped (Falzar's crosses and Beast Over, mods).
    Unknown(u8),
}

impl Form {
    /// Decode the game's form byte: 0 or 255 normal, 1-5 a Cross, 11 Beast
    /// Out, 12 + n Cross Beast with Cross n, 23 Beast Over (Gregar).
    pub fn from_raw(raw: u8) -> Self {
        match raw {
            0 | 0xff => Form::Normal,
            1..=5 => Form::Cross(raw),
            11 => Form::BeastOut,
            13..=17 => Form::CrossBeast(raw - 12),
            23 => Form::BeastOver,
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
            lines.push(format!(
                "P{}{me}: {} · beast {} left · {}",
                p + 1,
                u.form,
                u.beast_turns_left,
                u.charge
            ));
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
    fn forms_decode_as_seen_in_ram() {
        assert_eq!(Form::from_raw(0), Form::Normal);
        assert_eq!(Form::from_raw(255), Form::Normal);
        assert_eq!(Form::from_raw(4), Form::Cross(4));
        assert_eq!(Form::from_raw(11), Form::BeastOut);
        // Heat then Beast Out, and Beast Out then Elec.
        assert_eq!(Form::from_raw(13), Form::CrossBeast(1));
        assert_eq!(Form::from_raw(14), Form::CrossBeast(2));
        assert_eq!(Form::from_raw(23), Form::BeastOver);
        assert_eq!(Form::from_raw(12), Form::Unknown(12));
    }
}
