//! Oberfläche des Launchers. Die Logik liegt in der Bibliothek `spedhub_launcher`, hier wird
//! nur angezeigt und weitergereicht. Alles, was dauert (Server, Spielstand), läuft in einem
//! eigenen Thread, damit das Fenster nicht hängt.
#![cfg_attr(windows, windows_subsystem = "windows")]

use eframe::egui;
use spedhub_launcher::cli::{self, Command, InstallEnv, InstallOutcome};
use spedhub_launcher::client::{
    Client, Company, Me, PurchaseCandidate, Truck, format_percent, version_warning,
};
use spedhub_launcher::flows::{describe, lease_package, profile_to_report, sync_and_write};
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
use spedhub_launcher::update::{self, Release, Updater};
use spedhub_launcher::version::VERSION;
use spedhub_launcher::{Error, Result};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::PathBuf,
    sync::mpsc::{Receiver, Sender, channel},
    time::SystemTime,
};

const UNCONFIRMED: &str = "Hinweis: Im Spiel bestätigt ist, dass ein eingesetzter Lkw lädt, der neue Speicherplatz des Launchers erscheint und das Spiel den Kontostand aus dem Geldabgleich übernimmt. \
Der Launcher legt immer einen neuen Speicherplatz an und sichert den alten vorher. \
Trotzdem gilt: vorher selbst ein Backup des Profils anlegen.";

