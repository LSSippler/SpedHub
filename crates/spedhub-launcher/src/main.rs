//! Oberfläche des Launchers. Die Logik liegt in der Bibliothek `spedhub_launcher`, hier wird
//! nur angezeigt und weitergereicht. Alles, was dauert (Server, Spielstand), läuft in einem
//! eigenen Thread, damit das Fenster nicht hängt.
#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui;
use spedhub_launcher::client::{Client, Company, Me, Truck};
use spedhub_launcher::flows::{describe, lease_package, sync_and_write};
use spedhub_launcher::profiles::{Profile, SaveSlot, find_profiles, list_slots};
use spedhub_launcher::savegame::{
    FoundBy, ReturnData, SaveTruck, new_truck_from_save, package_from_checkout, read_for_return,
    trucks_in_save,
};
use spedhub_launcher::settings::{
    AppPaths, DEFAULT_SERVER, Settings, remove_plugin_config, write_plugin_config,
};
use spedhub_launcher::system::{self, ProcessCheck};
use spedhub_launcher::{Error, Result};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::mpsc::{Receiver, Sender, channel},
    time::SystemTime,
};

const UNCONFIRMED: &str = "Hinweis: Ein eingesetzter Lkw lädt im Spiel, das ist bestätigt. \
Noch nicht bestätigt ist, ob das Spiel den neuen Speicherplatz des Launchers anzeigt. \
Der Launcher legt immer einen neuen Speicherplatz an und sichert den alten vorher. \
Trotzdem gilt: vorher selbst ein Backup des Profils anlegen.";

const STEAM_WARNING: &str = "Achtung: Dieses Profil liegt in steam_profiles und wird mit Steam Cloud abgeglichen. \
Steam kann einen bearbeiteten Spielstand mit einer älteren Fassung aus der Cloud überschreiben. \
Besser ein Profil ohne Steam Cloud als Firmenprofil nehmen.";

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SpedHub")
            .with_inner_size([900.0, 640.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "SpedHub",
        options,
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
    )
}

/// Ergebnis einer Arbeit im Hintergrund.
enum Done {
    LoggedIn {
        token: String,
        user_name: String,
    },
    Loaded {
        me: Me,
        company: Option<Company>,
        trucks: Vec<Truck>,
    },
    /// Erfolg mit Meldung, danach wird neu geladen
    Info(String),
    ReturnRead {
        truck_id: i64,
        plate: String,
        data: Option<ReturnData>,
        /// Warum der Spielstand nichts hergab, zum Beispiel zwei gleich passende Lkw
        problem: Option<String>,
    },
    SaveTrucks(Vec<SaveTruck>),
    LoggedOut,
    Failed(Error),
}

struct ReturnDialog {
    truck_id: i64,
    plate: String,
    data: Option<ReturnData>,
    problem: Option<String>,
    manual_km: String,
}

struct Message {
    text: String,
    error: bool,
}

struct App {
    paths: Option<AppPaths>,
    settings: Settings,
    ets2_dir: Option<PathBuf>,
    profiles: Vec<Profile>,

    // Anmeldung
    name: String,
    password: String,
    token: Option<String>,

    // Firma
    me: Option<Me>,
    company: Option<Company>,
    trucks: Vec<Truck>,

    // Lkw übernehmen
    import_slots: Vec<SaveSlot>,
    import_slot: Option<usize>,
    import_trucks: Vec<SaveTruck>,
    import_plate: Option<String>,

    returning: Option<ReturnDialog>,
    message: Option<Message>,
    busy: Option<String>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
}

