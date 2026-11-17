//! Schaden eines Lkw aus dem gespeicherten Zustand.
//!
//! Der Zustand liegt als JSON in `trucks.state`, mit den Feldnamen von
//! `spedhub_save::TruckState`. Verschleiß liegt zwischen 0 und 1. Der Schaden in Prozent ist
//! die gewichtete Summe der reparierbaren Verschleißwerte, siehe [`WEIGHTS`]. Der dauerhafte
//! Verschleiß (`*_unfixable`) zählt nicht als Schaden. Er wird mit derselben Gewichtung getrennt
//! berechnet und nur angezeigt.
//!
//! Fehlende Felder und Werte, die keine Zahl sind, zählen als 0. Werte außerhalb von 0 bis 1
//! werden auf diesen Bereich begrenzt. Die Räder haben einen Wert je Achse, es zählt der
//! Mittelwert. Das Ergebnis wird auf eine Nachkommastelle gerundet.

use serde_json::{Map, Value, json};

/// Gewichtung der Bauteile in Prozentpunkten: (Verschleiß, dauerhafter Verschleiß, Gewicht).
/// Die Gewichte ergeben zusammen 100, ein ganz verschlissener Lkw hat also 100 Prozent Schaden.
/// Diese Tabelle steht auch in `server/README.md` und `docs/architektur.md`.
pub const WEIGHTS: [(&str, &str, f64); 5] = [
    ("engine_wear", "engine_wear_unfixable", 30.0),
    ("transmission_wear", "transmission_wear_unfixable", 20.0),
    ("cabin_wear", "cabin_wear_unfixable", 10.0),
    ("chassis_wear", "chassis_wear_unfixable", 25.0),
    ("wheels_wear", "wheels_wear_unfixable", 15.0),
];

/// Schaden eines Lkw in Prozent, je auf eine Nachkommastelle gerundet.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Damage {
    /// Reparierbarer Schaden. Danach richten sich Werkstatt, Reparaturkosten und Haftung.
    pub fixable: f64,
    /// Dauerhafter Verschleiß, nur zur Anzeige
    pub unfixable: f64,
}

/// Rundet auf eine Nachkommastelle.
pub fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Ein Verschleißwert: eine Zahl oder, bei den Rädern, eine Liste mit einem Wert je Achse.
/// Fehlt er oder ist er keine Zahl, zählt er als 0.
fn wear(state: &Map<String, Value>, key: &str) -> f64 {
    let one = |v: &Value| v.as_f64().filter(|f| f.is_finite()).unwrap_or(0.0);
    let value = match state.get(key) {
        None => 0.0,
        Some(Value::Array(list)) if list.is_empty() => 0.0,
        Some(Value::Array(list)) => list.iter().map(one).sum::<f64>() / list.len() as f64,
        Some(v) => one(v),
    };
    value.clamp(0.0, 1.0)
}

fn weighted(state: &Map<String, Value>, unfixable: bool) -> f64 {
    let sum: f64 = WEIGHTS
        .iter()
        .map(|&(fix, perm, weight)| weight * wear(state, if unfixable { perm } else { fix }))
        .sum();
    round1(sum)
}

/// Schaden aus einem Zustand. Ein Zustand, der kein JSON-Objekt ist, hat keinen Schaden.
pub fn damage(state: &Value) -> Damage {
    match state.as_object() {
        Some(s) => Damage {
            fixable: weighted(s, false),
            unfixable: weighted(s, true),
        },
        None => Damage {
            fixable: 0.0,
            unfixable: 0.0,
        },
    }
}

/// Schaden aus dem Text in `trucks.state`. Unlesbares JSON hat keinen Schaden.
pub fn damage_of(state: &str) -> Damage {
    damage(&serde_json::from_str(state).unwrap_or(Value::Null))
}

/// Was eine Reparatur kostet: Schaden mal Satz je Prozentpunkt, auf ganze Euro gerundet.
pub fn repair_cost(damage: f64, rate: i64) -> i64 {
    (damage * rate as f64).round() as i64
}

/// Schaden, der während einer Leihe dazugekommen ist, in Prozentpunkten. Nie unter 0: Wer
/// einen Lkw besser zurückbringt, als er ihn bekommen hat, bekommt nichts gutgeschrieben.
pub fn new_damage(at_return: f64, at_checkout: f64) -> f64 {
    round1((at_return - at_checkout).max(0.0))
}

/// Was der Fahrer für neuen Schaden zahlt: neuer Schaden mal Satz je Prozentpunkt mal
/// Haftungsanteil in Prozent, auf ganze Euro gerundet.
pub fn liability(new_damage: f64, rate: i64, percent: i64) -> i64 {
    (new_damage * rate as f64 * percent as f64 / 100.0).round() as i64
}

/// Der Zustand nach der Werkstatt: alle reparierbaren Verschleißwerte auf 0, bei den Rädern
/// jede Achse. Der dauerhafte Verschleiß und alles andere bleiben. Felder, die fehlen, zählen
/// schon als 0 und werden nicht angelegt, damit der Zustand so vollständig bleibt, wie er war.
pub fn repaired(state: &Value) -> Value {
    let mut out = state.clone();
    if let Some(s) = out.as_object_mut() {
        for (key, _, _) in WEIGHTS {
            match s.get_mut(key) {
                None => {}
                Some(Value::Array(list)) => list.iter_mut().for_each(|v| *v = json!(0.0)),
                Some(v) => *v = json!(0.0),
            }
        }
    }
    out
}

