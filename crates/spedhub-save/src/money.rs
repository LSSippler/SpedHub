//! Kontostand in `game.sii` (`bank.money_account`) und `info.sii` (`info_money_account`).

use crate::error::{Error, Result};
use crate::layout::money as m;
use crate::sii::Save;
use crate::value::parse_int;

/// Kontostand aus `game.sii`.
pub fn money(game: &Save) -> Result<i64> {
    read(game, m::BANK_KIND, m::BANK_FIELD)
}

pub fn set_money(game: &mut Save, amount: i64) -> Result<()> {
    write(game, m::BANK_KIND, m::BANK_FIELD, amount)
}

/// Kontostand für den Ladebildschirm aus `info.sii`.
pub fn info_money(info: &Save) -> Result<i64> {
    read(info, m::INFO_KIND, m::INFO_FIELD)
}

pub fn set_info_money(info: &mut Save, amount: i64) -> Result<()> {
    write(info, m::INFO_KIND, m::INFO_FIELD, amount)
}

fn read(save: &Save, kind: &str, field: &str) -> Result<i64> {
    let unit = save.single(kind)?;
    let raw = save.get(&unit, field)?;
    parse_int(raw).ok_or_else(|| Error::invalid_value(&unit, field, raw))
}

fn write(save: &mut Save, kind: &str, field: &str, amount: i64) -> Result<()> {
    let unit = save.single(kind)?;
    // Erst lesen: so steht fest, dass das Feld da ist und eine Zahl enthält.
    read(save, kind, field)?;
    save.set(&unit, field, &amount.to_string())
}