impl App {
    fn new(ctx: &egui::Context) -> App {
        let paths = AppPaths::from_env();
        let settings = paths.as_ref().map(Settings::load).unwrap_or_else(|| {
            let mut s = Settings::default();
            s.server_url = DEFAULT_SERVER.into();
            s
        });
        let ets2_dir = system::ets2_documents_dir();
        let profiles = ets2_dir.as_deref().map(find_profiles).unwrap_or_default();
        let (tx, rx) = channel();
        let mut app = App {
            name: settings.user_name.clone(),
            token: settings.token(),
            paths,
            settings,
            ets2_dir,
            profiles,
            password: String::new(),
            me: None,
            company: None,
            trucks: Vec::new(),
            import_slots: Vec::new(),
            import_slot: None,
            import_trucks: Vec::new(),
            import_plate: None,
            returning: None,
            message: None,
            busy: None,
            tx,
            rx,
        };
        if app.paths.is_none() {
            app.error("LOCALAPPDATA ist nicht gesetzt. Einstellungen und plugin.json lassen sich nicht speichern");
        }
        if app.token.is_some() {
            app.refresh(ctx);
        }
        app
    }

    // ------------------------------------------------------------------ Hilfen

    fn info(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            error: false,
        });
    }

    fn error(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            error: true,
        });
    }

    fn client(&self) -> Client {
        match &self.token {
            Some(t) => Client::with_token(&self.settings.server_url, t),
            None => Client::new(&self.settings.server_url),
        }
    }

    fn profile(&self) -> Option<&Profile> {
        let dir = self.settings.profile_dir.as_ref()?;
        self.profiles.iter().find(|p| &p.dir == dir)
    }

    fn save_settings(&mut self) {
        if let Some(paths) = &self.paths
            && let Err(e) = self.settings.save(paths)
        {
            self.error(format!("Einstellungen ließen sich nicht speichern: {e}"));
        }
    }

    /// Schreibt plugin.json für die gewählte Firma.
    fn write_plugin_config(&mut self) {
        let (Some(paths), Some(token), Some(company)) =
            (&self.paths, &self.token, self.settings.company_id)
        else {
            return;
        };
        if let Err(e) = write_plugin_config(paths, &self.settings.server_url, token, company) {
            self.error(format!("plugin.json ließ sich nicht schreiben: {e}"));
        }
    }

    /// Startet eine Arbeit im Hintergrund. Eine Panic darin wird zur Fehlermeldung.
    fn run(
        &mut self,
        ctx: &egui::Context,
        label: &str,
        job: impl FnOnce() -> Result<Done> + Send + 'static,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(label.to_string());
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let done = match catch_unwind(AssertUnwindSafe(job)) {
                Ok(Ok(d)) => d,
                Ok(Err(e)) => Done::Failed(e),
                Err(_) => Done::Failed(Error::Other("Interner Fehler im Launcher".into())),
            };
            let _ = tx.send(done);
            ctx.request_repaint();
        });
    }

    /// Lädt Konto, Firma und Fuhrpark neu.
    fn refresh(&mut self, ctx: &egui::Context) {
        let client = self.client();
        let wanted = self.settings.company_id;
        self.run(ctx, "Fuhrpark wird geladen", move || {
            let me = client.me()?;
            let id = wanted
                .filter(|id| me.companies.iter().any(|c| c.id == *id))
                .or_else(|| me.companies.first().map(|c| c.id));
            let (company, trucks) = match id {
                Some(id) => (Some(client.company(id)?), client.trucks(id)?),
                None => (None, Vec::new()),
            };
            Ok(Done::Loaded {
                me,
                company,
                trucks,
            })
        });
    }

    fn handle(&mut self, ctx: &egui::Context, done: Done) {
        self.busy = None;
        match done {
            Done::LoggedIn { token, user_name } => {
                self.token = Some(token.clone());
                self.password.clear();
                self.settings.user_name = user_name;
                if let Err(e) = self.settings.set_token(&token) {
                    self.error(format!("Der Token ließ sich nicht sicher speichern: {e}"));
                }
                self.save_settings();
                self.refresh(ctx);
            }
            Done::Loaded {
                me,
                company,
                trucks,
            } => {
                if company.is_none() {
                    self.info("Du bist noch in keiner Firma. Gründe eine im Dashboard oder tritt mit einem Einladungscode bei");
                }
                let changed = company.as_ref().map(|c| c.id) != self.settings.company_id;
                self.settings.company_id = company.as_ref().map(|c| c.id);
                self.me = Some(me);
                self.company = company;
                self.trucks = trucks;
                if changed {
                    self.save_settings();
                }
                self.write_plugin_config();
            }
            Done::Info(text) => {
                self.info(text);
                self.refresh(ctx);
            }
            Done::ReturnRead {
                truck_id,
                plate,
                data,
                problem,
            } => {
                self.returning = Some(ReturnDialog {
                    truck_id,
                    plate,
                    manual_km: String::new(),
                    data,
                    problem,
                });
            }
            Done::SaveTrucks(trucks) => {
                self.import_plate = None;
                self.import_trucks = trucks;
            }
            Done::LoggedOut => {
                self.token = None;
                self.me = None;
                self.company = None;
                self.trucks.clear();
                self.settings.clear_token();
                self.save_settings();
                if let Some(paths) = &self.paths {
                    let _ = remove_plugin_config(paths);
                }
                self.info(
                    "Abgemeldet. Das Plugin meldet nichts mehr, bis du dich wieder anmeldest",
                );
            }
            Done::Failed(e) => {
                if e.is_unauthorized() && self.token.is_some() {
                    self.token = None;
                    self.settings.clear_token();
                    self.save_settings();
                    self.error("Die Anmeldung ist abgelaufen. Bitte neu anmelden");
                } else {
                    self.error(e.to_string());
                }
            }
        }
    }

    // ------------------------------------------------------------------ Aktionen

    fn login(&mut self, ctx: &egui::Context, register: bool) {
        let (url, name, password) = (
            self.settings.server_url.trim().to_string(),
            self.name.trim().to_string(),
            self.password.clone(),
        );
        self.settings.server_url = url.clone();
        let label = if register {
            "Konto wird angelegt"
        } else {
            "Anmeldung läuft"
        };
        self.run(ctx, label, move || {
            let mut c = Client::new(&url);
            let login = if register {
                c.register(&name, &password)?
            } else {
                c.login(&name, &password)?
            };
            Ok(Done::LoggedIn {
                token: login.token,
                user_name: login.name,
            })
        });
    }

    fn logout(&mut self, ctx: &egui::Context) {
        let mut client = self.client();
        self.run(ctx, "Abmelden", move || {
            // Auch wenn der Server nicht erreichbar ist: lokal abmelden
            let _ = client.logout();
            Ok(Done::LoggedOut)
        });
    }

    fn checkout(&mut self, ctx: &egui::Context, truck: &Truck) {
        let client = self.client();
        let (truck_id, plate) = (truck.id, truck.plate.clone());
        let company = self.company.as_ref().map(|c| c.id);
        let paths = self.paths.clone();
        let profile = self.profile().map(|p| p.dir.clone());
        self.run(ctx, "Lkw wird ausgeliehen", move || {
            let checkout = client.checkout(truck_id)?;
            // Ab hier ist der Lkw ausgeliehen. Was danach schiefgeht, steht in der Meldung,
            // damit niemand denkt, das Ausleihen sei gescheitert.
            let not_in_save = |e: Error| {
                Done::Info(format!(
                    "{plate} ist ausgeliehen, aber nicht im Spielstand: {e}"
                ))
            };
            let package = match package_from_checkout(&checkout.truck) {
                Ok(p) => p,
                Err(e) => return Ok(not_in_save(e)),
            };
            if let Some(p) = &paths
                && let Err(e) = p.store_lease(truck_id, &package)
            {
                return Ok(not_in_save(e));
            }
            let Some(profile) = profile else {
                return Ok(Done::Info(format!(
                    "{plate} ist ausgeliehen. Wähle ein Firmenprofil und schreibe den Lkw dann in den Spielstand"
                )));
            };
            match deploy(&client, company, &profile, Some(&package), paths.as_ref()) {
                Ok(text) => Ok(Done::Info(format!("{plate} ist ausgeliehen. {text}"))),
                Err(e) => {
                    // Nur wenn das Spiel lief, hilft ein zweiter Versuch später
                    let later = if e == Error::GameRunning {
                        ". Später mit \"In Spielstand schreiben\" erneut versuchen"
                    } else {
                        ""
                    };
                    Ok(Done::Info(format!(
                        "{plate} ist ausgeliehen, aber nicht im Spielstand: {e}{later}"
                    )))
                }
            }
        });
    }

    fn write_to_save(&mut self, ctx: &egui::Context, truck: &Truck) {
        let Some(profile) = self.profile().map(|p| p.dir.clone()) else {
            self.error("Bitte zuerst ein Firmenprofil wählen");
            return;
        };
        let paths = self.paths.clone();
        let client = self.client();
        let company = self.company.as_ref().map(|c| c.id);
        let truck_id = truck.id;
        self.run(ctx, "Lkw wird in den Spielstand geschrieben", move || {
            // Liegt der Bauplan nicht auf diesem Rechner, kommt er vom Server
            let package = package_for(&client, paths.as_ref(), truck_id)?;
            deploy(&client, company, &profile, Some(&package), paths.as_ref()).map(Done::Info)
        });
    }

    /// Geldabgleich ohne Lkw: Kontostand und Erfahrung melden, neuen Kontostand schreiben.
    fn sync_money(&mut self, ctx: &egui::Context) {
        let Some(profile) = self.profile().map(|p| p.dir.clone()) else {
            self.error("Bitte zuerst ein Firmenprofil wählen");
            return;
        };
        let paths = self.paths.clone();
        let client = self.client();
        let company = self.company.as_ref().map(|c| c.id);
        self.run(ctx, "Geld wird abgeglichen", move || {
            deploy(&client, company, &profile, None, paths.as_ref()).map(Done::Info)
        });
    }

    fn start_return(&mut self, ctx: &egui::Context, truck: &Truck) {
        let (truck_id, plate) = (truck.id, truck.plate.clone());
        let Some(profile) = self.profile().map(|p| p.dir.clone()) else {
            self.returning = Some(ReturnDialog {
                truck_id,
                plate,
                data: None,
                problem: Some("Kein Firmenprofil gewählt".into()),
                manual_km: String::new(),
            });
            return;
        };
        let paths = self.paths.clone();
        let client = self.client();
        self.run(ctx, "Spielstand wird gelesen", move || {
            // Der Bauplan hilft, den Lkw ohne Kennzeichen wiederzuerkennen. Ohne ihn geht es
            // nur über das Kennzeichen.
            let package = package_for(&client, paths.as_ref(), truck_id).ok();
            // Lässt sich der Spielstand nicht lesen, bleibt die Eingabe von Hand.
            let (data, problem) = match read_for_return(&profile, &plate, package.as_ref()) {
                Ok(d) => (d, None),
                Err(e) => (None, Some(e.to_string())),
            };
            Ok(Done::ReturnRead {
                truck_id,
                plate,
                data,
                problem,
            })
        });
    }

    fn finish_return(&mut self, ctx: &egui::Context) {
        let Some(dialog) = &self.returning else {
            return;
        };
        let (km, state, new_plate) = match &dialog.data {
            Some(d) => (d.odometer_km, Some(d.state.clone()), d.new_plate.clone()),
            None => match dialog.manual_km.trim().parse::<i64>() {
                Ok(km) if km >= 0 => (km, None, None),
                _ => {
                    self.error("Bitte den Kilometerstand als ganze Zahl eingeben");
                    return;
                }
            },
        };
        let (truck_id, plate) = (dialog.truck_id, dialog.plate.clone());
        self.returning = None;
        let client = self.client();
        let paths = self.paths.clone();
        self.run(ctx, "Lkw wird zurückgegeben", move || {
            let r = client.checkin(truck_id, km, state.as_ref(), new_plate.as_deref())?;
            if let Some(p) = &paths {
                p.remove_lease(truck_id);
            }
            let renamed = match &new_plate {
                Some(p) => format!(". Neues Kennzeichen {p}, das alte merkt sich der Server"),
                None => String::new(),
            };
            Ok(Done::Info(format!(
                "{plate} ist zurückgegeben, {} km gefahren{renamed}",
                r.driven_km
            )))
        });
    }

    fn load_save_trucks(&mut self, ctx: &egui::Context) {
        let Some(slot) = self.import_slot.and_then(|i| self.import_slots.get(i)) else {
            return;
        };
        let game = slot.game();
        self.run(ctx, "Spielstand wird gelesen", move || {
            trucks_in_save(&game).map(Done::SaveTrucks)
        });
    }

    fn import_truck(&mut self, ctx: &egui::Context) {
        let (Some(slot), Some(plate), Some(company)) = (
            self.import_slot.and_then(|i| self.import_slots.get(i)),
            self.import_plate.clone(),
            self.company.as_ref().map(|c| c.id),
        ) else {
            return;
        };
        let game = slot.game();
        let client = self.client();
        self.run(ctx, "Lkw wird übernommen", move || {
            let truck = new_truck_from_save(&game, &plate)?;
            client.add_truck(company, &truck)?;
            Ok(Done::Info(format!(
                "{} ({}, {} km) ist jetzt im Fuhrpark",
                truck.plate, truck.model, truck.odometer_km
            )))
        });
    }

    // ------------------------------------------------------------------ Ansichten

    fn login_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Anmelden");
        ui.add_space(8.0);
        egui::Grid::new("login")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label("Server");
                ui.text_edit_singleline(&mut self.settings.server_url);
                ui.end_row();
                ui.label("Name");
                ui.text_edit_singleline(&mut self.name);
                ui.end_row();
                ui.label("Passwort");
                ui.add(egui::TextEdit::singleline(&mut self.password).password(true));
                ui.end_row();
            });
        ui.add_space(8.0);
        let ready =
            !self.name.trim().is_empty() && !self.password.is_empty() && self.busy.is_none();
        ui.horizontal(|ui| {
            if ui
                .add_enabled(ready, egui::Button::new("Anmelden"))
                .clicked()
            {
                self.login(ui.ctx(), false);
            }
            if ui
                .add_enabled(ready, egui::Button::new("Konto anlegen"))
                .clicked()
            {
                self.login(ui.ctx(), true);
            }
        });
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if let Some(me) = &self.me {
                ui.label(format!("Angemeldet als {}", me.name));
            }
            ui.separator();
            let companies = self
                .me
                .as_ref()
                .map(|m| m.companies.clone())
                .unwrap_or_default();
            if !companies.is_empty() {
                let current = self
                    .company
                    .as_ref()
                    .map_or_else(|| "Firma wählen".to_string(), |c| c.name.clone());
                let mut chosen = self.settings.company_id;
                egui::ComboBox::from_id_salt("firma")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        for c in &companies {
                            let role = if c.role == "chef" { "Chef" } else { "Fahrer" };
                            ui.selectable_value(
                                &mut chosen,
                                Some(c.id),
                                format!("{} ({role})", c.name),
                            );
                        }
                    });
                if chosen != self.settings.company_id {
                    self.settings.company_id = chosen;
                    self.save_settings();
                    self.write_plugin_config();
                    self.refresh(ui.ctx());
                }
            }
            if ui
                .add_enabled(self.busy.is_none(), egui::Button::new("Aktualisieren"))
                .clicked()
            {
                self.refresh(ui.ctx());
            }
            if ui.button("Spiel starten").clicked() {
                match system::start_game() {
                    Ok(()) => self.info("Steam startet Euro Truck Simulator 2"),
                    Err(e) => self.error(e),
                }
            }
            if ui
                .add_enabled(self.busy.is_none(), egui::Button::new("Abmelden"))
                .clicked()
            {
                self.logout(ui.ctx());
            }
        });
    }

    fn profile_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label("Firmenprofil");
            let current = self
                .profile()
                .map_or_else(|| "Profil wählen".to_string(), profile_label);
            let mut chosen = self.settings.profile_dir.clone();
            egui::ComboBox::from_id_salt("profil")
                .selected_text(current)
                .width(260.0)
                .show_ui(ui, |ui| {
                    for p in &self.profiles {
                        ui.selectable_value(&mut chosen, Some(p.dir.clone()), profile_label(p));
                    }
                });
            if chosen != self.settings.profile_dir {
                self.settings.profile_dir = chosen;
                self.import_slots.clear();
                self.import_slot = None;
                self.import_trucks.clear();
                self.save_settings();
            }
            if ui.button("Profile neu suchen").clicked() {
                self.profiles = self
                    .ets2_dir
                    .as_deref()
                    .map(find_profiles)
                    .unwrap_or_default();
            }
        });
        if self.profiles.is_empty() {
            let place = self.ets2_dir.as_ref().map_or_else(
                || "Der Dokumente-Ordner ließ sich nicht finden".to_string(),
                |d| format!("Keine Profile gefunden unter {}", d.display()),
            );
            ui.label(place);
        }
        if self.profile().is_some_and(|p| p.steam_cloud) {
            ui.colored_label(egui::Color32::from_rgb(200, 120, 0), STEAM_WARNING);
        }
    }

    fn fleet_view(&mut self, ui: &mut egui::Ui) {
        let Some(company) = self.company.clone() else {
            return;
        };
        ui.heading(format!("Fuhrpark von {}", company.name));
        ui.label(format!("Firmenkasse: {} Euro", company.balance));
        let my_id = self.me.as_ref().map(|m| m.user_id);
        if let Some(m) = company.members.iter().find(|m| Some(m.user_id) == my_id) {
            let stage = m.stage.map_or_else(
                || "keine Lohnstufe".to_string(),
                |s| format!("Lohnstufe {s}"),
            );
            let claim = if m.open_claim > 0 {
                format!(", offene Lohnforderung {} Euro", m.open_claim)
            } else {
                String::new()
            };
            ui.label(format!(
                "Dein Lohnkonto: {} Euro{claim}. Level {}, {} Firmenkilometer, {stage} ({} Cent je km)",
                m.wallet, m.game_level, m.company_km, m.cents_per_km
            ));
        }
        ui.horizontal_wrapped(|ui| {
            let ready = self.busy.is_none() && self.profile().is_some();
            if ui
                .add_enabled(ready, egui::Button::new("Geld abgleichen"))
                .clicked()
            {
                self.sync_money(ui.ctx());
            }
            ui.label(format!(
                "Liest Kontostand und Erfahrung aus dem neuesten Spielstand und legt einen neuen Speicherplatz mit Lohnkonto plus {} Euro Vorschuss an",
                company.advance
            ));
        });
        ui.add_space(4.0);
        if self.trucks.is_empty() {
            ui.label("Noch keine Lkw im Fuhrpark");
        }
        let mut action: Option<(&'static str, Truck)> = None;
        egui::Grid::new("fuhrpark")
            .num_columns(5)
            .striped(true)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.strong("Kennzeichen");
                ui.strong("Modell");
                ui.strong("km");
                ui.strong("Fährt gerade");
                ui.strong("");
                ui.end_row();
                for t in &self.trucks {
                    ui.label(&t.plate);
                    ui.label(&t.model);
                    ui.label(t.odometer_km.to_string());
                    let mine = t
                        .leased_by
                        .as_ref()
                        .is_some_and(|l| Some(l.user_id) == my_id);
                    match &t.leased_by {
                        None => ui.label("frei"),
                        Some(_) if mine => ui.label("du"),
                        Some(l) => ui.label(&l.name),
                    };
                    ui.horizontal(|ui| {
                        let idle = self.busy.is_none();
                        if t.leased_by.is_none() {
                            if ui
                                .add_enabled(idle, egui::Button::new("Ausleihen"))
                                .clicked()
                            {
                                action = Some(("ausleihen", t.clone()));
                            }
                        } else if mine {
                            if ui
                                .add_enabled(idle, egui::Button::new("In Spielstand schreiben"))
                                .clicked()
                            {
                                action = Some(("schreiben", t.clone()));
                            }
                            if ui
                                .add_enabled(idle, egui::Button::new("Zurückgeben"))
                                .clicked()
                            {
                                action = Some(("zurueck", t.clone()));
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        match action {
            Some(("ausleihen", t)) => self.checkout(ui.ctx(), &t),
            Some(("schreiben", t)) => self.write_to_save(ui.ctx(), &t),
            Some(("zurueck", t)) => self.start_return(ui.ctx(), &t),
            _ => {}
        }
    }

    fn return_view(&mut self, ui: &mut egui::Ui) {
        let mut cancel = false;
        let mut confirm = false;
        let Some(dialog) = &mut self.returning else {
            return;
        };
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong(format!("{} zurückgeben", dialog.plate));
            match &dialog.data {
                Some(d) => {
                    let how = match d.found_by {
                        FoundBy::Plate => "Gefunden über das Kennzeichen".to_string(),
                        FoundBy::Parts { share } => format!(
                            "Gefunden über Modell und Teile ({} Prozent Übereinstimmung)",
                            (share * 100.0).floor()
                        ),
                    };
                    ui.label(format!(
                        "{how}. Im neuesten Spielstand (Speicherplatz {}) steht der Lkw mit {} km. Zustand und Verschleiß gehen mit an den Server",
                        d.slot, d.odometer_km
                    ));
                    if let Some(p) = &d.new_plate {
                        ui.label(format!(
                            "Das Spiel hat das Kennzeichen ersetzt. Neu ist {p}, das meldet der Launcher mit"
                        ));
                    }
                }
                None => {
                    if let Some(problem) = &dialog.problem {
                        ui.colored_label(egui::Color32::from_rgb(200, 40, 40), problem);
                    }
                    ui.label("Im neuesten Spielstand ist der Lkw nicht sicher zu finden. Bitte den Kilometerstand von Hand eingeben");
                    ui.horizontal(|ui| {
                        ui.label("Kilometerstand");
                        ui.text_edit_singleline(&mut dialog.manual_km);
                    });
                }
            }
            ui.horizontal(|ui| {
                if ui.button("Zurückgeben").clicked() {
                    confirm = true;
                }
                if ui.button("Abbrechen").clicked() {
                    cancel = true;
                }
            });
        });
        if cancel {
            self.returning = None;
        } else if confirm {
            self.finish_return(ui.ctx());
        }
    }

    fn import_view(&mut self, ui: &mut egui::Ui) {
        if !self.company.as_ref().is_some_and(Company::is_chef) {
            return;
        }
        egui::CollapsingHeader::new("Lkw in den Fuhrpark übernehmen")
            .default_open(false)
            .show(ui, |ui| {
                let Some(profile) = self.profile().map(|p| p.dir.clone()) else {
                    ui.label("Bitte zuerst ein Profil wählen");
                    return;
                };
                if self.import_slots.is_empty() {
                    self.import_slots = list_slots(&profile);
                }
                if self.import_slots.is_empty() {
                    ui.label("Im Profil gibt es noch keinen Spielstand");
                    return;
                }
                ui.label("Aus welchem Spielstand?");
                let current = self
                    .import_slot
                    .and_then(|i| self.import_slots.get(i))
                    .map_or_else(|| "Spielstand wählen".to_string(), SaveSlot::label);
                let mut chosen = self.import_slot;
                egui::ComboBox::from_id_salt("spielstand")
                    .selected_text(current)
                    .width(260.0)
                    .show_ui(ui, |ui| {
                        for (i, s) in self.import_slots.iter().enumerate() {
                            ui.selectable_value(&mut chosen, Some(i), s.label());
                        }
                    });
                if chosen != self.import_slot {
                    self.import_slot = chosen;
                    self.import_trucks.clear();
                    self.load_save_trucks(ui.ctx());
                }
                for t in &self.import_trucks {
                    let model = t.model.as_deref().unwrap_or("unbekanntes Modell");
                    let extra = if t.active { ", fährt gerade" } else { "" };
                    ui.radio_value(
                        &mut self.import_plate,
                        Some(t.plate.clone()),
                        format!("{} ({model}{extra})", t.plate),
                    );
                }
                let ready = self.import_plate.is_some() && self.busy.is_none();
                if ui
                    .add_enabled(ready, egui::Button::new("Lkw übernehmen"))
                    .clicked()
                {
                    self.import_truck(ui.ctx());
                }
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if let Some(b) = &self.busy {
                ui.spinner();
                ui.label(b);
            } else if let Some(m) = &self.message {
                if m.error {
                    ui.colored_label(egui::Color32::from_rgb(200, 40, 40), &m.text);
                } else {
                    ui.label(&m.text);
                }
            }
        });
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Ok(done) = self.rx.try_recv() {
            let ctx = ui.ctx().clone();
            self.handle(&ctx, done);
        }
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                if self.token.is_none() {
                    self.login_view(ui);
                    return;
                }
                self.top_bar(ui);
                ui.separator();
                self.profile_view(ui);
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(200, 120, 0), UNCONFIRMED);
                ui.separator();
                self.return_view(ui);
                self.fleet_view(ui);
                ui.add_space(12.0);
                self.import_view(ui);
            });
        });
    }
}

