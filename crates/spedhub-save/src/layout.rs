//! Alle Units und Felder, die SpedHub im Spielstand liest oder schreibt, an einer Stelle.
//!
//! Stand ETS2 1.62, Save-Version 105, ohne Mods. Die Liste ist aus echten Spielständen
//! abgelesen, aber noch nicht im Spiel bestätigt (siehe `docs/save-format.md`). Wenn sich
//! beim Test im Spiel etwas anderes herausstellt, wird es hier geändert und nirgends sonst.

/// `player`: der Spieler mit seinen Lkw, Fahrern und dem aktuellen Auftrag.
pub mod player {
    pub const KIND: &str = "player";
    pub const HQ_CITY: &str = "hq_city";
    /// Eigene Lkw als Verweise auf `vehicle`-Units
    pub const TRUCKS: &str = "trucks";
    /// Parallel zu `trucks`, je ein `profit_log`
    pub const TRUCK_PROFIT_LOGS: &str = "truck_profit_logs";
    /// Verweis auf `player_vehicles`: was der Spieler gerade fährt
    pub const ASSIGNED_VEHICLES: &str = "assigned_vehicles";
    /// Liste von `player_vehicles`. Der Eintrag [`MY_TRUCK_INDEX`] ist der eigene Lkw.
    pub const MY_VEHICLES: &str = "my_vehicles";
    pub const MY_TRUCK_INDEX: usize = 0;
    pub const MY_VEHICLES_MODE: &str = "my_vehicles_mode";
    pub const MY_VEHICLES_MODE_TRUCK: &str = "truck";
    pub const MY_VEHICLES_MODE_NONE: &str = "none";
    /// Verweis auf `player_job` oder `null`
    pub const CURRENT_JOB: &str = "current_job";
    /// Fahrer des Spielers. Eintrag 0 ist der Spieler selbst.
    pub const DRIVERS: &str = "drivers";
}

/// `player_vehicles`: ein Fahrzeug des Spielers mit Standort.
pub mod player_vehicles {
    pub const VEHICLE: &str = "vehicle";
    pub const PLACEMENT: &str = "stored_vehicle_placement";
    pub const CITY: &str = "stored_vehicle_city";
    pub const TRAILER: &str = "trailer";
    pub const TRAILER_PLACEMENTS: &str = "stored_trailer_placements";
    pub const TRAILER_ATTACHED: &str = "stored_trailer_attached";
}

/// `player_job`: der laufende Auftrag.
pub mod player_job {
    pub const KIND: &str = "player_job";
    /// Bei einem Schnellauftrag der Firmen-Lkw, sonst `null`
    pub const COMPANY_TRUCK: &str = "company_truck";
    pub const COMPANY_TRAILER: &str = "company_trailer";
}

/// `garage.<stadt>`: Stellplätze mit parallelen Listen für Lkw und Fahrer.
pub mod garage {
    pub const KIND: &str = "garage";
    pub const NAME_PREFIX: &str = "garage.";
    pub const VEHICLES: &str = "vehicles";
    pub const DRIVERS: &str = "drivers";
}

/// `driver_ai`: angestellte Fahrer. Diese Felder können auf einen Lkw zeigen und werden beim
/// Entfernen dieses Lkw auf `null` gesetzt.
pub mod driver_ai {
    pub const KIND: &str = "driver_ai";
    pub const TRUCK_FIELDS: &[&str] = &["assigned_truck", "adopted_truck"];
}

/// `game_progress`: Modellnamen aller Lkw, die der Spieler besitzt oder besessen hat.
pub mod game_progress {
    pub const KIND: &str = "game_progress";
    pub const OWNED_TRUCKS: &str = "owned_trucks";

    /// Eintrag für ein Modell, zum Beispiel `"vehicle.scania.s_2016"`.
    pub fn owned_truck_entry(model: &str) -> String {
        format!("\"vehicle.{model}\"")
    }
}

