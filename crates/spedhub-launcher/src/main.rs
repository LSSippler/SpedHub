//! Oberfläche des Launchers. Die Logik liegt in der Bibliothek `spedhub_launcher`, hier wird
//! nur angezeigt und weitergereicht. Alles, was dauert (Server, Spielstand), läuft in einem
//! eigenen Thread, damit das Fenster nicht hängt.
#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui::{
    self, Align, CornerRadius, Frame, Layout, Margin, Rect, Sense, Vec2, pos2, vec2,
};
use spedhub_launcher::cli::{self, Command, InstallEnv, InstallOutcome};
use spedhub_launcher::client::{
    Client, Company, Me, Member, PurchaseCandidate, Truck, TruckState, age_text, format_percent,
    version_warning,
};
use spedhub_launcher::flows::{
    ProfileReport, describe, lease_package, profile_to_report, return_truck, sync_and_write,
};
use spedhub_launcher::plugin_install::{self, GameSource, PluginStatus};
use spedhub_launcher::profiles::{
    Profile, SaveSlot, ets2_dir_of, find_profiles, game_log, list_slots, profile_name_of,
};
use spedhub_launcher::savegame::{
    FoundBy, ReturnData, SaveTruck, new_truck_from_save, package_from_checkout, read_for_return,
    trucks_in_save, unix_seconds,
};
use spedhub_launcher::settings::{
    AppPaths, DEFAULT_SERVER, Settings, remove_plugin_config, write_plugin_config,
};
use spedhub_launcher::system::{self, MessageKind, ProcessCheck};
use spedhub_launcher::theme::{
    self, ActionButton, CARD_MIN_WIDTH, CONTENT_MAX_WIDTH, CONTROL_HEIGHT, FS_BASE, FS_LG, FS_MD,
    FS_SM, FS_XS, Icon, Kpi, NAV_WIDTH, PlateSize, R_MD, R_SM, SP_1, SP_2, SP_3, SP_4, SP_5, SP_6,
    SP_10, Tone, regular, semibold, text,
};
use spedhub_launcher::update::{self, Release, Updater};
use spedhub_launcher::version::VERSION;
use spedhub_launcher::{Error, Result};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::mpsc::{Receiver, Sender, channel},
    time::SystemTime,
};

const UNCONFIRMED: &str = "Im Spiel bestätigt ist, dass ein eingesetzter Lkw lädt, der neue Speicherplatz des Launchers erscheint und das Spiel den Kontostand aus dem Geldabgleich übernimmt. \
Der Launcher legt immer einen neuen Speicherplatz an und sichert den alten vorher. \
Trotzdem gilt: vorher selbst ein Backup des Profils anlegen.";

const STEAM_WARNING: &str = "Dieses Profil liegt in steam_profiles und wird mit Steam Cloud abgeglichen. \
Steam kann einen bearbeiteten Spielstand mit einer älteren Fassung aus der Cloud überschreiben. \
Besser ein Profil ohne Steam Cloud als Firmenprofil nehmen.";

const NO_PROFILE: &str = "Ohne Firmenprofil meldet das Plugin nichts an den Server. \
Es zählt nur, was im Firmenprofil passiert.";

const TITLE: &str = "SpedHub";

fn main() {
    let code = match cli::parse_args(std::env::args_os().skip(1)) {
        Ok(Command::Gui) => match run_gui() {
            Ok(()) => cli::EXIT_OK,
            Err(e) => {
                system::message_box(
                    TITLE,
                    &format!("Das Fenster ließ sich nicht öffnen: {e}"),
                    MessageKind::Error,
                );
                cli::EXIT_ERROR
            }
        },
        Ok(command) => run_cli(command),
        Err(e) => {
            system::message_box(TITLE, &e, MessageKind::Error);
            cli::EXIT_USAGE
        }
    };
    std::process::exit(code);
}

fn run_gui() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("SpedHub {VERSION}"))
            .with_inner_size([1180.0, 780.0])
            .with_min_inner_size([820.0, 540.0]),
        ..Default::default()
    };
    eframe::run_native(
        "SpedHub",
        options,
        Box::new(|cc| {
            let (regular, semibold) = system::ui_fonts();
            let preference =
                theme::theme_preference(std::env::var(theme::THEME_VAR).ok().as_deref());
            theme::install(
                &cc.egui_ctx,
                theme::font_definitions(regular, semibold),
                preference,
            );
            Ok(Box::new(App::new(&cc.egui_ctx)))
        }),
    )
}

/// `--install-plugin`, `--uninstall-plugin`, `--remember-game-dir` und `--version` ohne
/// Oberfläche. Liefert den Exit-Code. Fehler stehen in einer MessageBox, mit `--quiet` nur im
/// Exit-Code.
fn run_cli(command: Command) -> i32 {
    let paths = AppPaths::from_env();
    let env = InstallEnv {
        paths: paths.as_ref(),
        registry: system::steam_registry(),
        bundled: plugin_install::bundled_dll(),
        game: &ProcessCheck,
        delete_later: &system::delete_on_reboot,
    };
    let show = |quiet: bool, text: &str, kind: MessageKind| {
        eprintln!("{text}");
        if !quiet {
            system::message_box(TITLE, text, kind);
        }
    };
    match command {
        Command::Gui => cli::EXIT_USAGE,
        Command::Version => {
            show(
                false,
                &format!("SpedHub-Launcher {VERSION}"),
                MessageKind::Info,
            );
            cli::EXIT_OK
        }
        Command::InstallPlugin {
            game_dir,
            report,
            quiet,
        } => {
            let mut choose = |reason: &str| -> Option<PathBuf> {
                if quiet {
                    return None;
                }
                system::message_box(TITLE, reason, MessageKind::Warning);
                system::pick_folder("Ordner von Euro Truck Simulator 2 wählen")
            };
            match cli::install_plugin(game_dir.as_deref(), &env, &mut choose) {
                Ok(InstallOutcome::Installed {
                    not_remembered,
                    game_dir,
                    source,
                    ..
                }) => {
                    // Für das Setup, das den Ordner als ursprünglicher Benutzer merkt
                    if source == GameSource::Argument
                        && let Some(file) = report
                        && let Err(e) = cli::write_game_dir_report(&file, &game_dir)
                    {
                        eprintln!("{e}");
                    }
                    if let Some(e) = not_remembered {
                        show(
                            quiet,
                            &format!(
                                "Das Plugin ist installiert, aber der Spielordner ließ sich nicht merken: {e}"
                            ),
                            MessageKind::Warning,
                        );
                    }
                    cli::EXIT_OK
                }
                Ok(InstallOutcome::Cancelled { reason }) => {
                    show(
                        quiet,
                        &format!(
                            "Das Plugin wurde nicht installiert: {reason}. Das geht später im Launcher über \"Plugin installieren\""
                        ),
                        MessageKind::Info,
                    );
                    cli::EXIT_CANCELLED
                }
                Err(e) => {
                    show(
                        quiet,
                        &format!("Das Plugin wurde nicht installiert: {e}"),
                        MessageKind::Error,
                    );
                    cli::EXIT_ERROR
                }
            }
        }
        Command::UninstallPlugin { quiet } => {
            let mut retry = |reason: &str| !quiet && system::ask_retry(TITLE, reason);
            match cli::uninstall_plugin(&env, &mut retry) {
                Ok(removed) => {
                    if let Some(note) = cli::removal_note(&removed) {
                        show(quiet, &note, MessageKind::Info);
                    }
                    cli::EXIT_OK
                }
                Err(e) => {
                    show(
                        quiet,
                        &format!("Das Plugin ließ sich nicht entfernen: {e}"),
                        MessageKind::Error,
                    );
                    cli::EXIT_ERROR
                }
            }
        }
        Command::RememberGameDir { game_dir, quiet } => {
            match cli::remember_game_dir(paths.as_ref(), &game_dir) {
                Ok(_) => cli::EXIT_OK,
                Err(e) => {
                    show(
                        quiet,
                        &format!("Der Spielordner ließ sich nicht merken: {e}"),
                        MessageKind::Warning,
                    );
                    cli::EXIT_ERROR
                }
            }
        }
    }
}

/// Stand des Plugins und wo das Spiel liegt.
struct PluginView {
    status: PluginStatus,
    game_dir: Option<PathBuf>,
    source: Option<GameSource>,
    /// Warum das Spiel nicht gefunden wurde
    problem: Option<String>,
    /// Gerade läuft eine Installation mit Adminrechten
    installing: bool,
}

impl PluginView {
    fn detect(settings: &Settings) -> PluginView {
        let found = plugin_install::locate_game(
            None,
            settings.game_dir.as_deref(),
            &system::steam_registry(),
        );
        let bundled = plugin_install::bundled_dll();
        match found {
            Ok((dir, source)) => PluginView {
                status: plugin_install::status(Some(&dir), bundled.as_deref()),
                game_dir: Some(dir),
                source: Some(source),
                problem: None,
                installing: false,
            },
            Err(problem) => PluginView {
                status: PluginStatus::GameNotFound,
                game_dir: None,
                source: None,
                problem: Some(problem),
                installing: false,
            },
        }
    }
}

/// Stand der Update-Prüfung.
enum UpdateState {
    Checking,
    /// Kein neueres Release
    Current,
    Available(Release),
    Downloading(Release),
    /// Prüfung ging nicht, wird nur leise angezeigt
    Failed(String),
}

/// Ergebnisse von Arbeiten, die nebenher laufen und keine anderen sperren.
enum Background {
    UpdateChecked(std::result::Result<Option<Release>, String>),
    /// Setup geladen, geprüft und gestartet, oder warum nicht
    UpdateStarted(std::result::Result<PathBuf, String>),
    /// Exit-Code des Launchers mit Adminrechten
    PluginInstalled(std::result::Result<i32, String>),
    /// Ergebnis des Ordnerdialogs. Mit `then_install` kam er von "Plugin installieren" und die
    /// Installation geht danach weiter.
    FolderChosen {
        dir: Option<PathBuf>,
        then_install: bool,
    },
    /// Vorschlag des Servers für das Kennzeichen beim Übernehmen, oder warum nicht
    PlateSuggested(std::result::Result<String, String>),
    /// Firmenprofil beim Server gemeldet, oder warum nicht
    ProfileReported {
        company_id: i64,
        name: String,
        result: std::result::Result<(), String>,
    },
    /// Eine Panic in einer dieser Arbeiten
    Failed(String),
}

