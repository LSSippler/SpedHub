//! Uhrzeit des Servers und Kalendertage in Europe/Berlin.
//!
//! Die Uhr lässt sich in Tests stellen, damit die tägliche Abrechnung ohne Warten prüfbar ist.
//! Die Zeitzone ist fest Europe/Berlin: MEZ (UTC+1), Sommerzeit MESZ (UTC+2) vom letzten
//! Sonntag im März 01:00 UTC bis zum letzten Sonntag im Oktober 01:00 UTC. Das sind die
//! EU-Regeln seit 1996, eine eigene Zeitzonen-Bibliothek braucht es dafür nicht.
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

/// Uhr des Servers. `system()` liest die echte Zeit, `fixed()` eine gestellte.
#[derive(Clone, Default)]
pub struct Clock(Arc<Mutex<Option<i64>>>);

impl Clock {
    pub fn system() -> Clock {
        Clock::default()
    }

    /// Gestellte Uhr für Tests, Sekunden seit 1970 (UTC).
    pub fn fixed(t: i64) -> Clock {
        Clock(Arc::new(Mutex::new(Some(t))))
    }

    /// Stellt die Uhr. Bei einer Systemuhr wird sie damit zur gestellten.
    pub fn set(&self, t: i64) {
        *self.0.lock().unwrap() = Some(t);
    }

    pub fn advance(&self, secs: i64) {
        let t = self.now();
        self.set(t + secs);
    }

    pub fn now(&self) -> i64 {
        match *self.0.lock().unwrap() {
            Some(t) => t,
            None => system_now(),
        }
    }
}

pub fn system_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

const DAY: i64 = 86_400;

/// Tage seit 1970-01-01 für ein Datum im gregorianischen Kalender (Howard Hinnant).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Datum zu Tagen seit 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (
        if m <= 2 {
            yoe + era * 400 + 1
        } else {
            yoe + era * 400
        },
        m,
        d,
    )
}

/// Letzter Sonntag eines Monats, als Tage seit 1970.
fn last_sunday(y: i64, m: i64, last_day: i64) -> i64 {
    let d = days_from_civil(y, m, last_day);
    // 1970-01-01 war ein Donnerstag, Sonntag ist 0
    d - (d + 4).rem_euclid(7)
}

/// Gilt in Berlin zum Zeitpunkt `t` (UTC) die Sommerzeit?
pub fn is_dst(t: i64) -> bool {
    let (y, _, _) = civil_from_days(t.div_euclid(DAY));
    let start = last_sunday(y, 3, 31) * DAY + 3600;
    let end = last_sunday(y, 10, 31) * DAY + 3600;
    (start..end).contains(&t)
}

/// Abstand zu UTC in Sekunden zum Zeitpunkt `t`.
pub fn offset(t: i64) -> i64 {
    if is_dst(t) { 7200 } else { 3600 }
}

/// Kalendertag in Berlin, als Tage seit 1970.
pub fn local_day(t: i64) -> i64 {
    (t + offset(t)).div_euclid(DAY)
}

/// UTC-Zeitpunkt für eine Uhrzeit in Berlin: Tag `day`, `minute` Minuten nach Mitternacht.
/// In der Stunde, die es bei der Umstellung nicht gibt oder doppelt gibt, kommt eine der
/// beiden möglichen Lösungen heraus.
pub fn local_to_utc(day: i64, minute: i64) -> i64 {
    let local = day * DAY + minute * 60;
    let winter = local - 3600;
    if is_dst(winter) { local - 7200 } else { winter }
}

/// Beginn eines Kalendertags in Berlin (Mitternacht) als UTC-Zeitpunkt.
pub fn day_start(day: i64) -> i64 {
    local_to_utc(day, 0)
}

