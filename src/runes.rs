use {
  self::{flag::Flag, tag::Tag},
  super::*,
};

pub use {edict::Edict, rune::Rune, rune_id::RuneId, runestone::Runestone};

pub(crate) use {etching::Etching, mint::Mint, pile::Pile, spaced_rune::SpacedRune};

pub const MAX_DIVISIBILITY: u8 = 38;
pub(crate) const MAX_LIMIT: u128 = 1 << 64;
const RESERVED: u128 = 6402364363415443603228541259936211926;

mod edict;
mod etching;
mod flag;
mod mint;
mod pile;
mod rune;
mod rune_id;
mod runestone;
mod spaced_rune;
mod tag;
pub mod varint;

type Result<T, E = Error> = std::result::Result<T, E>;