/// Ergebnis einer Arbeit im Hintergrund.
enum Done {
    LoggedIn {
        token: String,
        user_name: String,
        version_warning: Option<String>,
    },
    Loaded {
        me: Me,
        company: Option<Company>,
        trucks: Vec<Truck>,
        /// Neue Warnung zur Version des Servers, `None`, wenn er nicht gefragt werden konnte
        version_warning: Option<Option<String>>,
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
    /// Lkw übernommen. Gibt es Betriebskosten-Buchungen, fragt der Launcher, ob eine davon
    /// der Kauf dieses Lkw war.
    Imported {
        text: String,
        truck_id: i64,
        plate: String,
        candidates: Vec<PurchaseCandidate>,
    },
    PasswordChanged,
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

/// Nach "Lkw übernehmen": Betriebskosten-Buchung als Kauf dieses Lkw verknüpfen?
struct LinkDialog {
    truck_id: i64,
    plate: String,
    candidates: Vec<PurchaseCandidate>,
    /// Gewählte Buchung
    chosen: Option<i64>,
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

    // Einstellungen: Passwort ändern
    pw_old: String,
    pw_new: String,
    pw_repeat: String,

    // Lkw übernehmen
    import_slots: Vec<SaveSlot>,
    import_slot: Option<usize>,
    import_trucks: Vec<SaveTruck>,
    import_plate: Option<String>,
    /// Kennzeichen des Lkw in der Firma, vorbelegt mit dem Vorschlag des Servers
    import_company_plate: String,

    /// Gewählter Bereich in der Seitenleiste
    view: View,
    /// Filter über dem Fuhrpark, `None` für alle
    fleet_filter: Option<TruckState>,

    returning: Option<ReturnDialog>,
    linking: Option<LinkDialog>,
    message: Option<Message>,
    busy: Option<String>,
    tx: Sender<Done>,
    rx: Receiver<Done>,

    /// Launcher und Server passen nicht zusammen
    version_warning: Option<String>,
    plugin: PluginView,
    update: UpdateState,
    /// Laufende oder geklappte Meldung des Firmenprofils, siehe [`ProfileReport`]
    profile_report: ProfileReport,
    bg_tx: Sender<Background>,
    bg_rx: Receiver<Background>,
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
        let (bg_tx, bg_rx) = channel();
        let plugin = PluginView::detect(&settings);
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
            pw_old: String::new(),
            pw_new: String::new(),
            pw_repeat: String::new(),
            import_slots: Vec::new(),
            import_slot: None,
            import_trucks: Vec::new(),
            import_plate: None,
            import_company_plate: String::new(),
            view: View::Fleet,
            fleet_filter: None,
            returning: None,
            linking: None,
            message: None,
            busy: None,
            tx,
            rx,
            version_warning: None,
            plugin,
            update: UpdateState::Checking,
            profile_report: ProfileReport::default(),
            bg_tx,
            bg_rx,
        };
        app.check_for_update(ctx);
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

    /// Name des gewählten Firmenprofils, wie das Spiel ihn anzeigt.
    fn profile_name(&self) -> Option<String> {
        self.settings
            .profile_dir
            .as_deref()
            .and_then(profile_name_of)
    }

    /// Schreibt plugin.json für die gewählte Firma und das gewählte Firmenprofil.
    fn write_plugin_config(&mut self) {
        let (Some(paths), Some(token), Some(company)) =
            (&self.paths, &self.token, self.settings.company_id)
        else {
            return;
        };
        let profile_dir = self.settings.profile_dir.as_deref();
        let log = self
            .ets2_dir
            .as_deref()
            .or_else(|| profile_dir.and_then(ets2_dir_of))
            .map(game_log);
        let name = self.profile_name();
        if let Err(e) = write_plugin_config(
            paths,
            &self.settings.server_url,
            token,
            company,
            name.as_deref(),
            log.as_deref(),
        ) {
            self.error(format!("plugin.json ließ sich nicht schreiben: {e}"));
        }
    }

    /// Meldet das gewählte Firmenprofil beim Server, wenn er ein anderes oder keins kennt.
    /// Läuft nebenher und sperrt nichts.
    fn report_profile(&mut self, ctx: &egui::Context) {
        let (Some(company), Some(me)) = (&self.company, &self.me) else {
            return;
        };
        let name = self.profile_name();
        let Some(name) = profile_to_report(company, me.user_id, name.as_deref()) else {
            return;
        };
        let company_id = company.id;
        if !self.profile_report.start(company_id, &name) {
            return;
        }
        let client = self.client();
        self.background(ctx, move || Background::ProfileReported {
            result: client
                .set_profile(company_id, Some(&name))
                .map_err(|e| e.to_string()),
            company_id,
            name,
        });
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
            let version_warning = client
                .health()
                .ok()
                .map(|h| version_warning(h.version.as_deref()));
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
                version_warning,
            })
        });
    }

    fn handle(&mut self, ctx: &egui::Context, done: Done) {
        self.busy = None;
        match done {
            Done::LoggedIn {
                token,
                user_name,
                version_warning,
            } => {
                self.version_warning = version_warning;
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
                version_warning,
            } => {
                if let Some(w) = version_warning {
                    self.version_warning = w;
                }
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
                    // Der Vorschlag für das Kennzeichen gehörte zur alten Firma
                    self.import_company_plate.clear();
                }
                self.write_plugin_config();
                self.report_profile(ctx);
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
                if self.import_company_plate.trim().is_empty() {
                    self.suggest_plate(ctx);
                }
            }
            Done::Imported {
                text,
                truck_id,
                plate,
                candidates,
            } => {
                // Der nächste Lkw bekommt einen neuen Vorschlag
                self.import_company_plate.clear();
                self.import_plate = None;
                self.suggest_plate(ctx);
                if candidates.is_empty() {
                    self.info(text);
                } else {
                    self.info(format!(
                        "{text}. Hast du ihn im Spiel gekauft? Dann verknüpfe im Fenster die Buchung des Kaufs"
                    ));
                    self.linking = Some(LinkDialog {
                        truck_id,
                        plate,
                        candidates,
                        chosen: None,
                    });
                }
                self.refresh(ctx);
            }
            Done::PasswordChanged => {
                self.pw_old.clear();
                self.pw_new.clear();
                self.pw_repeat.clear();
                self.info("Passwort geändert. Andere Anmeldungen mit diesem Konto sind abgemeldet, der Launcher und das Plugin bleiben angemeldet");
            }
            Done::LoggedOut => {
                self.token = None;
                self.version_warning = None;
                self.linking = None;
                self.pw_old.clear();
                self.pw_new.clear();
                self.pw_repeat.clear();
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

    // ------------------------------------------------------------------ Nebenher

    /// Startet eine Arbeit, die nebenher läuft und nichts sperrt.
    fn background(&self, ctx: &egui::Context, job: impl FnOnce() -> Background + Send + 'static) {
        let tx = self.bg_tx.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let done = catch_unwind(AssertUnwindSafe(job))
                .unwrap_or_else(|_| Background::Failed("Interner Fehler im Launcher".into()));
            let _ = tx.send(done);
            ctx.request_repaint();
        });
    }

    /// Fragt GitHub nach einer neueren Version. Fehler stören nicht.
    fn check_for_update(&mut self, ctx: &egui::Context) {
        self.update = UpdateState::Checking;
        self.background(ctx, || {
            Background::UpdateChecked(Updater::new(update::DEFAULT_API).check_current())
        });
    }

    /// Setup laden, Prüfsumme prüfen, starten. Danach schließt sich der Launcher, damit das
    /// Setup ihn ersetzen kann.
    fn start_update(&mut self, ctx: &egui::Context) {
        let release = match std::mem::replace(&mut self.update, UpdateState::Current) {
            UpdateState::Available(r) => r,
            other => {
                self.update = other;
                return;
            }
        };
        self.update = UpdateState::Downloading(release.clone());
        self.background(ctx, move || {
            let updater = Updater::new(update::DEFAULT_API);
            Background::UpdateStarted(update::download_and_start(
                &updater,
                &release,
                &update::download_dir(),
                &system::open_path,
            ))
        });
    }

    /// Startet den Launcher über UAC mit `--install-plugin` und wartet im Hintergrund auf ihn.
    ///
    /// Ist das Spiel nicht gefunden, wird der Ordner zuerst hier gewählt, ohne Adminrechte, und
    /// dann mit `--game-dir` übergeben. Der Launcher mit Adminrechten kann unter einem anderen
    /// Konto laufen, wenn im UAC-Dialog die Anmeldung eines Admins eingegeben wird. Ein dort
    /// gewählter Ordner landete in dessen `launcher.json` statt in unserer.
    fn install_plugin(&mut self, ctx: &egui::Context) {
        if self.plugin.game_dir.is_none() {
            self.info("Spiel nicht gefunden. Bitte den Ordner von Euro Truck Simulator 2 wählen");
            self.choose_game_folder(ctx, true);
            return;
        }
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => {
                self.error(format!("Der Launcher findet sich selbst nicht: {e}"));
                return;
            }
        };
        // Einen von Hand gewählten Ordner ausdrücklich mitgeben, sonst sucht der Launcher mit
        // Adminrechten selbst über Steam
        let dir = match self.plugin.source {
            Some(GameSource::Settings) | Some(GameSource::Argument) => self.plugin.game_dir.clone(),
            _ => None,
        };
        let params = cli::install_parameters(dir.as_deref());
        self.plugin.installing = true;
        self.info("Plugin wird installiert. Bitte die Nachfrage nach Adminrechten bestätigen");
        self.background(ctx, move || {
            Background::PluginInstalled(system::run_elevated_and_wait(&exe, &params))
        });
    }

    fn choose_game_folder(&mut self, ctx: &egui::Context, then_install: bool) {
        self.background(ctx, move || Background::FolderChosen {
            dir: system::pick_folder("Ordner von Euro Truck Simulator 2 wählen"),
            then_install,
        });
    }

    fn handle_background(&mut self, ctx: &egui::Context, done: Background) {
        match done {
            Background::UpdateChecked(Ok(Some(r))) => self.update = UpdateState::Available(r),
            Background::UpdateChecked(Ok(None)) => self.update = UpdateState::Current,
            Background::UpdateChecked(Err(e)) => self.update = UpdateState::Failed(e),
            Background::UpdateStarted(Ok(path)) => {
                self.info(format!(
                    "Prüfsumme stimmt, das Setup startet: {}. Der Launcher schließt sich",
                    path.display()
                ));
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Background::UpdateStarted(Err(e)) => {
                if let UpdateState::Downloading(r) =
                    std::mem::replace(&mut self.update, UpdateState::Current)
                {
                    self.update = UpdateState::Available(r);
                }
                self.error(format!("Update nicht installiert: {e}"));
            }
            Background::PluginInstalled(result) => {
                self.plugin.installing = false;
                // Den Spielordner hat die Oberfläche selbst gewählt und gemerkt, der Launcher
                // mit Adminrechten fragt nur, wenn er Steam anders als wir nicht findet
                self.plugin = PluginView::detect(&self.settings);
                match result {
                    Ok(cli::EXIT_OK) => self.info(format!(
                        "{}. Beim nächsten Start fragt das Spiel einmal, ob es Plugins laden darf, das mit OK bestätigen",
                        self.plugin.status.label()
                    )),
                    Ok(cli::EXIT_CANCELLED) => {
                        self.error("Das Plugin wurde nicht installiert, kein Spielordner gewählt")
                    }
                    Ok(code) => self.error(format!(
                        "Das Plugin wurde nicht installiert (Code {code}). Der Grund stand im Fenster davor"
                    )),
                    Err(e) => self.error(e),
                }
            }
            Background::PlateSuggested(result) => match result {
                // Was der Chef schon eingetragen hat, bleibt
                Ok(p) if self.import_company_plate.trim().is_empty() => {
                    self.import_company_plate = p;
                }
                Ok(_) => {}
                Err(e) => self.error(format!(
                    "Kein Vorschlag für das Kennzeichen vom Server ({e}). Bitte selbst eins eintragen"
                )),
            },
            Background::ProfileReported {
                company_id,
                name,
                result,
            } => {
                self.profile_report
                    .finished(company_id, &name, result.is_ok());
                match result {
                    Ok(()) => self.info(format!(
                        "Firmenprofil {name} beim Server gemeldet. Gebucht wird nur, was in diesem Profil passiert"
                    )),
                    Err(e) => self.error(format!(
                        "Das Firmenprofil {name} ließ sich dem Server nicht melden: {e}. Bis das klappt, lehnt der Server Aufträge aus diesem Profil ab. Der Launcher versucht es beim nächsten Aktualisieren wieder"
                    )),
                }
            }
            Background::Failed(e) => {
                self.plugin.installing = false;
                match std::mem::replace(&mut self.update, UpdateState::Current) {
                    UpdateState::Downloading(r) => self.update = UpdateState::Available(r),
                    UpdateState::Checking => self.update = UpdateState::Failed(e.clone()),
                    other => self.update = other,
                }
                self.error(e);
            }
            Background::FolderChosen {
                dir: None,
                then_install,
            } => {
                if then_install {
                    self.error("Das Plugin wurde nicht installiert, kein Spielordner gewählt");
                }
            }
            Background::FolderChosen {
                dir: Some(dir),
                then_install,
            } => match spedhub_launcher::steam::validate_game_dir(&dir) {
                Ok(game_dir) => {
                    self.settings.game_dir = Some(game_dir);
                    self.save_settings();
                    self.plugin = PluginView::detect(&self.settings);
                    if then_install && self.plugin.status.can_install() {
                        self.install_plugin(ctx);
                    } else {
                        self.info(format!(
                            "Spielordner gemerkt. {}",
                            self.plugin.status.label()
                        ));
                    }
                }
                Err(e) => self.error(e),
            },
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
                version_warning: login.version_warning(),
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
        let (truck_id, plate) = (truck.id, truck.plate.clone());
        self.run(ctx, "Lkw wird in den Spielstand geschrieben", move || {
            // Liegt der Bauplan nicht auf diesem Rechner, kommt er vom Server
            let package = package_for(&client, paths.as_ref(), truck_id, &plate)?;
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
            let package = package_for(&client, paths.as_ref(), truck_id, &plate).ok();
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
            let returned = return_truck(&client, truck_id, km, state.as_ref(), new_plate.as_deref())?;
            let r = returned.checkin;
            if let Some(p) = &paths {
                p.remove_lease(truck_id);
            }
            let renamed = match (&new_plate, &returned.plate_taken, &r.plate_ignored) {
                (_, Some(p), _) => format!(
                    ". Das Spiel hat ihm das Kennzeichen {p} gegeben, das ein anderer Firmen-Lkw trägt. Er behält {}",
                    r.plate
                ),
                (Some(_), None, Some(i)) => format!(
                    ". Das Spiel hat ihm das Kennzeichen {} gegeben, das sich nicht als eigenes setzen lässt ({}). Er behält {}",
                    i.plate, i.reason, r.plate
                ),
                (Some(p), None, None) => {
                    format!(". Neues Kennzeichen {p}, das alte merkt sich der Server")
                }
                (None, None, _) => String::new(),
            };
            let mut text = format!(
                "{plate} ist zurückgegeben, {} km gefahren{renamed}. Schaden {} %",
                r.driven_km,
                format_percent(r.damage)
            );
            if r.liability > 0 {
                text.push_str(&format!(
                    ". Haftung für {} Prozentpunkte neuen Schaden: {} Euro vom Lohnkonto",
                    format_percent(r.damage_new),
                    r.liability
                ));
            }
            if r.in_workshop {
                text.push_str(
                    ". Das ist über der Schadensgrenze, der Lkw steht jetzt in der Werkstatt",
                );
            }
            Ok(Done::Info(text))
        });
    }

    fn change_password(&mut self, ctx: &egui::Context) {
        if self.pw_new.len() < 8 {
            self.error("Das neue Passwort braucht mindestens 8 Zeichen");
            return;
        }
        if self.pw_new != self.pw_repeat {
            self.error("Die Wiederholung passt nicht zum neuen Passwort");
            return;
        }
        let client = self.client();
        let (old, new) = (self.pw_old.clone(), self.pw_new.clone());
        self.run(ctx, "Passwort wird geändert", move || {
            client.change_password(&old, &new)?;
            Ok(Done::PasswordChanged)
        });
    }

    /// Holt den Vorschlag des Servers für das Kennzeichen des nächsten übernommenen Lkw.
    fn suggest_plate(&mut self, ctx: &egui::Context) {
        let Some(company) = self.company.as_ref().filter(|c| c.is_chef()).map(|c| c.id) else {
            return;
        };
        let client = self.client();
        self.background(ctx, move || {
            Background::PlateSuggested(client.plate_suggestion(company).map_err(|e| e.to_string()))
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
        let company_plate = self.import_company_plate.trim().to_string();
        if company_plate.is_empty() {
            self.error("Bitte das Kennzeichen in der Firma eintragen");
            return;
        }
        let game = slot.game();
        let client = self.client();
        self.run(ctx, "Lkw wird übernommen", move || {
            let truck = new_truck_from_save(&game, &plate, &company_plate)?;
            let truck_id = client.add_truck(company, &truck)?;
            // Ohne Kandidaten (oder wenn die Abfrage scheitert) gibt es nichts zu verknüpfen
            let candidates = client.purchase_candidates(company).unwrap_or_default();
            Ok(Done::Imported {
                text: format!(
                    "{} ({}, {} km) ist jetzt im Fuhrpark. Beim Einsetzen bekommt er dieses Kennzeichen",
                    truck.plate, truck.model, truck.odometer_km
                ),
                truck_id,
                plate: truck.plate,
                candidates,
            })
        });
    }

    /// Die gewählte Betriebskosten-Buchung als Kauf des gerade übernommenen Lkw verknüpfen.
    fn link_purchase(&mut self, ctx: &egui::Context) {
        let (Some(dialog), Some(company)) = (&self.linking, self.company.as_ref().map(|c| c.id))
        else {
            return;
        };
        let Some(ledger_id) = dialog.chosen else {
            return;
        };
        let truck_id = dialog.truck_id;
        self.linking = None;
        let client = self.client();
        self.run(ctx, "Kauf wird verknüpft", move || {
            let l = client.link_purchase(company, ledger_id, truck_id)?;
            Ok(Done::Info(format!(
                "Verknüpft. Im Kassenbuch steht jetzt Lkw-Kauf {}",
                l.truck_plate
            )))
        });
    }

    // ------------------------------------------------------------------ Ansichten

    fn is_chef(&self) -> bool {
        self.company.as_ref().is_some_and(Company::is_chef)
    }

    /// Eigener Eintrag in der Mitgliederliste der gewählten Firma.
    fn my_member(&self) -> Option<&Member> {
        let me = self.me.as_ref()?.user_id;
        self.company
            .as_ref()?
            .members
            .iter()
            .find(|m| m.user_id == me)
    }

    fn choose_company(&mut self, ctx: &egui::Context, chosen: Option<i64>) {
        if chosen != self.settings.company_id {
            self.settings.company_id = chosen;
            self.save_settings();
            self.write_plugin_config();
            self.refresh(ctx);
        }
    }

    fn login_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        let width = 420.0_f32.min(ui.available_width());
        ui.add_space(SP_10);
        ui.vertical_centered(|ui| {
            ui.set_max_width(width);
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, CornerRadius::same(R_MD + 2), p.nav_active_bg);
            theme::paint_logo(ui.painter(), rect.shrink(9.0), p.accent, p.nav_active_bg);
            ui.add_space(SP_2);
            ui.label(text("SpedHub", semibold(FS_LG), p.text));
            ui.label(text(
                "Fuhrpark und Kasse deiner Spedition",
                regular(FS_SM),
                p.text_2,
            ));
            ui.add_space(SP_6);
            if self.update_notice(ui) {
                ui.add_space(SP_4);
            }
            theme::card(ui)
                .inner_margin(Margin::same(SP_6 as i8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                        theme::card_title(ui, "Anmelden", "Mit deinem Konto beim Server der Firma");
                        ui.add_space(SP_3);
                        theme::field_label(ui, "Server");
                        ui.add(
                            theme::text_field(&mut self.settings.server_url)
                                .desired_width(f32::INFINITY),
                        );
                        ui.add_space(SP_1);
                        theme::field_label(ui, "Name");
                        ui.add(theme::text_field(&mut self.name).desired_width(f32::INFINITY));
                        ui.add_space(SP_1);
                        theme::field_label(ui, "Passwort");
                        ui.add(
                            theme::text_field(&mut self.password)
                                .password(true)
                                .desired_width(f32::INFINITY),
                        );
                        ui.add_space(SP_4);
                        let ready = !self.name.trim().is_empty()
                            && !self.password.is_empty()
                            && self.busy.is_none();
                        ui.horizontal(|ui| {
                            if ui
                                .add_enabled(ready, ActionButton::primary("Anmelden"))
                                .clicked()
                            {
                                self.login(ui.ctx(), false);
                            }
                            if ui
                                .add_enabled(ready, ActionButton::secondary("Konto anlegen"))
                                .clicked()
                            {
                                self.login(ui.ctx(), true);
                            }
                        });
                    });
                });
            ui.add_space(SP_4);
            self.plugin_card(ui);
            ui.add_space(SP_6);
        });
    }

    /// Seitenleiste: Logo und Firma, Bereiche, unten Nutzer, Version und Server.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, CornerRadius::same(R_SM + 2), p.nav_active_bg);
            theme::paint_logo(ui.painter(), rect.shrink(6.0), p.accent, p.nav_active_bg);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(text("SpedHub", semibold(FS_MD), p.nav_active));
                let company = self
                    .company
                    .as_ref()
                    .map_or("Keine Firma gewählt", |c| c.name.as_str());
                ui.add(egui::Label::new(text(company, regular(FS_XS), p.nav_text)).truncate());
            });
        });
        ui.add_space(SP_6);
        ui.spacing_mut().item_spacing.y = 2.0;
        for view in View::ALL {
            if view == View::Import && !self.is_chef() {
                continue;
            }
            if theme::nav_item(ui, view.icon(), view.nav_label(), self.view == view).clicked() {
                self.view = view;
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.spacing_mut().item_spacing.y = SP_1;
            let server = self
                .settings
                .server_url
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_end_matches('/');
            ui.add(
                egui::Label::new(text(format!("Server {server}"), regular(FS_XS), p.nav_text))
                    .truncate(),
            )
            .on_hover_text(&self.settings.server_url);
            let (version, color, hover) = match &self.update {
                UpdateState::Checking => (
                    format!("Version {VERSION}"),
                    p.nav_text,
                    Some("Suche nach Updates".to_string()),
                ),
                UpdateState::Current => (format!("Version {VERSION}, aktuell"), p.nav_text, None),
                UpdateState::Failed(e) => (
                    format!("Version {VERSION}"),
                    p.nav_text,
                    Some(format!("Update-Prüfung: {e}")),
                ),
                UpdateState::Available(r) | UpdateState::Downloading(r) => (
                    format!("Version {VERSION}, {} verfügbar", r.version),
                    p.accent,
                    None,
                ),
            };
            let label = ui.add(egui::Label::new(text(version, regular(FS_XS), color)).truncate());
            if let Some(h) = hover {
                label.on_hover_text(h);
            }
            ui.add_space(SP_2);
            self.user_block(ui);
            ui.add_space(SP_1);
            let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
            ui.painter()
                .rect_filled(rect, CornerRadius::ZERO, p.nav_active_bg);
        });
    }

    /// Nutzer unten in der Seitenleiste, mit Menü: Firma wechseln, Abmelden.
    fn user_block(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        let Some(me) = self.me.clone() else {
            return;
        };
        let role = match self.company.as_ref() {
            Some(c) if c.is_chef() => "Chef",
            Some(_) => "Fahrer",
            None => "ohne Firma",
        };
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::Role::Button, true, format!("{}, {role}", me.name))
        });
        if response.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(R_SM), p.nav_active_bg);
        }
        let painter = ui.painter();
        let center = pos2(rect.left() + 8.0 + 16.0, rect.center().y);
        painter.circle_filled(center, 16.0, p.accent);
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            theme::initials(&me.name),
            semibold(FS_SM),
            p.nav_bg,
        );
        let x = rect.left() + 8.0 + 32.0 + 10.0;
        painter.text(
            pos2(x, rect.center().y - 9.0),
            egui::Align2::LEFT_CENTER,
            &me.name,
            semibold(FS_BASE),
            p.nav_active,
        );
        painter.text(
            pos2(x, rect.center().y + 9.0),
            egui::Align2::LEFT_CENTER,
            role,
            regular(FS_XS),
            p.nav_text,
        );
        theme::paint_icon(
            painter,
            Rect::from_center_size(
                pos2(rect.right() - 16.0, rect.center().y),
                Vec2::splat(16.0),
            ),
            Icon::ChevronDown,
            p.nav_text,
        );
        let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
        let companies = me.companies.clone();
        let mut chosen = self.settings.company_id;
        let mut logout = false;
        let idle = self.busy.is_none();
        egui::Popup::menu(&response)
            .align(egui::RectAlign::TOP_START)
            .width(rect.width().max(220.0))
            .show(|ui| {
                ui.set_min_width(200.0);
                ui.spacing_mut().item_spacing.y = 2.0;
                if companies.len() > 1 {
                    ui.label(text(
                        "Firma wechseln",
                        semibold(FS_XS),
                        theme::palette(ui).text_2,
                    ));
                    for c in &companies {
                        let role = if c.role == "chef" { "Chef" } else { "Fahrer" };
                        if ui
                            .selectable_label(chosen == Some(c.id), format!("{} ({role})", c.name))
                            .clicked()
                        {
                            chosen = Some(c.id);
                            ui.close();
                        }
                    }
                    ui.separator();
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Einstellungen"))
                    .clicked()
                {
                    self.view = View::Settings;
                    ui.close();
                }
                if ui
                    .add_enabled(idle, egui::Button::new("Abmelden"))
                    .clicked()
                {
                    logout = true;
                    ui.close();
                }
            });
        self.choose_company(ui.ctx(), chosen);
        if logout {
            self.logout(ui.ctx());
        }
    }

    /// Kopf, Hinweise, Kennzahlen und der gewählte Bereich.
    fn content(&mut self, ui: &mut egui::Ui) {
        let subtitle = self.subtitle();
        let idle = self.busy.is_none();
        let mut refresh = false;
        theme::page_header(ui, self.view.title(), &subtitle, |ui| {
            if ui
                .add(ActionButton::primary("Spiel starten").icon(Icon::Play))
                .clicked()
            {
                match system::start_game() {
                    Ok(()) => self.info("Steam startet Euro Truck Simulator 2"),
                    Err(e) => self.error(e),
                }
            }
            if ui
                .add_enabled(
                    idle,
                    ActionButton::secondary("Aktualisieren").icon(Icon::Refresh),
                )
                .clicked()
            {
                refresh = true;
            }
        });
        if refresh {
            self.refresh(ui.ctx());
        }
        ui.add_space(SP_5);
        self.notices(ui);
        self.kpis(ui);
        ui.add_space(SP_2);
        match self.view {
            View::Fleet => self.fleet_view(ui),
            View::Money => self.money_view(ui),
            View::Import => self.import_view(ui),
            View::Settings => self.settings_view(ui),
        }
    }

    /// Eine Zeile Nebentext unter dem Titel.
    fn subtitle(&self) -> String {
        match self.view {
            View::Fleet => match &self.company {
                None => "Noch keine Firma".into(),
                Some(c) => {
                    let active: Vec<&Truck> =
                        self.trucks.iter().filter(|t| t.retired.is_none()).collect();
                    let count = |s: TruckState| active.iter().filter(|t| t.state() == s).count();
                    format!(
                        "{}: {} Lkw, {} unterwegs, {} in der Werkstatt",
                        c.name,
                        active.len(),
                        count(TruckState::Leased),
                        count(TruckState::Workshop)
                    )
                }
            },
            View::Money => "Firmenprofil wählen und Geld mit dem Spielstand abgleichen".into(),
            View::Import => {
                "Einen Lkw aus deinem Spielstand in den Fuhrpark der Firma holen".into()
            }
            View::Settings => "Konto, Passwort, Plugin und Version".into(),
        }
    }

    /// Warnungen als Hinweiskarten: Update, Version, Plugin, Firmenprofil.
    fn notices(&mut self, ui: &mut egui::Ui) {
        if self.update_notice(ui) {
            ui.add_space(SP_3);
        }
        if let Some(w) = self.version_warning.clone() {
            theme::notice(
                ui,
                Tone::Warning,
                "Launcher und Server passen nicht zusammen",
                &w,
                |_| {},
            );
            ui.add_space(SP_3);
        }
        let status = self.plugin.status;
        let problem = !matches!(status, PluginStatus::Installed | PluginStatus::NoBundledDll);
        // In den Einstellungen steht das Plugin in seiner eigenen Karte
        if (self.plugin.installing || problem) && self.view != View::Settings {
            let body = match status {
                PluginStatus::Outdated => {
                    "Im Spiel liegt eine andere Fassung als die mitgelieferte. Bitte neu installieren"
                        .to_string()
                }
                PluginStatus::Missing => {
                    "Ohne Plugin meldet das Spiel keine Aufträge, Maut und Strafen an den Server"
                        .to_string()
                }
                PluginStatus::GameNotFound => self.plugin.problem.clone().unwrap_or_else(|| {
                    "Der Ordner von Euro Truck Simulator 2 ist nicht bekannt".to_string()
                }),
                _ => String::new(),
            };
            theme::notice(ui, Tone::Warning, status.label(), &body, |ui| {
                self.plugin_actions(ui);
            });
            ui.add_space(SP_3);
        }
        if self.settings.profile_dir.is_none() {
            let elsewhere = self.view != View::Money;
            theme::notice(
                ui,
                Tone::Warning,
                "Kein Firmenprofil gewählt",
                NO_PROFILE,
                |ui| {
                    if elsewhere
                        && ui
                            .add(ActionButton::secondary("Firmenprofil wählen"))
                            .clicked()
                    {
                        self.view = View::Money;
                    }
                },
            );
            ui.add_space(SP_3);
        }
        if self.profile().is_some_and(|p| p.steam_cloud) {
            theme::notice(
                ui,
                Tone::Warning,
                "Firmenprofil mit Steam Cloud",
                STEAM_WARNING,
                |_| {},
            );
            ui.add_space(SP_3);
        }
    }

    /// Hinweis auf eine neue Version. `true`, wenn er angezeigt wurde.
    fn update_notice(&mut self, ui: &mut egui::Ui) -> bool {
        let (release, downloading) = match &self.update {
            UpdateState::Available(r) => (r.clone(), false),
            UpdateState::Downloading(r) => (r.clone(), true),
            _ => return false,
        };
        theme::notice(
            ui,
            Tone::Info,
            &format!("Version {} verfügbar", release.version),
            &format!("Du hast {VERSION}."),
            |ui| {
                if downloading {
                    ui.spinner();
                    ui.label("Setup wird geladen und geprüft");
                } else if ui
                    .add(
                        ActionButton::primary("Herunterladen und installieren")
                            .icon(Icon::Download),
                    )
                    .clicked()
                {
                    self.start_update(ui.ctx());
                }
                if !release.html_url.is_empty() {
                    ui.hyperlink_to("Was ist neu?", &release.html_url);
                }
            },
        );
        true
    }

    /// Kennzahl-Kacheln: Lohnkonto, Firmenkasse, Plugin, Firmenprofil.
    fn kpis(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        let minus = |n: i64| (n < 0).then_some(p.danger);
        let mut tiles = Vec::new();
        if let Some(m) = self.my_member() {
            let note = if m.open_claim > 0 {
                format!("offene Lohnforderung {}", theme::euro(m.open_claim))
            } else {
                format!("Level {}, {}", m.game_level, stage_text(m.stage))
            };
            tiles.push(Kpi {
                icon: Icon::Wallet,
                tone: Tone::Primary,
                label: "Dein Lohnkonto",
                value: theme::euro(m.wallet),
                value_color: minus(m.wallet),
                small_value: false,
                note,
            });
        }
        if let Some(c) = &self.company {
            let count = self.trucks.iter().filter(|t| t.retired.is_none()).count();
            tiles.push(Kpi {
                icon: Icon::Money,
                tone: Tone::Success,
                label: "Firmenkasse",
                value: theme::euro(c.balance),
                value_color: minus(c.balance),
                small_value: false,
                note: format!("{count} Lkw im Fuhrpark"),
            });
        }
        let (state, tone) = plugin_state(self.plugin.status);
        let note = match (&self.plugin.game_dir, &self.plugin.problem) {
            (Some(dir), _) => dir.display().to_string(),
            (None, Some(problem)) => problem.clone(),
            (None, None) => String::new(),
        };
        tiles.push(Kpi {
            icon: Icon::Plugin,
            tone,
            label: "Plugin",
            value: state.into(),
            value_color: Some(tone.strong(p)),
            small_value: true,
            note,
        });
        let (value, tone, note) = match self.profile() {
            Some(pr) if pr.steam_cloud => (
                pr.name.clone(),
                Tone::Warning,
                "mit Steam Cloud, besser ein anderes".to_string(),
            ),
            Some(pr) => (pr.name.clone(), Tone::Info, "ohne Steam Cloud".to_string()),
            None => (
                "nicht gewählt".to_string(),
                Tone::Warning,
                "ohne Profil meldet das Plugin nichts".to_string(),
            ),
        };
        tiles.push(Kpi {
            icon: Icon::Profile,
            tone,
            label: "Firmenprofil",
            value,
            value_color: None,
            small_value: true,
            note,
        });
        let (cols, width) = theme::grid_columns(ui.available_width(), 200.0, SP_4);
        for row in tiles.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = SP_4;
                for kpi in row {
                    theme::column(ui, width, |ui| theme::kpi_tile(ui, kpi));
                }
            });
            ui.add_space(SP_2);
        }
    }

    fn fleet_view(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        let Some(company) = self.company.clone() else {
            theme::empty_state(
                ui,
                Icon::Building,
                "Du bist noch in keiner Firma. Gründe eine im Dashboard oder tritt mit einem Einladungscode bei",
                |_| {},
            );
            return;
        };
        let my_id = self.me.as_ref().map(|m| m.user_id);
        let limit = company.damage_limit.unwrap_or(DEFAULT_DAMAGE_LIMIT) as f64;
        let chef = company.is_chef();
        // Ausgemusterte und verkaufte Lkw stehen unten für sich, ohne Knöpfe
        let trucks = self.trucks.clone();
        let (active, retired): (Vec<&Truck>, Vec<&Truck>) =
            trucks.iter().partition(|t| t.retired.is_none());
        let count = |s: TruckState| active.iter().filter(|t| t.state() == s).count();
        ui.horizontal_wrapped(|ui| {
            for (label, filter) in [
                ("Alle", None),
                ("Frei", Some(TruckState::Free)),
                ("Unterwegs", Some(TruckState::Leased)),
                ("Werkstatt", Some(TruckState::Workshop)),
            ] {
                let n = filter.map_or(active.len(), count);
                if theme::filter_pill(ui, label, n, self.fleet_filter == filter).clicked() {
                    self.fleet_filter = filter;
                }
            }
        });
        ui.add_space(SP_3);
        let shown: Vec<&Truck> = active
            .iter()
            .copied()
            .filter(|t| self.fleet_filter.is_none_or(|f| t.state() == f))
            .collect();
        if active.is_empty() {
            let sentence = if chef {
                "Noch kein Lkw im Fuhrpark. Übernimm den ersten aus deinem Spielstand"
            } else {
                "Noch kein Lkw im Fuhrpark. Der Chef legt Lkw an"
            };
            theme::empty_state(ui, Icon::Truck, sentence, |ui| {
                if chef
                    && ui
                        .add(ActionButton::primary("Lkw übernehmen").icon(Icon::Import))
                        .clicked()
                {
                    self.view = View::Import;
                }
            });
        } else if shown.is_empty() {
            theme::empty_state(ui, Icon::Truck, "Kein Lkw mit diesem Status", |_| {});
        }
        let mut action: Option<(TruckAction, Truck)> = None;
        let (cols, width) = theme::grid_columns(ui.available_width(), CARD_MIN_WIDTH, SP_4);
        for (i, row) in shown.chunks(cols).enumerate() {
            // Karten einer Reihe sind gleich hoch, die Knöpfe stehen unten auf einer Linie.
            // Die Höhe kommt aus dem letzten Bild.
            let row_id = egui::Id::unique(("fuhrpark-reihe", i, cols));
            let target: f32 = ui.data(|d| d.get_temp(row_id)).unwrap_or(0.0);
            let mut tallest = 0.0_f32;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = SP_4;
                for t in row {
                    theme::column(ui, width, |ui| {
                        let (a, body) = self.truck_card(ui, t, my_id, limit, target);
                        tallest = tallest.max(body);
                        if let Some(a) = a {
                            action = Some((a, (*t).clone()));
                        }
                    });
                }
            });
            if (tallest - target).abs() > 0.5 {
                ui.data_mut(|d| d.insert_temp(row_id, tallest));
                ui.ctx().request_repaint();
            }
            ui.add_space(SP_2);
        }
        if !retired.is_empty() {
            ui.add_space(SP_2);
            egui::CollapsingHeader::new(text(
                format!("Ausgemustert und verkauft ({})", retired.len()),
                semibold(FS_BASE),
                p.text_2,
            ))
            .default_open(false)
            .show(ui, |ui| {
                ui.multiply_opacity(0.8);
                theme::card(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for (i, t) in retired.iter().enumerate() {
                        if i > 0 {
                            theme::divider(ui);
                        }
                        row(ui, 30.0, |ui| {
                            theme::plate(ui, &t.plate, PlateSize::Small);
                            ui.label(text(
                                theme::model_name(&t.model),
                                regular(FS_BASE),
                                p.text_2,
                            ));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                let why = t.retired.as_ref().map(|r| r.text()).unwrap_or_default();
                                ui.label(text(why, regular(FS_SM), p.text_3));
                                ui.label(text(theme::km(t.odometer_km), regular(FS_SM), p.text_3));
                            });
                        });
                    }
                });
            });
        }
        ui.add_space(SP_4);
        theme::notice(
            ui,
            Tone::Info,
            "Vor dem Schreiben in den Spielstand",
            UNCONFIRMED,
            |_| {},
        );
        match action {
            Some((TruckAction::Checkout, t)) => self.checkout(ui.ctx(), &t),
            Some((TruckAction::Write, t)) => self.write_to_save(ui.ctx(), &t),
            Some((TruckAction::Return, t)) => self.start_return(ui.ctx(), &t),
            None => {}
        }
    }

    /// Karte eines Lkw: Kennzeichen, Modell und Plakette, Kilometer und Schaden, Fahrer, eine
    /// Hauptaktion.
    fn truck_card(
        &self,
        ui: &mut egui::Ui,
        t: &Truck,
        my_id: Option<i64>,
        limit: f64,
        body_height: f32,
    ) -> (Option<TruckAction>, f32) {
        let p = theme::palette(ui);
        let mine = t
            .leased_by
            .as_ref()
            .is_some_and(|l| Some(l.user_id) == my_id);
        let state = t.state();
        let (status, tone) = match state {
            TruckState::Free => ("frei", Tone::Success),
            TruckState::Workshop => ("in der Werkstatt", Tone::Danger),
            TruckState::Leased if t.driving => ("wird gefahren", Tone::Warning),
            TruckState::Leased if mine => ("bei dir", Tone::Warning),
            TruckState::Leased => ("verliehen", Tone::Warning),
            TruckState::Retired => ("ausgemustert", Tone::Neutral),
        };
        let now = unix_seconds(SystemTime::now()) as i64;
        let idle = self.busy.is_none();
        let mut action = None;
        let mut body = 0.0;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let top = ui.min_rect().top();
            // Kopf: Kennzeichen, Modell, Plakette
            row(ui, 36.0, |ui| {
                theme::plate(ui, &t.plate, PlateSize::Large);
                ui.add_space(SP_1);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    theme::badge(ui, status, tone);
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let model = egui::Label::new(text(
                            theme::model_name(&t.model),
                            semibold(FS_BASE),
                            p.text,
                        ))
                        .truncate();
                        if t.old_plates.is_empty() {
                            ui.add(model).on_hover_text(&t.model);
                        } else {
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.add(model).on_hover_text(&t.model);
                                ui.add(
                                    egui::Label::new(text(
                                        format!("früher {}", t.old_plates.join(", ")),
                                        regular(FS_XS),
                                        p.text_3,
                                    ))
                                    .truncate(),
                                );
                            });
                        }
                    });
                });
            });
            ui.add_space(SP_3);
            // Kennwerte: Kilometer und Schaden
            let half = (ui.available_width() - SP_4) / 2.0;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = SP_4;
                theme::column(ui, half, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(text("Kilometerstand", regular(FS_XS), p.text_2));
                    ui.label(text(theme::km(t.odometer_km), semibold(FS_MD), p.text));
                });
                theme::column(ui, half, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(text("Schaden", regular(FS_XS), p.text_2));
                    ui.allocate_ui_with_layout(
                        vec2(ui.available_width(), 22.0),
                        Layout::right_to_left(Align::Center),
                        |ui| {
                            let color = match theme::damage_tone(t.damage, limit) {
                                Tone::Danger => p.danger,
                                _ => p.text,
                            };
                            ui.label(text(
                                format!("{} %", format_percent(t.damage)),
                                semibold(FS_BASE),
                                color,
                            ));
                            let bar = ui.available_width().max(24.0);
                            theme::damage_bar(ui, t.damage, limit, bar).on_hover_text(format!(
                                "Schadensgrenze {} %, darüber kommt der Lkw in die Werkstatt",
                                format_percent(limit)
                            ));
                        },
                    );
                    if t.damage_unfixable > 0.0 {
                        ui.label(text(
                            format!("dauerhaft {} %", format_percent(t.damage_unfixable)),
                            regular(FS_XS),
                            p.text_3,
                        ));
                    }
                });
            });
            // Fahrer, wenn verliehen
            if let Some(l) = &t.leased_by {
                ui.add_space(SP_3);
                theme::inset(ui).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.spacing_mut().item_spacing.y = SP_1;
                    row(ui, 26.0, |ui| {
                        theme::avatar(ui, &l.name, 26.0, p.primary_soft, p.primary);
                        let who = if mine { "Du".to_string() } else { l.name.clone() };
                        ui.label(text(who, semibold(FS_BASE), p.text));
                        ui.add(
                            egui::Label::new(text(
                                format!("ausgeliehen {}", age_text(now - l.since)),
                                regular(FS_SM),
                                p.text_2,
                            ))
                            .truncate(),
                        );
                    });
                    if t.driving {
                        row(ui, 20.0, |ui| {
                            theme::icon(ui, Icon::Speed, 18.0, p.warning);
                            let speed = match t.speed_kmh {
                                Some(v) => format!("fährt gerade, {v:.0} km/h"),
                                None => "fährt gerade".to_string(),
                            };
                            ui.label(text(speed, regular(FS_SM), p.text));
                        });
                    }
                });
            }
            if t.in_workshop {
                ui.add_space(SP_3);
                let cost = if t.repair_cost > 0 {
                    format!(". Reparatur {}", theme::euro(t.repair_cost))
                } else {
                    String::new()
                };
                ui.add(
                    egui::Label::new(text(
                        format!(
                            "Über der Schadensgrenze. Ausleihen geht erst, wenn der Chef ihn im Dashboard reparieren lässt{cost}"
                        ),
                        regular(FS_SM),
                        p.danger,
                    ))
                    .wrap(),
                );
            }
            body = ui.min_rect().bottom() - top;
            if body_height > body {
                ui.add_space(body_height - body);
            }
            ui.add_space(SP_3);
            theme::divider(ui);
            ui.add_space(SP_3);
            row(ui, CONTROL_HEIGHT, |ui| match state {
                TruckState::Free => {
                    if ui
                        .add_enabled(idle, ActionButton::primary("Ausleihen"))
                        .on_disabled_hover_text("Bitte warten")
                        .clicked()
                    {
                        action = Some(TruckAction::Checkout);
                    }
                }
                TruckState::Workshop => {
                    // Ein Lkw in der Werkstatt lässt sich erst nach der Reparatur ausleihen
                    ui.add_enabled(false, ActionButton::primary("Ausleihen"))
                        .on_disabled_hover_text(
                            "In der Werkstatt. Ausleihen geht erst, wenn der Chef ihn im Dashboard reparieren lässt",
                        );
                }
                TruckState::Leased if mine => {
                    if ui
                        .add_enabled(idle, ActionButton::primary("Zurückgeben"))
                        .clicked()
                    {
                        action = Some(TruckAction::Return);
                    }
                    if ui
                        .add_enabled(
                            idle,
                            ActionButton::secondary("In Spielstand schreiben").icon(Icon::Save),
                        )
                        .clicked()
                    {
                        action = Some(TruckAction::Write);
                    }
                }
                TruckState::Leased => {
                    let name = t.leased_by.as_ref().map_or("", |l| l.name.as_str());
                    ui.label(text(
                        format!("Zurückgeben kann nur {name}"),
                        regular(FS_SM),
                        p.text_3,
                    ));
                }
                TruckState::Retired => {}
            });
        });
        (action, body)
    }

    fn money_view(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_width();
        if theme::grid_columns(full, 360.0, SP_4).0 >= 2 {
            let left = ((full - SP_4) * 0.58).floor();
            let right = full - SP_4 - left;
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = SP_4;
                theme::column(ui, left, |ui| {
                    self.profile_card(ui);
                    self.sync_card(ui);
                });
                theme::column(ui, right, |ui| self.wallet_card(ui));
            });
        } else {
            self.profile_card(ui);
            self.sync_card(ui);
            self.wallet_card(ui);
        }
        ui.add_space(SP_2);
        theme::notice(ui, Tone::Info, "Vor dem Abgleich", UNCONFIRMED, |_| {});
    }

    fn profile_card(&mut self, ui: &mut egui::Ui) {
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(
                ui,
                "Firmenprofil",
                "Das Profil im Spiel, in dem du für die Firma fährst. Plugin und Server zählen nur, was dort passiert",
            );
            ui.add_space(SP_3);
            theme::field_label(ui, "Profil");
            ui.horizontal(|ui| {
                let current = self
                    .profile()
                    .map_or_else(|| "Profil wählen".to_string(), profile_label);
                let mut chosen = self.settings.profile_dir.clone();
                egui::ComboBox::from_id_salt("profil")
                    .selected_text(current)
                    .width((ui.available_width() - 190.0).max(160.0))
                    .show_ui(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
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
                    // Das Plugin meldet nur aus dem Firmenprofil, Plugin und Server müssen es wissen
                    self.write_plugin_config();
                    self.report_profile(ui.ctx());
                }
                if ui
                    .add(ActionButton::secondary("Profile neu suchen").icon(Icon::Refresh))
                    .clicked()
                {
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
                theme::help_text(ui, &place);
            }
        });
        ui.add_space(SP_4);
    }

    fn sync_card(&mut self, ui: &mut egui::Ui) {
        let Some(advance) = self.company.as_ref().map(|c| c.advance) else {
            return;
        };
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(
                ui,
                "Geld abgleichen",
                &format!(
                    "Liest Kontostand und Erfahrung aus dem neuesten Spielstand und legt einen neuen Speicherplatz mit Lohnkonto plus {} Vorschuss an",
                    theme::euro(advance)
                ),
            );
            ui.add_space(SP_3);
            let ready = self.busy.is_none() && self.profile().is_some();
            if ui
                .add_enabled(ready, ActionButton::primary("Geld abgleichen").icon(Icon::Money))
                .on_disabled_hover_text("Bitte zuerst ein Firmenprofil wählen")
                .clicked()
            {
                self.sync_money(ui.ctx());
            }
        });
        ui.add_space(SP_4);
    }

    fn wallet_card(&mut self, ui: &mut egui::Ui) {
        let (Some(m), Some(company)) = (self.my_member().cloned(), self.company.clone()) else {
            return;
        };
        let p = theme::palette(ui);
        let minus = |n: i64| (n < 0).then_some(p.danger);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, "Dein Lohnkonto", "");
            ui.add_space(SP_2);
            theme::key_value(ui, "Lohnkonto", &theme::euro(m.wallet), minus(m.wallet));
            if m.open_claim > 0 {
                theme::key_value(
                    ui,
                    "Offene Lohnforderung",
                    &theme::euro(m.open_claim),
                    Some(p.danger),
                );
            }
            theme::key_value(ui, "Level im Spiel", &m.game_level.to_string(), None);
            theme::key_value(ui, "Firmenkilometer", &theme::km(m.company_km), None);
            theme::key_value(ui, "Lohnstufe", &stage_text(m.stage), None);
            theme::key_value(
                ui,
                "Lohn je Kilometer",
                &format!("{} Cent", m.cents_per_km),
                None,
            );
            ui.add_space(SP_1);
            theme::divider(ui);
            ui.add_space(SP_1);
            theme::key_value(
                ui,
                "Firmenkasse",
                &theme::euro(company.balance),
                minus(company.balance),
            );
            theme::key_value(
                ui,
                "Vorschuss beim Abgleich",
                &theme::euro(company.advance),
                None,
            );
        });
        ui.add_space(SP_4);
    }

    fn import_view(&mut self, ui: &mut egui::Ui) {
        if !self.is_chef() {
            return;
        }
        let Some(profile) = self.profile().map(|p| p.dir.clone()) else {
            theme::empty_state(ui, Icon::Profile, "Bitte zuerst ein Profil wählen", |ui| {
                if ui
                    .add(ActionButton::secondary("Firmenprofil wählen"))
                    .clicked()
                {
                    self.view = View::Money;
                }
            });
            return;
        };
        if self.import_slots.is_empty() {
            self.import_slots = list_slots(&profile);
        }
        if self.import_slots.is_empty() {
            theme::empty_state(
                ui,
                Icon::Save,
                "Im Profil gibt es noch keinen Spielstand",
                |_| {},
            );
            return;
        }
        let width = ui.available_width().min(760.0);
        theme::column(ui, width, |ui| {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                theme::card_title(
                    ui,
                    "Lkw in den Fuhrpark übernehmen",
                    "Danach gehört der Lkw der Firma und alle Fahrer können ihn ausleihen",
                );
                ui.add_space(SP_4);
                theme::field_label(ui, "Aus welchem Spielstand?");
                let current = self
                    .import_slot
                    .and_then(|i| self.import_slots.get(i))
                    .map_or_else(|| "Spielstand wählen".to_string(), SaveSlot::label);
                let mut chosen = self.import_slot;
                egui::ComboBox::from_id_salt("spielstand")
                    .selected_text(current)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (i, s) in self.import_slots.iter().enumerate() {
                            ui.selectable_value(&mut chosen, Some(i), s.label());
                        }
                    });
                if chosen != self.import_slot {
                    self.import_slot = chosen;
                    self.import_trucks.clear();
                    self.load_save_trucks(ui.ctx());
                }
                if !self.import_trucks.is_empty() {
                    ui.add_space(SP_3);
                    theme::field_label(ui, "Welcher Lkw?");
                    let mut picked = None;
                    for t in &self.import_trucks {
                        let selected = self.import_plate.as_deref() == Some(t.plate.as_str());
                        let model = t
                            .model
                            .as_deref()
                            .map_or_else(|| "unbekanntes Modell".to_string(), theme::model_name);
                        let p = theme::palette(ui);
                        let response = theme::choice_row(ui, &t.plate, selected, |ui| {
                            theme::plate(ui, &t.plate, PlateSize::Small);
                            ui.label(text(model, regular(FS_BASE), p.text));
                            if t.active {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    theme::badge(ui, "fährt gerade", Tone::Warning);
                                });
                            }
                        });
                        if response.clicked() {
                            picked = Some(t.plate.clone());
                        }
                    }
                    if picked.is_some() {
                        self.import_plate = picked;
                    }
                }
                ui.add_space(SP_3);
                theme::field_label(ui, "Kennzeichen in der Firma");
                ui.add(
                    theme::text_field(&mut self.import_company_plate)
                        .char_limit(16)
                        .desired_width(180.0),
                );
                theme::help_text(
                    ui,
                    "Mit diesem Kennzeichen setzt der Launcher den Lkw in die Spielstände der Fahrer. Vorgeschlagen ist das nächste freie der Firma. Erlaubt sind A bis Z, Ziffern, Leerzeichen, Punkt und Bindestrich",
                );
                ui.add_space(SP_4);
                let ready = self.import_plate.is_some()
                    && !self.import_company_plate.trim().is_empty()
                    && self.busy.is_none();
                if ui
                    .add_enabled(ready, ActionButton::primary("Lkw übernehmen").icon(Icon::Import))
                    .clicked()
                {
                    self.import_truck(ui.ctx());
                }
            });
        });
    }

    fn settings_view(&mut self, ui: &mut egui::Ui) {
        let full = ui.available_width();
        if theme::grid_columns(full, 360.0, SP_4).0 >= 2 {
            let half = ((full - SP_4) / 2.0).floor();
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = SP_4;
                theme::column(ui, half, |ui| {
                    self.account_card(ui);
                    self.password_card(ui);
                });
                theme::column(ui, half, |ui| {
                    self.plugin_card(ui);
                    ui.add_space(SP_4);
                    self.version_card(ui);
                });
            });
        } else {
            self.account_card(ui);
            self.password_card(ui);
            self.plugin_card(ui);
            ui.add_space(SP_4);
            self.version_card(ui);
        }
    }

    fn account_card(&mut self, ui: &mut egui::Ui) {
        let name = self.me.as_ref().map(|m| m.name.clone()).unwrap_or_default();
        let companies = self
            .me
            .as_ref()
            .map(|m| m.companies.clone())
            .unwrap_or_default();
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, "Konto", "");
            ui.add_space(SP_2);
            theme::key_value(ui, "Angemeldet als", &name, None);
            theme::key_value(ui, "Server", &self.settings.server_url, None);
            if !companies.is_empty() {
                ui.add_space(SP_2);
                theme::field_label(ui, "Firma");
                let current = self
                    .company
                    .as_ref()
                    .map_or_else(|| "Firma wählen".to_string(), |c| c.name.clone());
                let mut chosen = self.settings.company_id;
                egui::ComboBox::from_id_salt("firma")
                    .selected_text(current)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for c in &companies {
                            let role = if c.role == "chef" { "Chef" } else { "Fahrer" };
                            ui.selectable_value(
                                &mut chosen,
                                Some(c.id),
                                format!("{} ({role})", c.name),
                            );
                        }
                    });
                self.choose_company(ui.ctx(), chosen);
            }
            ui.add_space(SP_3);
            if ui
                .add_enabled(
                    self.busy.is_none(),
                    ActionButton::secondary("Abmelden").icon(Icon::Logout),
                )
                .clicked()
            {
                self.logout(ui.ctx());
            }
        });
        ui.add_space(SP_4);
    }

    fn password_card(&mut self, ui: &mut egui::Ui) {
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(
                ui,
                "Passwort ändern",
                "Danach sind alle anderen Anmeldungen mit diesem Konto abgemeldet, zum Beispiel im Dashboard. Der Launcher bleibt angemeldet",
            );
            ui.add_space(SP_3);
            for (label, value) in [
                ("Altes Passwort", &mut self.pw_old),
                ("Neues Passwort, mindestens 8 Zeichen", &mut self.pw_new),
                ("Neues Passwort wiederholen", &mut self.pw_repeat),
            ] {
                theme::field_label(ui, label);
                ui.add(
                    theme::text_field(value)
                        .password(true)
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(SP_1);
            }
            ui.add_space(SP_2);
            let ready = !self.pw_old.is_empty()
                && !self.pw_new.is_empty()
                && !self.pw_repeat.is_empty()
                && self.busy.is_none();
            if ui
                .add_enabled(ready, ActionButton::primary("Passwort ändern").icon(Icon::Key))
                .clicked()
            {
                self.change_password(ui.ctx());
            }
        });
        ui.add_space(SP_4);
    }

    /// Stand des Plugins mit Knöpfen, im Anmeldefenster und in den Einstellungen.
    fn plugin_card(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        let (state, tone) = plugin_state(self.plugin.status);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(Layout::top_down(Align::Min), |ui| {
                row(ui, 28.0, |ui| {
                    ui.label(text("Plugin", semibold(FS_MD), p.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        theme::badge(ui, state, tone);
                    });
                });
                theme::help_text(
                    ui,
                    "Meldet Aufträge, Maut, Strafen und den Herzschlag aus dem Spiel an den Server",
                );
                if let Some(dir) = &self.plugin.game_dir {
                    ui.add_space(SP_1);
                    theme::key_value(ui, "Spiel", &dir.display().to_string(), None);
                }
                if let Some(problem) = &self.plugin.problem {
                    ui.add(egui::Label::new(text(problem, regular(FS_SM), p.text_2)).wrap());
                }
                ui.add_space(SP_3);
                ui.horizontal_wrapped(|ui| self.plugin_actions(ui));
            });
        });
    }

    /// Knöpfe zum Plugin: installieren, Ordner wählen, neu prüfen.
    fn plugin_actions(&mut self, ui: &mut egui::Ui) {
        let status = self.plugin.status;
        if self.plugin.installing {
            ui.spinner();
            ui.label("Plugin wird installiert");
            return;
        }
        if (status.can_install() || status == PluginStatus::GameNotFound)
            && ui
                .add(ActionButton::primary("Plugin installieren").icon(Icon::Plugin))
                .clicked()
        {
            self.install_plugin(ui.ctx());
        }
        if status == PluginStatus::GameNotFound
            && ui.add(ActionButton::secondary("Ordner wählen")).clicked()
        {
            self.choose_game_folder(ui.ctx(), false);
        }
        if ui
            .add(ActionButton::quiet("Neu prüfen").icon(Icon::Refresh))
            .clicked()
        {
            self.plugin = PluginView::detect(&self.settings);
        }
    }

    fn version_card(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, "Version", "");
            ui.add_space(SP_2);
            theme::key_value(ui, "Launcher", VERSION, None);
            let (state, color) = match &self.update {
                UpdateState::Checking => ("Suche nach Updates".to_string(), None),
                UpdateState::Current => ("aktuell".to_string(), Some(p.success)),
                UpdateState::Failed(_) => ("Update-Prüfung ging nicht".to_string(), Some(p.text_2)),
                UpdateState::Available(r) | UpdateState::Downloading(r) => {
                    (format!("Version {} verfügbar", r.version), Some(p.warning))
                }
            };
            theme::key_value(ui, "Stand", &state, color);
            if let UpdateState::Failed(e) = &self.update {
                theme::help_text(ui, e);
            }
        });
    }

    /// Unten: was gerade läuft, oder die letzte Meldung.
    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        ui.horizontal(|ui| {
            if let Some(b) = &self.busy {
                ui.spinner();
                ui.label(text(b, regular(FS_BASE), p.text));
            } else if let Some(m) = &self.message {
                let tone = if m.error { Tone::Danger } else { Tone::Info };
                theme::icon(ui, tone.icon(), 18.0, tone.strong(p));
                let color = if m.error { p.danger } else { p.text };
                ui.add(egui::Label::new(text(&m.text, regular(FS_BASE), color)).wrap());
            }
        });
    }

    /// Dialog "Lkw zurückgeben": Kilometer und Zustand aus dem Spielstand, sonst von Hand.
    fn return_dialog(&mut self, ctx: &egui::Context) {
        let mut cancel = false;
        let mut confirm = false;
        let Some(dialog) = &mut self.returning else {
            return;
        };
        let p = theme::palette_of(ctx);
        let modal = egui::Modal::new(egui::Id::unique("rueckgabe"))
            .frame(theme::dialog_frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(440.0);
                row(ui, 32.0, |ui| {
                    ui.label(text("Lkw zurückgeben", semibold(FS_MD), p.text));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        theme::plate(ui, &dialog.plate, PlateSize::Small);
                    });
                });
                ui.add_space(SP_2);
                match &dialog.data {
                    Some(d) => {
                        let how = match d.found_by {
                            FoundBy::Plate => "Kennzeichen".to_string(),
                            FoundBy::Parts { share } => format!(
                                "Modell und Teile, {} Prozent Übereinstimmung",
                                (share * 100.0).floor()
                            ),
                        };
                        theme::inset(ui).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            theme::key_value(ui, "Speicherplatz", &d.slot, None);
                            theme::key_value(ui, "Kilometerstand", &theme::km(d.odometer_km), None);
                            theme::key_value(ui, "Gefunden über", &how, None);
                        });
                        ui.add_space(SP_1);
                        ui.add(
                            egui::Label::new(text(
                                "Gelesen aus dem neuesten Spielstand. Zustand und Verschleiß gehen mit an den Server",
                                regular(FS_SM),
                                p.text_2,
                            ))
                            .wrap(),
                        );
                        if let Some(new) = &d.new_plate {
                            ui.add_space(SP_2);
                            theme::notice(
                                ui,
                                Tone::Info,
                                "",
                                &format!(
                                    "Das Spiel hat das Kennzeichen ersetzt. Neu ist {new}, das meldet der Launcher mit"
                                ),
                                |_| {},
                            );
                        }
                    }
                    None => {
                        if let Some(problem) = &dialog.problem {
                            theme::notice(ui, Tone::Danger, "", problem, |_| {});
                            ui.add_space(SP_2);
                        }
                        ui.add(
                            egui::Label::new(text(
                                "Im neuesten Spielstand ist der Lkw nicht sicher zu finden. Bitte den Kilometerstand von Hand eingeben",
                                regular(FS_BASE),
                                p.text,
                            ))
                            .wrap(),
                        );
                        ui.add_space(SP_2);
                        theme::field_label(ui, "Kilometerstand");
                        ui.add(theme::text_field(&mut dialog.manual_km).desired_width(180.0));
                    }
                }
                ui.add_space(SP_5);
                row_right(ui, |ui| {
                    if ui.add(ActionButton::primary("Zurückgeben")).clicked() {
                        confirm = true;
                    }
                    if ui.add(ActionButton::quiet("Abbrechen")).clicked() {
                        cancel = true;
                    }
                });
            });
        if modal.should_close() {
            cancel = true;
        }
        if confirm {
            self.finish_return(ctx);
        } else if cancel {
            self.returning = None;
        }
    }

    /// Dialog nach "Lkw übernehmen": Betriebskosten-Buchung als Kauf verknüpfen.
    fn link_dialog(&mut self, ctx: &egui::Context) {
        let mut skip = false;
        let mut confirm = false;
        let idle = self.busy.is_none();
        let Some(dialog) = &mut self.linking else {
            return;
        };
        let p = theme::palette_of(ctx);
        let now = unix_seconds(SystemTime::now()) as i64;
        egui::Modal::new(egui::Id::unique("kauf"))
            .frame(theme::dialog_frame(ctx))
            .show(ctx, |ui| {
                ui.set_width(460.0);
                ui.label(text(
                    format!("Kauf von {} verknüpfen", dialog.plate),
                    semibold(FS_MD),
                    p.text,
                ));
                ui.add_space(SP_2);
                ui.add(
                    egui::Label::new(text(
                        format!(
                            "Hast du {} im Spiel gekauft, hat der Geldabgleich den Kaufpreis als Betriebskosten gebucht. Wähle die Buchung, dann steht im Kassenbuch Lkw-Kauf {} statt Betriebskosten. Die Buchung selbst bleibt, wie sie ist",
                            dialog.plate, dialog.plate
                        ),
                        regular(FS_BASE),
                        p.text_2,
                    ))
                    .wrap(),
                );
                ui.add_space(SP_3);
                for c in &dialog.candidates {
                    let selected = dialog.chosen == Some(c.id);
                    if theme::choice_row(ui, c.id, selected, |ui| {
                        ui.label(text(c.label(now), regular(FS_BASE), p.text));
                    })
                    .clicked()
                    {
                        dialog.chosen = Some(c.id);
                    }
                }
                ui.add_space(SP_5);
                row_right(ui, |ui| {
                    if ui
                        .add_enabled(
                            idle && dialog.chosen.is_some(),
                            ActionButton::primary("Als Lkw-Kauf verknüpfen"),
                        )
                        .clicked()
                    {
                        confirm = true;
                    }
                    if ui.add(ActionButton::quiet("Nicht verknüpfen")).clicked() {
                        skip = true;
                    }
                });
            });
        if skip {
            self.linking = None;
        } else if confirm {
            self.link_purchase(ctx);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        while let Ok(done) = self.rx.try_recv() {
            let ctx = ui.ctx().clone();
            self.handle(&ctx, done);
        }
        while let Ok(done) = self.bg_rx.try_recv() {
            let ctx = ui.ctx().clone();
            self.handle_background(&ctx, done);
        }
        let p = theme::palette(ui);
        if self.busy.is_some() || self.message.is_some() {
            egui::Panel::bottom("status")
                .show_separator_line(true)
                .frame(
                    Frame::new()
                        .fill(p.surface)
                        .inner_margin(Margin::symmetric(SP_6 as i8, SP_3 as i8)),
                )
                .show(ui, |ui| self.status_bar(ui));
        }
        if self.token.is_none() {
            egui::CentralPanel::default()
                .frame(Frame::new().fill(p.bg))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| self.login_view(ui));
                });
            return;
        }
        if self.view == View::Import && !self.is_chef() {
            self.view = View::Fleet;
        }
        egui::Panel::left("navigation")
            .exact_size(NAV_WIDTH)
            .resizable(false)
            .show_separator_line(false)
            .frame(
                Frame::new()
                    .fill(p.nav_bg)
                    .inner_margin(Margin::symmetric(SP_3 as i8, SP_4 as i8)),
            )
            .show(ui, |ui| self.sidebar(ui));
        egui::CentralPanel::default()
            .frame(Frame::new().fill(p.bg))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .show(ui, |ui| {
                        Frame::new()
                            .inner_margin(Margin::same(SP_6 as i8))
                            .show(ui, |ui| {
                                ui.set_max_width(ui.available_width().min(CONTENT_MAX_WIDTH));
                                self.content(ui);
                            });
                    });
            });
        let ctx = ui.ctx().clone();
        self.return_dialog(&ctx);
        self.link_dialog(&ctx);
    }
}

