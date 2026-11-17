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
    pub wage_percent: i64,
    pub your_role: String,
    pub balance: i64,
    pub members: Vec<Member>,
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
}

/// Ein Lkw im Fuhrpark, wie die Liste ihn zeigt.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