fn profile_label(p: &Profile) -> String {
    if p.steam_cloud {
        format!("{} (Steam Cloud)", p.name)
    } else {
        p.name.clone()
    }
}

/// Bauplan eines geliehenen Lkw: aus `leases\` oder vom Server.
fn package_for(
    client: &Client,
    paths: Option<&AppPaths>,
    truck_id: i64,
) -> Result<spedhub_save::TruckPackage> {
    match paths {
        Some(p) => lease_package(client, p, truck_id).map(|(package, _)| package),
        None => package_from_checkout(&client.lease(truck_id)?.truck),
    }
}

/// Neuen Speicherplatz mit Geldabgleich anlegen, mit `package` auch mit dem Lkw. Text für die
/// Erfolgsmeldung.
fn deploy(
    client: &Client,
    company: Option<i64>,
    profile: &std::path::Path,
    package: Option<&spedhub_save::TruckPackage>,
    paths: Option<&AppPaths>,
) -> Result<String> {
    let company = company.ok_or_else(|| Error::Other("Bitte zuerst eine Firma wählen".into()))?;
    let backups = paths
        .map(AppPaths::backups)
        .ok_or_else(|| Error::Other("Kein Ort für Backups, LOCALAPPDATA fehlt".into()))?;
    let o = sync_and_write(
        client,
        company,
        profile,
        package,
        &backups,
        &ProcessCheck,
        SystemTime::now(),
    )?;
    let mut text = format!(
        "Neuer Speicherplatz \"{}\" angelegt. Im Spiel unter Laden auswählen. {}",
        o.deployed.name,
        describe(&o.sync)
    );
    if !o.confirmed {
        text.push_str(". Der Server hat die Bestätigung nicht bekommen, beim nächsten Abgleich kann deshalb eine Differenz entstehen");
    }
    Ok(text)
}