/// Die Bereiche des Launchers in der Seitenleiste.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Fleet,
    Money,
    Import,
    Settings,
}

impl View {
    const ALL: [View; 4] = [View::Fleet, View::Money, View::Import, View::Settings];

    fn nav_label(self) -> &'static str {
        match self {
            View::Fleet => "Fuhrpark",
            View::Money => "Geld und Spielstand",
            View::Import => "Übernehmen",
            View::Settings => "Einstellungen",
        }
    }

    fn title(self) -> &'static str {
        match self {
            View::Fleet => "Fuhrpark",
            View::Money => "Geld und Spielstand",
            View::Import => "Lkw übernehmen",
            View::Settings => "Einstellungen",
        }
    }

    fn icon(self) -> Icon {
        match self {
            View::Fleet => Icon::Truck,
            View::Money => Icon::Money,
            View::Import => Icon::Import,
            View::Settings => Icon::Settings,
        }
    }
}

/// Hauptaktion auf einer Lkw-Karte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TruckAction {
    Checkout,
    Write,
    Return,
}

/// Schadensgrenze, wenn der Server keine meldet (sein Standard).
const DEFAULT_DAMAGE_LIMIT: i64 = 25;

/// Zeile fester Höhe, Inhalt links nach rechts, mittig.
fn row<R>(ui: &mut egui::Ui, height: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), height),
        Layout::left_to_right(Align::Center),
        add,
    )
    .inner
}