/// Tag als `2026-11-16`.
pub fn format_day(day: i64) -> String {
    let (y, m, d) = civil_from_days(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Zeitpunkt als `16.11.2026 10:00` in Berlin, für Notizen im Kassenbuch.
pub fn format_local(t: i64) -> String {
    let local = t + offset(t);
    let (y, m, d) = civil_from_days(local.div_euclid(DAY));
    let minute = local.rem_euclid(DAY) / 60;
    format!("{d:02}.{m:02}.{y:04} {}", format_minute(minute))
}

/// `4:00` als `04:00`.
pub fn format_minute(minute: i64) -> String {
    format!("{:02}:{:02}", minute / 60, minute % 60)
}

/// `04:00` oder `4:00` in Minuten nach Mitternacht.
pub fn parse_minute(s: &str) -> Option<i64> {
    let (h, m) = s.trim().split_once(':')?;
    let h: i64 = h.parse().ok()?;
    let m: i64 = m.parse().ok()?;
    if (0..24).contains(&h) && (0..60).contains(&m) && !s.contains(['+', '-']) {
        Some(h * 60 + m)
    } else {
        None
    }
}

/// UTC-Zeitpunkt aus einem Datum und einer Uhrzeit in Berlin, für Tests und Anzeigen.
pub fn berlin(y: i64, mo: i64, d: i64, h: i64, mi: i64) -> i64 {
    local_to_utc(days_from_civil(y, mo, d), h * 60 + mi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kalender_hin_und_zurueck() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2026, 11, 17), 20_774);
        for z in [-1000, 0, 59, 60, 20_774, 30_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z);
        }
        assert_eq!(format_day(20_774), "2026-11-17");
    }

    #[test]
    fn sommerzeit_in_berlin() {
        // 2026: Umstellung am 29. März und 25. Oktober, jeweils 01:00 UTC
        let maerz = days_from_civil(2026, 3, 29) * DAY + 3600;
        assert!(!is_dst(maerz - 1));
        assert!(is_dst(maerz));
        let oktober = days_from_civil(2026, 10, 25) * DAY + 3600;
        assert!(is_dst(oktober - 1));
        assert!(!is_dst(oktober));
        assert_eq!(offset(days_from_civil(2026, 7, 1) * DAY), 7200);
        assert_eq!(offset(days_from_civil(2026, 1, 1) * DAY), 3600);
    }

    #[test]
    fn ortszeit_und_tage() {
        let d = days_from_civil(2026, 11, 16);
        // Winter: Mitternacht in Berlin ist 23:00 UTC am Vortag
        assert_eq!(day_start(d), d * DAY - 3600);
        assert_eq!(local_to_utc(d, 240), d * DAY + 3 * 3600);
        // Sommer: 04:00 MESZ ist 02:00 UTC
        let s = days_from_civil(2026, 7, 1);
        assert_eq!(local_to_utc(s, 240), s * DAY + 2 * 3600);
        // Am Tag der Umstellung im März: Mitternacht noch MEZ, 04:00 schon MESZ
        let m = days_from_civil(2026, 3, 29);
        assert_eq!(day_start(m), m * DAY - 3600);
        assert_eq!(local_to_utc(m, 240), m * DAY + 2 * 3600);
        // Im Oktober umgekehrt
        let o = days_from_civil(2026, 10, 25);
        assert_eq!(day_start(o), o * DAY - 7200);
        assert_eq!(local_to_utc(o, 240), o * DAY + 3 * 3600);

        // 23:30 in Berlin gehört noch zum Tag, 00:30 schon zum nächsten
        assert_eq!(local_day(berlin(2026, 11, 16, 23, 30)), d);
        assert_eq!(local_day(berlin(2026, 11, 17, 0, 30)), d + 1);
        assert_eq!(local_day(berlin(2026, 7, 1, 0, 30)), s);
    }

    #[test]
    fn uhrzeit_lesen() {
        assert_eq!(parse_minute("04:00"), Some(240));
        assert_eq!(parse_minute("4:30"), Some(270));
        assert_eq!(parse_minute("23:59"), Some(1439));
        assert_eq!(parse_minute("24:00"), None);
        assert_eq!(parse_minute("4"), None);
        assert_eq!(parse_minute("-1:00"), None);
        assert_eq!(format_minute(240), "04:00");
        assert_eq!(
            format_local(berlin(2026, 11, 16, 10, 5)),
            "16.11.2026 10:05"
        );
        assert_eq!(format_local(berlin(2026, 7, 1, 0, 30)), "01.07.2026 00:30");
    }

    #[test]
    fn gestellte_uhr() {
        let c = Clock::fixed(100);
        let c2 = c.clone();
        c.advance(50);
        assert_eq!(c2.now(), 150);
        assert!(Clock::system().now() > 1_700_000_000);
    }
}
