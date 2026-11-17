//! Level im Spiel aus Erfahrungspunkten und die Lohnstufe eines Fahrers.
//!
//! ETS2 speichert nur die Erfahrungspunkte (`info_players_experience` in `info.sii`,
//! `experience_points` in `game.sii`). Das Level rechnet das Spiel selbst aus: Für die ersten
//! 30 Level braucht es je Level die Punkte aus [`LEVEL_STEPS`], danach je Level so viele wie
//! für das 30. Quelle und Abgleich stehen in `docs/save-format.md` unter "Erfahrung und Level".
use rusqlite::{Connection, params};

/// Punkte je Level-Aufstieg in ETS2. Level 1 bei 200, Level 2 bei 700, Level 3 bei 1400 Punkten.
pub const LEVEL_STEPS: [i64; 30] = [
    200, 500, 700, 900, 1000, 1100, 1300, 1600, 1700, 2100, 2300, 2600, 2700, 2900, 3000, 3100,
    3400, 3700, 4000, 4300, 4600, 4700, 4900, 5200, 5700, 5900, 6000, 6200, 6600, 6800,
];

/// Level im Spiel für so viele Erfahrungspunkte. Ein neues Profil hat Level 0.
pub fn game_level(xp: i64) -> i64 {
    let mut threshold = 0;
    let mut level = 0;
    for step in LEVEL_STEPS {
        threshold += step;
        if xp < threshold {
            return level;
        }
        level += 1;
    }
    let last = LEVEL_STEPS[LEVEL_STEPS.len() - 1];
    level + (xp - threshold) / last
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WageLevel {
    pub position: i64,
    pub min_level: i64,
    pub min_km: i64,
    pub cents_per_km: i64,
}

pub fn wage_levels(conn: &Connection, company_id: i64) -> rusqlite::Result<Vec<WageLevel>> {
    conn.prepare(
        "SELECT position, min_level, min_km, cents_per_km FROM wage_levels
         WHERE company_id = ?1 ORDER BY position",
    )?
    .query_map([company_id], |r| {
        Ok(WageLevel {
            position: r.get(0)?,
            min_level: r.get(1)?,
            min_km: r.get(2)?,
            cents_per_km: r.get(3)?,
        })
    })?
    .collect()
}

/// Die höchste Stufe, deren beide Bedingungen erfüllt sind. `None`, wenn keine passt.
pub fn stage_for(levels: &[WageLevel], game_level: i64, company_km: i64) -> Option<WageLevel> {
    levels
        .iter()
        .rev()
        .find(|w| game_level >= w.min_level && company_km >= w.min_km)
        .copied()
}

/// Was der Server über einen Fahrer weiß, um seine Stufe zu bestimmen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriverStanding {
    /// Erfahrungspunkte aus dem letzten Abgleich, `None` ohne Abgleich
    pub xp: Option<i64>,
    pub game_level: i64,
    /// Kilometer aus abgelieferten Aufträgen für diese Firma
    pub company_km: i64,
    pub stage: Option<WageLevel>,
}

impl DriverStanding {
    pub fn cents_per_km(&self) -> i64 {
        self.stage.map(|s| s.cents_per_km).unwrap_or(0)
    }
}

pub fn company_km(conn: &Connection, company_id: i64, user_id: i64) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COALESCE(SUM(distance_km), 0) FROM ledger
         WHERE company_id = ?1 AND user_id = ?2 AND kind = 'auftrag' AND account = 'firma'",
        params![company_id, user_id],
        |r| r.get(0),
    )
}

pub fn last_xp(conn: &Connection, company_id: i64, user_id: i64) -> rusqlite::Result<Option<i64>> {
    use rusqlite::OptionalExtension;
    conn.query_row(
        "SELECT xp FROM sync_points WHERE company_id = ?1 AND user_id = ?2 AND xp IS NOT NULL
             ORDER BY id DESC LIMIT 1",
        params![company_id, user_id],
        |r| r.get(0),
    )
    .optional()
}

pub fn standing(
    conn: &Connection,
    levels: &[WageLevel],
    company_id: i64,
    user_id: i64,
) -> rusqlite::Result<DriverStanding> {
    let xp = last_xp(conn, company_id, user_id)?;
    let game_level = xp.map(game_level).unwrap_or(0);
    let company_km = company_km(conn, company_id, user_id)?;
    Ok(DriverStanding {
        xp,
        game_level,
        company_km,
        stage: stage_for(levels, game_level, company_km),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_aus_erfahrung() {
        assert_eq!(game_level(0), 0);
        assert_eq!(game_level(199), 0);
        assert_eq!(game_level(200), 1);
        assert_eq!(game_level(699), 1);
        assert_eq!(game_level(700), 2);
        assert_eq!(game_level(1400), 3);
        assert_eq!(game_level(3300), 5);
        assert_eq!(game_level(9000), 9);
        let thirty: i64 = LEVEL_STEPS.iter().sum();
        assert_eq!(thirty, 99_700);
        assert_eq!(game_level(thirty - 1), 29);
        assert_eq!(game_level(thirty), 30);
        // Danach 6800 Punkte je Level
        assert_eq!(game_level(thirty + 6799), 30);
        assert_eq!(game_level(thirty + 6800), 31);
        assert_eq!(game_level(thirty + 10 * 6800), 40);
    }

    #[test]
    fn hoechste_passende_stufe() {
        let l = |position, min_level, min_km, cents_per_km| WageLevel {
            position,
            min_level,
            min_km,
            cents_per_km,
        };
        let levels = [l(1, 0, 0, 40), l(2, 5, 5000, 50), l(3, 10, 20000, 60)];
        assert_eq!(stage_for(&levels, 0, 0).unwrap().position, 1);
        // Viele Kilometer, aber Level zu klein
        assert_eq!(stage_for(&levels, 4, 50_000).unwrap().position, 1);
        assert_eq!(stage_for(&levels, 5, 5000).unwrap().position, 2);
        assert_eq!(stage_for(&levels, 30, 20_000).unwrap().position, 3);
        assert_eq!(stage_for(&[l(1, 3, 0, 40)], 0, 0), None);
    }
}