/// Zeile für Knöpfe rechtsbündig, der zuerst hinzugefügte steht ganz rechts.
fn row_right<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.allocate_ui_with_layout(
        vec2(ui.available_width(), CONTROL_HEIGHT),
        Layout::right_to_left(Align::Center),
        add,
    )
    .inner
}

/// Kurzer Stand des Plugins für Kachel und Plakette.
fn plugin_state(status: PluginStatus) -> (&'static str, Tone) {
    match status {
        PluginStatus::Installed => ("installiert", Tone::Success),
        PluginStatus::Outdated => ("veraltet", Tone::Warning),
        PluginStatus::Missing => ("fehlt", Tone::Warning),
        PluginStatus::GameNotFound => ("kein Spielordner", Tone::Warning),
        PluginStatus::NoBundledDll => ("keine DLL", Tone::Neutral),
    }
}

fn stage_text(stage: Option<i64>) -> String {
    stage.map_or_else(
        || "keine Lohnstufe".to_string(),
        |s| format!("Lohnstufe {s}"),
    )
}

fn profile_label(p: &Profile) -> String {
    if p.steam_cloud {
        format!("{} (Steam Cloud)", p.name)
    } else {
        p.name.clone()
    }
}

/// Bauplan eines geliehenen Lkw: aus `leases\` oder vom Server, mit dem Kennzeichen vom
/// Server (`plate`, wie im Fuhrpark).
fn package_for(
    client: &Client,
    paths: Option<&AppPaths>,
    truck_id: i64,
    plate: &str,
) -> Result<spedhub_save::TruckPackage> {
    match paths {
        Some(p) => lease_package(client, p, truck_id, plate).map(|(package, _)| package),
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