/// `profit_log`: Statistik je Lkw. Beim Einsetzen wird ein leeres Log angelegt.
pub mod profit_log {
    pub const KIND: &str = "profit_log";
    pub const EMPTY: &[(&str, &str)] = &[
        ("stats_data", "0"),
        ("acc_distance_free", "0"),
        ("acc_distance_on_job", "0"),
        ("history_age", "nil"),
    ];
}

/// Präfix der Städte in `stored_vehicle_city`.
pub const CITY_PREFIX: &str = "city.";

/// `vehicle`: der Lkw selbst. Zubehör-Units haben Typen, die auf `_accessory` enden.
pub mod vehicle {
    pub const KIND: &str = "vehicle";
    pub const ACCESSORY_KIND_SUFFIX: &str = "_accessory";
    pub const ACCESSORIES: &str = "accessories";
    pub const DATA_PATH: &str = "data_path";
    pub const LICENSE_PLATE: &str = "license_plate";
    /// Aus `data_path` dieser Form ergibt sich das Modell.
    pub const MODEL_PATH_PREFIX: &str = "/def/vehicle/truck/";
    pub const MODEL_PATH_SUFFIX: &str = "/data.sii";

    pub const ODOMETER: &str = "odometer";
    pub const ODOMETER_FLOAT_PART: &str = "odometer_float_part";
    pub const INTEGRITY_ODOMETER: &str = "integrity_odometer";
    pub const INTEGRITY_ODOMETER_FLOAT_PART: &str = "integrity_odometer_float_part";
    pub const FUEL_RELATIVE: &str = "fuel_relative";
    pub const ENGINE_WEAR: &str = "engine_wear";
    pub const TRANSMISSION_WEAR: &str = "transmission_wear";
    pub const CABIN_WEAR: &str = "cabin_wear";
    pub const CHASSIS_WEAR: &str = "chassis_wear";
    pub const ENGINE_WEAR_UNFIXABLE: &str = "engine_wear_unfixable";
    pub const TRANSMISSION_WEAR_UNFIXABLE: &str = "transmission_wear_unfixable";
    pub const CABIN_WEAR_UNFIXABLE: &str = "cabin_wear_unfixable";
    pub const CHASSIS_WEAR_UNFIXABLE: &str = "chassis_wear_unfixable";
    pub const WHEELS_WEAR: &str = "wheels_wear";
    pub const WHEELS_WEAR_UNFIXABLE: &str = "wheels_wear_unfixable";
}

/// Geld: `bank.money_account` in `game.sii`, `save_container.info_money_account` in `info.sii`.
pub mod money {
    pub const BANK_KIND: &str = "bank";
    pub const BANK_FIELD: &str = "money_account";
    pub const INFO_KIND: &str = "save_container";
    pub const INFO_FIELD: &str = "info_money_account";
}

/// `info.sii`: eine Unit `save_container` mit dem Namen des Speicherplatzes für das Menü.
pub mod info {
    pub const KIND: &str = "save_container";
    /// Angezeigter Name, Zeichenkette in Anführungszeichen. Bei Autosaves leer.
    pub const NAME: &str = "name";
    /// Zeitpunkt des Speicherns in Sekunden seit 1970. Vermutlich sortiert das Spiel danach,
    /// bestätigt ist das nicht.
    pub const FILE_TIME: &str = "file_time";
}

/// Neue Unit-Namen sehen aus wie die des Spiels: `_nameless.2ff.<hhhh>.<llll>`, die hinteren
/// Gruppen vierstellig mit führenden Nullen, die letzte Stelle immer `8`. In echten
/// Spielständen ist das bei allen Namen so. Gezählt wird ab `_nameless.2ff.7a11.0008` in
/// Schritten von 0x10, vergebene Namen werden übersprungen. Nach `.fff8` geht es mit der
/// nächsten mittleren Gruppe weiter. Der erste Name ist derselbe, den `tools/transplant.py`
/// für das neue `profit_log` verwendet.
pub const NAME_PREFIX: &str = "_nameless.2ff.";
pub const NAME_MIDDLE_START: u32 = 0x7a11;
pub const NAME_LOW_START: u32 = 0x0008;
pub const NAME_STEP: u32 = 0x10;

pub const NULL: &str = "null";
