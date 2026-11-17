//! Integritätsprüfung nach `docs/save-format.md`, Abschnitt "Was immer gelten muss".
//!
//! 1. Jeder Unit-Name kommt genau einmal vor.
//! 2. Jeder `_nameless`-Verweis zeigt auf eine vorhandene Unit.
//! 3. Jede Unit ist von der ersten Unit aus über Verweise erreichbar.
//!
//! Dazu kommt eine eigene Prüfung: Listen sind vollständig, also Zählzeile direkt vor den
//! Einträgen, und die Einträge laufen lückenlos von 0 bis N-1. Das gilt in allen
//! Testdaten und schützt vor Fehlern beim Bearbeiten.

use crate::sii::{Save, array_base, is_nameless};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    NoUnits,
    DuplicateName(String),
    Dangling {
        unit: String,
        field: String,
        target: String,
    },
    Orphan {
        unit: String,
        kind: String,
    },
    BrokenList {
        unit: String,
        field: String,
    },
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Problem::NoUnits => write!(f, "der Spielstand enthält keine Unit"),
            Problem::DuplicateName(n) => write!(f, "den Unit-Namen {n} gibt es mehrfach"),
            Problem::Dangling {
                unit,
                field,
                target,
            } => write!(
                f,
                "{unit}, Feld {field} verweist auf {target}, das es nicht gibt"
            ),
            Problem::Orphan { unit, kind } => write!(
                f,
                "die Unit {unit} ({kind}) ist von der ersten Unit aus nicht erreichbar"
            ),
            Problem::BrokenList { unit, field } => {
                write!(f, "die Liste {field} in {unit} ist unvollständig")
            }
        }
    }
}

/// Ergebnis der Prüfung.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub units: usize,
    pub problems: Vec<Problem>,
}

impl Report {
    pub fn is_ok(&self) -> bool {
        self.problems.is_empty()
    }
}

pub fn check(save: &Save) -> Report {
    let units = save.units();
    let mut problems = Vec::new();
    let Some(first) = units.first() else {
        return Report {
            units: 0,
            problems: vec![Problem::NoUnits],
        };
    };

    // 1. Eindeutige Namen
    let mut seen = HashSet::new();
    for u in units {
        if !seen.insert(u.name.as_str()) {
            problems.push(Problem::DuplicateName(u.name.clone()));
        }
    }

    // 2. Kein _nameless-Verweis ins Leere
    for u in units {
        for f in &u.fields {
            let v = f.value.trim();
            if is_nameless(v) && !save.contains(v) {
                problems.push(Problem::Dangling {
                    unit: u.name.clone(),
                    field: f.key.clone(),
                    target: v.into(),
                });
            }
        }
    }

    // 3. Alles von der ersten Unit aus erreichbar
    let mut reached: HashSet<&str> = HashSet::new();
    let mut queue = VecDeque::from([first]);
    reached.insert(first.name.as_str());
    while let Some(u) = queue.pop_front() {
        for f in &u.fields {
            if let Some(target) = save.resolves(&f.value)
                && reached.insert(target)
                && let Some(t) = save.unit(target)
            {
                queue.push_back(t);
            }
        }
    }
    for u in units {
        if !reached.contains(u.name.as_str()) {
            problems.push(Problem::Orphan {
                unit: u.name.clone(),
                kind: u.kind.clone(),
            });
        }
    }

    // 4. Listen vollständig
    for u in units {
        let mut lists: HashMap<&str, Vec<(usize, usize)>> = HashMap::new();
        for (pos, f) in u.fields.iter().enumerate() {
            if let Some((base, index)) = array_base(&f.key) {
                lists.entry(base).or_default().push((pos, index));
            }
        }
        let mut broken: Vec<&str> = lists
            .into_iter()
            .filter(|(base, entries)| !list_is_complete(u, base, entries))
            .map(|(base, _)| base)
            .collect();
        broken.sort_unstable();
        problems.extend(broken.into_iter().map(|b| Problem::BrokenList {
            unit: u.name.clone(),
            field: b.into(),
        }));
    }

    Report {
        units: units.len(),
        problems,
    }
}

fn list_is_complete(unit: &crate::sii::Unit, base: &str, entries: &[(usize, usize)]) -> bool {
    let Some(count_pos) = unit.fields.iter().position(|f| f.key == base) else {
        return false;
    };
    let Some(count) = unit
        .fields
        .get(count_pos)
        .and_then(|f| f.value.parse::<usize>().ok())
    else {
        return false;
    };
    count == entries.len()
        && entries
            .iter()
            .enumerate()
            .all(|(n, &(pos, index))| index == n && pos == count_pos + 1 + n)
}