/// Prozent für Notizen im Kassenbuch: eine Nachkommastelle mit Komma, ohne ",0".
pub fn percent_text(p: f64) -> String {
    let p = round1(p);
    if p.fract() == 0.0 {
        format!("{}", p as i64)
    } else {
        format!("{p:.1}").replace('.', ",")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gewichte_ergeben_100() {
        let sum: f64 = WEIGHTS.iter().map(|w| w.2).sum();
        assert_eq!(sum, 100.0);
    }

    #[test]
    fn schaden_aus_bekannten_werten() {
        let state = json!({
            "odometer": 1000, "fuel_relative": 0.5,
            "engine_wear": 0.1, "transmission_wear": 0.2, "cabin_wear": 0.3, "chassis_wear": 0.4,
            "wheels_wear": [0.1, 0.3],
            "engine_wear_unfixable": 0.1, "transmission_wear_unfixable": 0.0,
            "cabin_wear_unfixable": 0.0, "chassis_wear_unfixable": 0.0,
            "wheels_wear_unfixable": [0.2, 0.0]
        });
        // 30*0,1 + 20*0,2 + 10*0,3 + 25*0,4 + 15*0,2 = 3 + 4 + 3 + 10 + 3
        // dauerhaft: 30*0,1 + 15*0,1 = 4,5
        assert_eq!(
            damage(&state),
            Damage {
                fixable: 23.0,
                unfixable: 4.5
            }
        );
    }

    #[test]
    fn ganz_verschlissen_ist_100_prozent() {
        let state = json!({
            "engine_wear": 1.0, "transmission_wear": 1.0, "cabin_wear": 1.0, "chassis_wear": 1.0,
            "wheels_wear": [1.0, 1.0, 1.0]
        });
        assert_eq!(damage(&state).fixable, 100.0);
        assert_eq!(damage(&state).unfixable, 0.0);
    }

    #[test]
    fn fehlende_und_kaputte_felder_zaehlen_als_0() {
        assert_eq!(damage(&json!({})).fixable, 0.0);
        assert_eq!(damage(&json!(null)).fixable, 0.0);
        assert_eq!(damage(&json!([1, 2])).fixable, 0.0);
        assert_eq!(damage_of("kein json").fixable, 0.0);
        assert_eq!(damage_of("{}").unfixable, 0.0);
        // Nur der Motor ist bekannt
        assert_eq!(damage(&json!({ "engine_wear": 0.5 })).fixable, 15.0);
        // Text statt Zahl, leere Räderliste
        assert_eq!(
            damage(&json!({ "engine_wear": "viel", "wheels_wear": [] })).fixable,
            0.0
        );
        // Ein einzelner Wert für die Räder statt einer Liste
        assert_eq!(damage(&json!({ "wheels_wear": 0.2 })).fixable, 3.0);
        // Über 1 und unter 0 wird begrenzt
        assert_eq!(
            damage(&json!({ "engine_wear": 3.0, "cabin_wear": -1.0 })).fixable,
            30.0
        );
    }

    #[test]
    fn rundung_auf_eine_nachkommastelle() {
        // 30 * 0,01234 = 0,3702
        assert_eq!(damage(&json!({ "engine_wear": 0.01234 })).fixable, 0.4);
        // 25 + 10 * 0,01 = 25,1
        assert_eq!(
            damage(&json!({ "chassis_wear": 1.0, "cabin_wear": 0.01 })).fixable,
            25.1
        );
        // Werte als f32 aus dem Spielstand, etwa 0.0399999991
        assert_eq!(
            damage(&json!({ "chassis_wear": 0.04_f32 as f64 })).fixable,
            1.0
        );
    }

    #[test]
    fn kosten_und_haftung() {
        assert_eq!(repair_cost(12.0, 150), 1800);
        assert_eq!(repair_cost(12.3, 150), 1845);
        assert_eq!(repair_cost(0.1, 5), 1);
        assert_eq!(repair_cost(12.0, 0), 0);
        assert_eq!(new_damage(16.0, 10.0), 6.0);
        assert_eq!(new_damage(8.0, 10.0), 0.0);
        assert_eq!(new_damage(10.3, 10.1), 0.2);
        assert_eq!(liability(6.0, 150, 50), 450);
        assert_eq!(liability(6.0, 150, 0), 0);
        assert_eq!(liability(0.3, 150, 10), 5);
    }

    #[test]
    fn reparatur_setzt_nur_reparierbares_auf_0() {
        let state = json!({
            "odometer": 1000, "fuel_relative": 0.5,
            "engine_wear": 0.1, "transmission_wear": 0.2, "cabin_wear": 0.3, "chassis_wear": 0.4,
            "wheels_wear": [0.1, 0.3],
            "engine_wear_unfixable": 0.1, "wheels_wear_unfixable": [0.2, 0.0]
        });
        let r = repaired(&state);
        assert_eq!(
            r,
            json!({
                "odometer": 1000, "fuel_relative": 0.5,
                "engine_wear": 0.0, "transmission_wear": 0.0, "cabin_wear": 0.0, "chassis_wear": 0.0,
                "wheels_wear": [0.0, 0.0],
                "engine_wear_unfixable": 0.1, "wheels_wear_unfixable": [0.2, 0.0]
            })
        );
        assert_eq!(damage(&r).fixable, 0.0);
        assert_eq!(damage(&r).unfixable, damage(&state).unfixable);
        // Fehlende Felder werden nicht angelegt
        assert_eq!(
            repaired(&json!({ "engine_wear": 0.5 })),
            json!({ "engine_wear": 0.0 })
        );
        assert_eq!(repaired(&json!(null)), json!(null));
    }

    #[test]
    fn prozent_fuer_notizen() {
        assert_eq!(percent_text(12.0), "12");
        assert_eq!(percent_text(7.5), "7,5");
        assert_eq!(percent_text(0.04), "0");
        assert_eq!(percent_text(25.1), "25,1");
    }
}