const STEAM_WARNING: &str = "Achtung: Dieses Profil liegt in steam_profiles und wird mit Steam Cloud abgeglichen. \
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
            .with_inner_size([900.0, 680.0])
            .with_min_inner_size([640.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "SpedHub",
        options,
        Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))),
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
    /// Firmenprofil beim Server gemeldet, oder warum nicht
    ProfileReported {
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
    /// Zuletzt versuchte Meldung des Firmenprofils (Firma, Name). Verhindert, dass dieselbe
    /// Meldung nach jedem Laden wieder rausgeht, etwa bei einem älteren Server.
    profile_report: Option<(i64, String)>,
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
            returning: None,
            linking: None,
            message: None,
            busy: None,
            tx,
            rx,
            version_warning: None,
            plugin,
            update: UpdateState::Checking,
            profile_report: None,
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
        let attempt = Some((company.id, name.clone()));
        if self.profile_report == attempt {
            return;
        }
        self.profile_report = attempt;
        let client = self.client();
        let company_id = company.id;
        self.background(ctx, move || Background::ProfileReported {
            result: client
                .set_profile(company_id, Some(&name))
                .map_err(|e| e.to_string()),
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
            }
            Done::Imported {
                text,
                truck_id,
                plate,
                candidates,
            } => {
                if candidates.is_empty() {
                    self.info(text);
                } else {
                    self.info(format!(
                        "{text}. Hast du ihn im Spiel gekauft? Dann verknüpfe über dem Fuhrpark die Buchung des Kaufs"
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
            Background::ProfileReported { name, result } => match result {
                Ok(()) => self.info(format!(
                    "Firmenprofil {name} beim Server gemeldet. Gebucht wird nur, was in diesem Profil passiert"
                )),
                Err(e) => self.error(format!(
                    "Das Firmenprofil ließ sich dem Server nicht melden: {e}"
                )),
            },
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
            let truck_id = client.add_truck(company, &truck)?;
            // Ohne Kandidaten (oder wenn die Abfrage scheitert) gibt es nichts zu verknüpfen
            let candidates = client.purchase_candidates(company).unwrap_or_default();
            Ok(Done::Imported {
                text: format!(
                    "{} ({}, {} km) ist jetzt im Fuhrpark",
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
                // Das Plugin meldet nur aus dem Firmenprofil, Plugin und Server müssen es wissen
                self.write_plugin_config();
                self.report_profile(ui.ctx());
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
        if self.settings.profile_dir.is_none() {
            ui.colored_label(egui::Color32::from_rgb(200, 120, 0), NO_PROFILE);
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
        // Ausgemusterte und verkaufte Lkw stehen unten für sich, ohne Knöpfe
        let (active, retired): (Vec<&Truck>, Vec<&Truck>) =
            self.trucks.iter().partition(|t| t.retired.is_none());
        if active.is_empty() {
            ui.label("Noch keine Lkw im Fuhrpark");
        }
        let mut action: Option<(&'static str, Truck)> = None;
        egui::Grid::new("fuhrpark")
            .num_columns(6)
            .striped(true)
            .spacing([16.0, 6.0])
            .show(ui, |ui| {
                ui.strong("Kennzeichen");
                ui.strong("Modell");
                ui.strong("km");
                ui.strong("Schaden");
                ui.strong("Fährt gerade");
                ui.strong("");
                ui.end_row();
                for t in &active {
                    ui.label(&t.plate);
                    ui.label(&t.model);
                    ui.label(t.odometer_km.to_string());
                    ui.label(t.damage_text());
                    let mine = t
                        .leased_by
                        .as_ref()
                        .is_some_and(|l| Some(l.user_id) == my_id);
                    let who = match &t.leased_by {
                        None if t.in_workshop => "in der Werkstatt".to_string(),
                        None => "frei".to_string(),
                        Some(_) if mine => "du".to_string(),
                        Some(l) => l.name.clone(),
                    };
                    // Aus dem Herzschlag des Plugins: wird der Lkw gerade gefahren?
                    let who = match (t.driving, t.speed_kmh) {
                        (true, Some(v)) => format!("{who}, fährt gerade, {v:.0} km/h"),
                        (true, None) => format!("{who}, fährt gerade"),
                        _ => who,
                    };
                    ui.label(who);
                    ui.horizontal(|ui| {
                        let idle = self.busy.is_none();
                        if t.leased_by.is_none() {
                            // Ein Lkw in der Werkstatt lässt sich erst nach der Reparatur ausleihen
                            if ui
                                .add_enabled(idle && !t.in_workshop, egui::Button::new("Ausleihen"))
                                .on_disabled_hover_text(if t.in_workshop {
                                    "In der Werkstatt. Ausleihen geht erst, wenn der Chef ihn im Dashboard reparieren lässt"
                                } else {
                                    "Bitte warten"
                                })
                                .clicked()
                            {
                                action = Some(("ausleihen", (*t).clone()));
                            }
                        } else if mine {
                            if ui
                                .add_enabled(idle, egui::Button::new("In Spielstand schreiben"))
                                .clicked()
                            {
                                action = Some(("schreiben", (*t).clone()));
                            }
                            if ui
                                .add_enabled(idle, egui::Button::new("Zurückgeben"))
                                .clicked()
                            {
                                action = Some(("zurueck", (*t).clone()));
                            }
                        }
                    });
                    ui.end_row();
                }
            });
        if !retired.is_empty() {
            egui::CollapsingHeader::new(format!("Ausgemustert und verkauft ({})", retired.len()))
                .default_open(false)
                .show(ui, |ui| {
                    for t in &retired {
                        let why = t.retired.as_ref().map(|r| r.text()).unwrap_or_default();
                        ui.label(format!(
                            "{}, {}, {} km, {why}",
                            t.plate, t.model, t.odometer_km
                        ));
                    }
                });
        }
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

    fn link_view(&mut self, ui: &mut egui::Ui) {
        let mut skip = false;
        let mut confirm = false;
        let idle = self.busy.is_none();
        let Some(dialog) = &mut self.linking else {
            return;
        };
        let now = unix_seconds(SystemTime::now()) as i64;
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.strong(format!("Kauf von {} verknüpfen", dialog.plate));
            ui.label(format!(
                "Hast du {} im Spiel gekauft, hat der Geldabgleich den Kaufpreis als Betriebskosten gebucht. Wähle die Buchung, dann steht im Kassenbuch Lkw-Kauf {} statt Betriebskosten. Die Buchung selbst bleibt, wie sie ist",
                dialog.plate, dialog.plate
            ));
            for c in &dialog.candidates {
                ui.radio_value(&mut dialog.chosen, Some(c.id), c.label(now));
            }
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        idle && dialog.chosen.is_some(),
                        egui::Button::new("Als Lkw-Kauf verknüpfen"),
                    )
                    .clicked()
                {
                    confirm = true;
                }
                if ui.button("Nicht verknüpfen").clicked() {
                    skip = true;
                }
            });
        });
        if skip {
            self.linking = None;
        } else if confirm {
            self.link_purchase(ui.ctx());
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

    fn settings_view(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Einstellungen")
            .default_open(false)
            .show(ui, |ui| {
                ui.strong("Passwort ändern");
                ui.label("Danach sind alle anderen Anmeldungen mit diesem Konto abgemeldet, zum Beispiel im Dashboard. Der Launcher bleibt angemeldet");
                egui::Grid::new("passwort")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Altes Passwort");
                        ui.add(egui::TextEdit::singleline(&mut self.pw_old).password(true));
                        ui.end_row();
                        ui.label("Neues Passwort, mindestens 8 Zeichen");
                        ui.add(egui::TextEdit::singleline(&mut self.pw_new).password(true));
                        ui.end_row();
                        ui.label("Neues Passwort wiederholen");
                        ui.add(egui::TextEdit::singleline(&mut self.pw_repeat).password(true));
                        ui.end_row();
                    });
                let ready = !self.pw_old.is_empty()
                    && !self.pw_new.is_empty()
                    && !self.pw_repeat.is_empty()
                    && self.busy.is_none();
                if ui
                    .add_enabled(ready, egui::Button::new("Passwort ändern"))
                    .clicked()
                {
                    self.change_password(ui.ctx());
                }
            });
    }

    /// Hinweis auf eine neue Version, oben im Fenster.
    fn update_banner(&mut self, ui: &mut egui::Ui) {
        let (release, downloading) = match &self.update {
            UpdateState::Available(r) => (r.clone(), false),
            UpdateState::Downloading(r) => (r.clone(), true),
            _ => return,
        };
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(
                egui::Color32::from_rgb(40, 110, 200),
                format!("Version {} verfügbar, du hast {VERSION}.", release.version),
            );
            if downloading {
                ui.spinner();
                ui.label("Setup wird geladen und geprüft");
            } else if ui.button("Herunterladen und installieren").clicked() {
                self.start_update(ui.ctx());
            }
            if !release.html_url.is_empty() {
                ui.hyperlink_to("Was ist neu?", &release.html_url);
            }
        });
        ui.separator();
    }

    fn plugin_view(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            let status = self.plugin.status;
            let color = match status {
                PluginStatus::Installed => egui::Color32::from_rgb(40, 140, 60),
                PluginStatus::NoBundledDll => ui.visuals().weak_text_color(),
                _ => egui::Color32::from_rgb(200, 120, 0),
            };
            ui.colored_label(color, status.label());
            if self.plugin.installing {
                ui.spinner();
            } else {
                if (status.can_install() || status == PluginStatus::GameNotFound)
                    && ui.button("Plugin installieren").clicked()
                {
                    self.install_plugin(ui.ctx());
                }
                if status == PluginStatus::GameNotFound && ui.button("Ordner wählen").clicked() {
                    self.choose_game_folder(ui.ctx(), false);
                }
                if ui.button("Neu prüfen").clicked() {
                    self.plugin = PluginView::detect(&self.settings);
                }
            }
        });
        if let Some(dir) = &self.plugin.game_dir {
            ui.weak(format!("Spiel: {}", dir.display()));
        }
        if let Some(problem) = &self.plugin.problem {
            ui.weak(problem);
        }
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
        // Version und Stand der Update-Prüfung, leise
        ui.horizontal_wrapped(|ui| {
            ui.weak(format!("SpedHub {VERSION}"));
            match &self.update {
                UpdateState::Checking => ui.weak("Suche nach Updates"),
                UpdateState::Current => ui.weak("aktuell"),
                UpdateState::Failed(e) => ui.weak(format!("Update-Prüfung: {e}")),
                UpdateState::Available(r) | UpdateState::Downloading(r) => {
                    ui.weak(format!("Version {} verfügbar", r.version))
                }
            };
        });
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
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.update_banner(ui);
                if self.token.is_none() {
                    self.login_view(ui);
                    ui.add_space(12.0);
                    ui.separator();
                    self.plugin_view(ui);
                    return;
                }
                self.top_bar(ui);
                if let Some(w) = &self.version_warning {
                    ui.colored_label(egui::Color32::from_rgb(200, 150, 0), w);
                }
                ui.separator();
                self.profile_view(ui);
                ui.add_space(4.0);
                self.plugin_view(ui);
                ui.add_space(4.0);
                ui.colored_label(egui::Color32::from_rgb(200, 120, 0), UNCONFIRMED);
                ui.separator();
                self.return_view(ui);
                self.link_view(ui);
                self.fleet_view(ui);
                ui.add_space(12.0);
                self.import_view(ui);
                ui.add_space(8.0);
                self.settings_view(ui);
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
