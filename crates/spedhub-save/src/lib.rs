//! Spielstände von Euro Truck Simulator 2 lesen und schreiben.
//!
//! - Lesen geht in jedem Format (ScsC, BSII, SiiN), geschrieben wird nur Klartext.
//! - Vor jedem Schreiben läuft die Integritätsprüfung. Schlägt sie fehl, wird nichts geschrieben.
//! - Lkw lassen sich als Paket exportieren, in einen anderen Spielstand einsetzen und entfernen.
//! - Aus `def/economy_data.sii` des Spiels baut [`economy`] die Wirtschafts-Mod.
//!
//! Welche Felder dabei gelesen und geschrieben werden, steht gesammelt in [`layout`].
#![cfg_attr(
    not(test),
    warn(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod check;
pub mod economy;
pub mod error;
pub mod info;
pub mod layout;
pub mod money;
pub mod plate;
pub mod sii;
pub mod state;
pub mod truck;
pub mod value;
pub mod write;
pub mod zip;

pub use check::{Problem, Report, check};
pub use economy::{
    Candidate, Change, EconomyEdit, Factor, ModOptions, ScaleOptions, build_mod, scale_economy,
    write_mod,
};
pub use error::{Error, Result};
pub use info::{info_experience, info_name, set_info_file_time, set_info_name};
pub use money::{info_money, money, set_info_money, set_money};
pub use plate::{custom_plate, plate_country, plate_text};
pub use sii::{Field, Format, Save, Unit, decode};
pub use state::TruckState;
pub use truck::{
    DEFAULT_MIN_MATCH, InsertOptions, InsertReport, PackageMatch, PackageUnit, RemoveReport,
    TruckInfo, TruckPackage, export_truck, find_package_truck, insert_truck, license_plate,
    list_trucks, model_key, remove_truck, set_license_plate, set_truck_state, truck_state,
};
pub use write::{Written, write_save, write_saves};
