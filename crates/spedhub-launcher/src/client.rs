//! Client für den SpedHub-Server, siehe `server/README.md`.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Login {
    pub user_id: i64,
    pub name: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Me {
    pub user_id: i64,
    pub name: String,
    pub companies: Vec<CompanyRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct CompanyRef {
    pub id: i64,
    pub name: String,
    pub role: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Company {
    pub id: i64,
    pub name: String,
    pub your_role: String,
    pub balance: i64,
    pub members: Vec<Member>,
    /// Betriebsvorschuss: so viel Geld hat der Fahrer im Spiel zusätzlich zum Lohnkonto
    #[serde(default)]
    pub advance: i64,
    #[serde(default)]
    pub daily_flat: i64,
    #[serde(default)]
    pub revenue_factor: f64,
    #[serde(default)]
    pub payroll_time: String,
}

impl Company {
    pub fn is_chef(&self) -> bool {
        self.your_role == "chef"
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Member {
    pub user_id: i64,
    pub name: String,
    pub role: String,
    pub wallet: i64,
    #[serde(default)]
    pub open_claim: i64,
    #[serde(default)]
    pub company_km: i64,
    #[serde(default)]
    pub game_level: i64,
    /// Lohnstufe, `None`, wenn keine passt
    #[serde(default)]
    pub stage: Option<i64>,
    #[serde(default)]
    pub cents_per_km: i64,
}

/// Ein Lkw im Fuhrpark, wie die Liste ihn zeigt.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Truck {
    pub id: i64,
    pub plate: String,
    pub model: String,
    pub odometer_km: i64,
    pub rev: i64,
    pub leased_by: Option<Lease>,
    /// Frühere Kennzeichen, älteste zuerst
    #[serde(default)]
    pub old_plates: Vec<String>,
    /// Der Ausleiher ist gerade online und fährt genau diesen Lkw (aus dem Herzschlag des
    /// Plugins). Ältere Server liefern das nicht.
    #[serde(default)]
    pub driving: bool,
    /// Geschwindigkeit in km/h, nur solange `driving`
    #[serde(default)]
    pub speed_kmh: Option<f64>,
    /// Reparierbarer Schaden in Prozent aus dem Zustand der letzten Rückgabe. Ältere Server
    /// liefern das nicht.
    #[serde(default)]
    pub damage: f64,
    /// Dauerhafter Verschleiß in Prozent, lässt sich nicht reparieren
    #[serde(default)]
    pub damage_unfixable: f64,
    /// Der Lkw steht in der Werkstatt und lässt sich erst nach der Reparatur ausleihen
    #[serde(default)]
    pub in_workshop: bool,
    /// Was eine Reparatur jetzt kosten würde, in Euro
    #[serde(default)]
    pub repair_cost: i64,
}

impl Truck {
    /// Schaden für die Fuhrparkliste, etwa `12,5 %, dauerhaft 3 %`.
    pub fn damage_text(&self) -> String {
        let mut text = format!("{} %", format_percent(self.damage));
        if self.damage_unfixable > 0.0 {
            text.push_str(&format!(
                ", dauerhaft {} %",
                format_percent(self.damage_unfixable)
            ));
        }
        text
    }
}

/// Prozent mit höchstens einer Nachkommastelle und deutschem Komma: `12,5`, `3`.
pub fn format_percent(p: f64) -> String {
    let p = (p * 10.0).round() / 10.0;
    if p.fract() == 0.0 {
        format!("{p:.0}")
    } else {
        format!("{p:.1}").replace('.', ",")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Lease {
    pub user_id: i64,
    pub name: String,
    pub since: i64,
}

#[derive(Deserialize)]
struct TruckList {
    trucks: Vec<Truck>,
}

/// Antwort beim Ausleihen: Bauplan und Zustand für den Spielstand.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Checkout {
    pub lease_id: i64,
    pub truck: LeasedTruck,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LeasedTruck {
    pub id: i64,
    pub plate: String,
    pub model: String,
    pub package: String,
    pub odometer_km: i64,
    pub rev: i64,
    pub state: Value,
}

/// Antwort von `GET /api/trucks/{id}/lease`: die laufende Leihe mit Bauplan.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LeaseInfo {
    pub lease_id: i64,
    /// Wer den Lkw hat
    pub user_id: i64,
    pub truck: LeasedTruck,
}

/// Antwort beim Geldabgleich.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SyncResult {
    pub sync_id: i64,
    /// Erster Abgleich dieses Fahrers: nichts gebucht
    pub first: bool,
    pub ingame_money: i64,
    /// Was die gemeldeten Ereignisse erklären, `None` beim ersten Abgleich
    pub expected: Option<i64>,
    pub booked: Vec<SyncBooking>,
    /// Mehr Geld im Spiel als erklärt
    pub unexplained: Option<Unexplained>,
    pub wallet: i64,
    pub advance: i64,
    /// Das kommt in den Spielstand
    pub write_money: i64,
    #[serde(default)]
    pub game_level: i64,
    #[serde(default)]
    pub stage: Option<i64>,
    /// Bis hier zählt der Server Ereignisse zu diesem Abgleich: die gemeldete Speicherzeit,
    /// oder seine Empfangszeit, wenn sie fehlte oder unplausibel war
    #[serde(default)]
    pub saved_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SyncBooking {
    pub account: String,
    pub kind: String,
    pub amount: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Unexplained {
    pub id: i64,
    pub amount: i64,
}

/// Ein neuer Lkw für `POST /api/companies/{id}/trucks`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NewTruck {
    pub plate: String,
    pub model: String,
    pub package: String,
    pub odometer_km: i64,
    pub state: Value,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Checkin {
    pub id: i64,
    /// Kennzeichen nach der Rückgabe
    #[serde(default)]
    pub plate: String,
    pub odometer_km: i64,
    pub rev: i64,
    pub driven_km: i64,
    /// Schaden in Prozent nach der Rückgabe
    #[serde(default)]
    pub damage: f64,
    /// Schaden, der während dieser Leihe dazugekommen ist, in Prozentpunkten
    #[serde(default)]
    pub damage_new: f64,
    /// Was der Ausleiher dafür aus dem Lohnkonto zahlt, in Euro
    #[serde(default)]
    pub liability: i64,
    /// Über der Schadensgrenze: Der Lkw steht jetzt in der Werkstatt
    #[serde(default)]
    pub in_workshop: bool,
}

pub struct Client {
    base: String,
    token: Option<String>,
    agent: ureq::Agent,
}

impl Client {
    pub fn new(server_url: &str) -> Client {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(15)))
            .timeout_connect(Some(Duration::from_secs(5)))
            .http_status_as_error(false)
            .proxy(None)
            .user_agent(concat!("spedhub-launcher/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Client {
            base: server_url.trim().trim_end_matches('/').to_string(),
            token: None,
            agent,
        }
    }

    pub fn with_token(server_url: &str, token: &str) -> Client {
        let mut c = Client::new(server_url);
        c.token = Some(token.to_string());
        c
    }

    pub fn server_url(&self) -> &str {
        &self.base
    }

    pub fn token(&self) -> Option<&str> {
        self.token.as_deref()
    }

    /// Prüft die Adresse, bevor eine Anfrage rausgeht.
    fn check_url(&self) -> Result<()> {
        let rest = self
            .base
            .strip_prefix("http://")
            .or_else(|| self.base.strip_prefix("https://"))
            .unwrap_or_default();
        if rest.is_empty() || rest.contains(char::is_whitespace) {
            return Err(Error::Other(
                "Die Server-Adresse muss mit http:// oder https:// beginnen, zum Beispiel http://127.0.0.1:8787".into(),
            ));
        }
        Ok(())
    }

    pub fn register(&mut self, name: &str, password: &str) -> Result<Login> {
        let login: Login = self.post(
            "/api/register",
            &json!({ "name": name, "password": password }),
        )?;
        self.token = Some(login.token.clone());
        Ok(login)
    }

    pub fn login(&mut self, name: &str, password: &str) -> Result<Login> {
        let login: Login =
            self.post("/api/login", &json!({ "name": name, "password": password }))?;
        self.token = Some(login.token.clone());
        Ok(login)
    }

    pub fn logout(&mut self) -> Result<()> {
        let _: Value = self.post("/api/logout", &json!({}))?;
        self.token = None;
        Ok(())
    }

    /// Passwort ändern. Alle anderen Anmeldungen dieses Kontos werden ungültig, dieser Client
    /// behält seinen Token.
    pub fn change_password(&self, old_password: &str, new_password: &str) -> Result<()> {
        let _: Value = self.post(
            "/api/me/password",
            &json!({ "old_password": old_password, "new_password": new_password }),
        )?;
        Ok(())
    }

    pub fn me(&self) -> Result<Me> {
        self.get("/api/me")
    }

    pub fn company(&self, id: i64) -> Result<Company> {
        self.get(&format!("/api/companies/{id}"))
    }

    pub fn trucks(&self, company_id: i64) -> Result<Vec<Truck>> {
        let list: TruckList = self.get(&format!("/api/companies/{company_id}/trucks"))?;
        Ok(list.trucks)
    }

    /// Lkw anlegen (nur Chef). Liefert die neue ID.
    pub fn add_truck(&self, company_id: i64, truck: &NewTruck) -> Result<i64> {
        let v: Value = self.post(&format!("/api/companies/{company_id}/trucks"), truck)?;
        v.get("id")
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::Network("Antwort ohne id".into()))
    }

    pub fn checkout(&self, truck_id: i64) -> Result<Checkout> {
        self.post(&format!("/api/trucks/{truck_id}/checkout"), &json!({}))
    }

    /// Lkw zurückgeben. Mit `plate` meldet der Launcher ein neues Kennzeichen, falls das Spiel
    /// es ersetzt hat. Der Server behält das alte in der Liste `old_plates`.
    pub fn checkin(
        &self,
        truck_id: i64,
        odometer_km: i64,
        state: Option<&Value>,
        plate: Option<&str>,
    ) -> Result<Checkin> {
        let mut body = json!({ "odometer_km": odometer_km });
        if let Some(s) = state {
            body["state"] = s.clone();
        }
        if let Some(p) = plate {
            body["plate"] = json!(p);
        }
        self.post(&format!("/api/trucks/{truck_id}/checkin"), &body)
    }

    /// Die laufende Leihe eines Lkw mit Bauplan, Zustand und Kennzeichen. Darf der Fahrer, der
    /// ihn hat, und der Chef.
    pub fn lease(&self, truck_id: i64) -> Result<LeaseInfo> {
        self.get(&format!("/api/trucks/{truck_id}/lease"))
    }

    /// Geldabgleich: Kontostand und Erfahrungspunkte aus dem Spielstand melden, dazu mit
    /// `saved_at` die Änderungszeit des gelesenen Spielstands (Unix-Sekunden). Der Server
    /// zählt dann nur Ereignisse bis zu diesem Zeitpunkt.
    pub fn sync(
        &self,
        company_id: i64,
        ingame_money: i64,
        xp: Option<u64>,
        saved_at: Option<i64>,
    ) -> Result<SyncResult> {
        let mut body = json!({ "ingame_money": ingame_money });
        if let Some(xp) = xp {
            body["xp"] = json!(xp);
        }
        if let Some(t) = saved_at {
            body["saved_at"] = json!(t);
        }
        self.post(&format!("/api/companies/{company_id}/sync"), &body)
    }

    /// Meldet, dass der Betrag aus dem Abgleich im Spielstand steht. `written_at` ist der
    /// Zeitpunkt, zu dem der neue Speicherplatz geschrieben wurde (Unix-Sekunden), ab da
    /// zählt der nächste Abgleich.
    pub fn sync_written(
        &self,
        company_id: i64,
        sync_id: i64,
        written_at: Option<i64>,
    ) -> Result<()> {
        let body = match written_at {
            Some(t) => json!({ "written_at": t }),
            None => json!({}),
        };
        let _: Value = self.post(
            &format!("/api/companies/{company_id}/sync/{sync_id}/written"),
            &body,
        )?;
        Ok(())
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.check_url()?;
        let mut req = self.agent.get(&format!("{}{path}", self.base));
        if let Some(t) = &self.token {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        read(req.call())
    }

    fn post<T: DeserializeOwned>(&self, path: &str, body: &impl Serialize) -> Result<T> {
        self.check_url()?;
        let mut req = self.agent.post(&format!("{}{path}", self.base));
        if let Some(t) = &self.token {
            req = req.header("Authorization", &format!("Bearer {t}"));
        }
        read(req.send_json(body))
    }
}

/// Wertet eine Antwort aus. Fehler des Servers kommen als `{ "error": "..." }` und werden im
/// Klartext durchgereicht.
fn read<T: DeserializeOwned>(
    response: std::result::Result<ureq::http::Response<ureq::Body>, ureq::Error>,
) -> Result<T> {
    let mut response = response.map_err(|e| Error::Network(e.to_string()))?;
    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .with_config()
        .limit(16 * 1024 * 1024)
        .read_to_string()
        .map_err(|e| Error::Network(e.to_string()))?;
    if !(200..300).contains(&status) {
        let message = serde_json::from_str::<Value>(&body)
            .ok()
            .and_then(|v| v.get("error")?.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("Antwort {status}"));
        return Err(Error::Server { status, message });
    }
    serde_json::from_str(&body)
        .map_err(|e| Error::Network(format!("unerwartete Antwort vom Server: {e}")))
}
