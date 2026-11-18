//! Warteschlange und Sender-Thread.
//!
//! Der Spiel-Thread ruft nur [`Agent::submit`] auf. Das übersetzt das Event, prüft das aktive
//! Profil, vergibt die `event_id`, merkt sich den Zeitpunkt (`occurred_at`, Systemzeit) und das
//! Profil und legt es in einen Kanal, der nie blockiert. Kein Netz. Gelesen werden nur die neuen
//! Zeilen von `game.log.txt` (siehe [`crate::profile`]), geschrieben wird nichts.
//!
//! Nur Ereignisse aus dem Firmenprofil werden angenommen. Ist ein anderes Profil aktiv oder
//! das Profil unbekannt, kommt das Ereignis nicht in den Postausgang, es steht nur im Log.
//! Für Herzschläge gilt dasselbe.
//!
//! Der Sender-Thread schreibt jedes Event zuerst in den Postausgang und schickt dann alles,
//! was dort liegt, der Reihe nach an den Server. Ist der Server nicht erreichbar, wartet er
//! und versucht es später wieder, mit wachsender Pause bis zu `retry_max`. Beim nächsten
//! Start des Spiels geht es mit dem Postausgang weiter.
//!
//! Ein eigener Herzschlag-Thread liest alle `heartbeat_interval` den [`Telemetry`]-Snapshot
//! und schickt einen Herzschlag, wenn das Spiel läuft, nicht pausiert ist, ein Kennzeichen
//! bekannt ist und das Firmenprofil aktiv ist. Herzschläge kommen nie in den Postausgang: Geht
//! einer nicht durch, ist er verloren. Weil der Thread eigen ist, hält ein hängender Herzschlag
//! keine Buchung auf.

use crate::config::{Config, ConfigError, Paths};
use crate::event::{GameplayData, translate, unix_now};
use crate::ids::{IdGenerator, StartNote};
use crate::log::Logger;
use crate::outbox::{Entry, Outbox};
use crate::profile::{ProfileGate, ProfileState};
use crate::send::{Outcome, Sender};
use crate::telemetry::Telemetry;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

/// Zeiten für Senden und Wiederholen.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// Pause nach dem ersten Fehlschlag
    pub retry_initial: Duration,
    /// Längste Pause zwischen zwei Versuchen
    pub retry_max: Duration,
    /// Höchstdauer einer Anfrage. Bestimmt auch, wie lange das Beenden höchstens dauert.
    pub request_timeout: Duration,
    /// Abstand zwischen zwei Herzschlägen
    pub heartbeat_interval: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            retry_initial: Duration::from_secs(5),
            retry_max: Duration::from_secs(300),
            request_timeout: Duration::from_secs(3),
            heartbeat_interval: Duration::from_secs(10),
        }
    }
}

/// Zähler des Herzschlag-Threads, zum Nachsehen in Tests und im Log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeartbeatStats {
    /// Wie oft der Takt geschlagen hat
    pub ticks: u64,
    /// Wie oft ein Herzschlag rausging (Spiel lief, Kennzeichen bekannt)
    pub attempts: u64,
    /// Wie viele der Server angenommen hat
    pub delivered: u64,
    /// Wie oft ein Herzschlag fällig war, aber nicht das Firmenprofil aktiv war
    pub blocked: u64,
}

#[derive(Default)]
struct Counters {
    ticks: AtomicU64,
    attempts: AtomicU64,
    delivered: AtomicU64,
    blocked: AtomicU64,
}

/// Was [`Agent::submit`] mit einem Event gemacht hat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submitted {
    /// In der Warteschlange, mit dieser `event_id`
    Queued(String),
    /// Ein Event, das SpedHub nicht bucht
    Ignored,
    /// Gebucht hätte werden sollen, aber die Werte fehlen. Steht im Log.
    Invalid,
    /// Aus diesem Profil, nicht dem Firmenprofil. Nicht angenommen, steht im Log.
    OtherProfile(String),
    /// Das aktive Profil ist unbekannt. Nicht angenommen, steht im Log.
    UnknownProfile,
    /// `plugin.json` nennt kein Firmenprofil. Nicht angenommen, steht im Log.
    NoCompanyProfile,
}

/// Das gemeinsame Tor für Spiel-Thread und Herzschlag-Thread.
type Gate = Arc<Mutex<ProfileGate>>;

fn check(gate: &Gate) -> (ProfileState, Option<String>) {
    gate.lock().unwrap_or_else(|e| e.into_inner()).check()
}

enum Message {
    Entry(Entry),
    Log(String),
    Stop,
}

pub struct Agent {
    tx: mpsc::Sender<Message>,
    ids: IdGenerator,
    company_id: i64,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    logger: Arc<Logger>,
    telemetry: Telemetry,
    /// Beim Beenden fallen lassen: Dann wacht der Herzschlag-Thread auf und hört auf.
    heartbeat_stop: Option<mpsc::Sender<()>>,
    heartbeat: Option<JoinHandle<()>>,
    counters: Arc<Counters>,
    gate: Gate,
}

impl Agent {
    /// Liest die Konfiguration und startet den Sender-Thread. Fehlt die Konfiguration oder ist
    /// sie kaputt, gibt es keinen Agent, und der Grund steht im Log.
    pub fn start(paths: &Paths, settings: Settings) -> Result<Agent, ConfigError> {
        let logger = Arc::new(Logger::new(paths.log(), crate::log::DEFAULT_MAX_BYTES));
        logger.write(&format!(
            "SpedHub-Plugin {} startet",
            env!("CARGO_PKG_VERSION")
        ));
        let config = match Config::load(&paths.config()) {
            Ok(c) => c,
            Err(e) => {
                logger.write(&e.to_string());
                return Err(e);
            }
        };
        logger.write(&format!(
            "Server {}, Firma {}, Firmenprofil {}, Log des Spiels {}",
            config.server_url,
            config.company_id,
            config
                .profile_name
                .as_deref()
                .map_or_else(|| "fehlt".to_string(), |n| format!("'{n}'")),
            config
                .game_log
                .as_deref()
                .map_or_else(|| "fehlt".to_string(), |p| p.display().to_string()),
        ));
        let gate: Gate = Arc::new(Mutex::new(ProfileGate::new(
            config.profile_name.as_deref(),
            config.game_log.as_deref(),
        )));
        if let (_, Some(note)) = check(&gate) {
            logger.write(&note);
        }

        let (ids, note) = IdGenerator::start(&paths.state());
        match note {
            StartNote::Ok => {}
            StartNote::Created => logger.write("Neue Kennung für event_ids angelegt"),
            StartNote::Replaced(e) => {
                logger.write(&format!("Zustand für event_ids war kaputt ({e}), neu angelegt"))
            }
            StartNote::NotSaved(e) => logger.write(&format!(
                "Zustand für event_ids ließ sich nicht speichern ({e}), IDs bekommen Zufall angehängt"
            )),
        }

        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker = {
            let paths = paths.clone();
            let config = config.clone();
            let stop = stop.clone();
            let logger = logger.clone();
            std::thread::Builder::new()
                .name("spedhub-sender".into())
                .spawn(move || run(rx, &paths, &config, settings, &stop, &logger))
        };
        let worker = match worker {
            Ok(w) => Some(w),
            Err(e) => {
                logger.write(&format!("Sender-Thread ließ sich nicht starten: {e}"));
                None
            }
        };

        let telemetry = Telemetry::default();
        let counters = Arc::new(Counters::default());
        let (heartbeat_stop, heartbeat_rx) = mpsc::channel();
        let heartbeat = {
            let telemetry = telemetry.clone();
            let counters = counters.clone();
            let logger = logger.clone();
            let config = config.clone();
            let gate = gate.clone();
            std::thread::Builder::new()
                .name("spedhub-heartbeat".into())
                .spawn(move || {
                    run_heartbeat(
                        &heartbeat_rx,
                        &telemetry,
                        &config,
                        settings,
                        &counters,
                        &logger,
                        &gate,
                    )
                })
        };
        let heartbeat = match heartbeat {
            Ok(h) => Some(h),
            Err(e) => {
                logger.write(&format!("Herzschlag-Thread ließ sich nicht starten: {e}"));
                None
            }
        };
        Ok(Agent {
            tx,
            ids,
            company_id: config.company_id,
            stop,
            worker,
            logger,
            telemetry,
            heartbeat_stop: Some(heartbeat_stop),
            heartbeat,
            counters,
            gate,
        })
    }

    /// Die laufenden Werte aus dem Spiel. Die FFI-Schicht schreibt hinein, der
    /// Herzschlag-Thread liest.
    pub fn telemetry(&self) -> &Telemetry {
        &self.telemetry
    }

    /// Was der Herzschlag-Thread bisher getan hat.
    pub fn heartbeat_stats(&self) -> HeartbeatStats {
        HeartbeatStats {
            ticks: self.counters.ticks.load(Ordering::SeqCst),
            attempts: self.counters.attempts.load(Ordering::SeqCst),
            delivered: self.counters.delivered.load(Ordering::SeqCst),
            blocked: self.counters.blocked.load(Ordering::SeqCst),
        }
    }

    /// Prüft jetzt, welches Profil aktiv ist. Liest nur neue Zeilen von `game.log.txt`.
    pub fn profile_state(&self) -> ProfileState {
        let (state, note) = check(&self.gate);
        if let Some(note) = note {
            let _ = self.tx.send(Message::Log(note));
        }
        state
    }

    /// Nimmt ein Event aus dem Spiel an. Greift nicht auf das Netz zu. Liest nur die neuen
    /// Zeilen von `game.log.txt`, um das aktive Profil zu prüfen. Warten kann es nur, solange
    /// der Herzschlag-Thread gerade selbst in die Datei schaut, also ebenso kurz.
    pub fn submit(&self, data: &GameplayData) -> Submitted {
        match translate(data) {
            Ok(Some(draft)) => {
                // Fracht und Städte kommen aus der letzten Job-Konfiguration. Abgeliefert oder
                // abgebrochen: Der Auftrag ist erledigt und gilt für kein späteres Event.
                let job = match draft.kind {
                    "job_delivered" => self.telemetry.take_last_job(),
                    "job_cancelled" => {
                        self.telemetry.take_last_job();
                        None
                    }
                    _ => None,
                };
                // Nur aus dem Firmenprofil. Alles andere kommt nicht in den Postausgang.
                let profile = match self.profile_state() {
                    ProfileState::Company(name) => name,
                    other => {
                        let (why, result) = match other {
                            ProfileState::Other(name) => {
                                let why = format!(
                                    "aus Profil '{name}', Firmenprofil ist '{}'",
                                    self.company_profile()
                                );
                                (why, Submitted::OtherProfile(name))
                            }
                            ProfileState::NotConfigured => (
                                "plugin.json nennt kein Firmenprofil".to_string(),
                                Submitted::NoCompanyProfile,
                            ),
                            _ => (
                                "aktives Profil unbekannt".to_string(),
                                Submitted::UnknownProfile,
                            ),
                        };
                        let _ = self.tx.send(Message::Log(format!(
                            "Ereignis nicht angenommen: {} {} Euro, {why}",
                            draft.kind, draft.amount
                        )));
                        return result;
                    }
                };
                // Der Zeitpunkt wird hier festgehalten, nicht beim Senden: Das Event kann lange
                // im Postausgang liegen.
                let event_id = self.ids.next();
                let entry = Entry {
                    company_id: self.company_id,
                    booking: draft
                        .with_id_at(event_id.clone(), unix_now())
                        .with_job(job)
                        .with_profile(&profile),
                };
                let _ = self.tx.send(Message::Entry(entry));
                Submitted::Queued(event_id)
            }
            Ok(None) => Submitted::Ignored,
            Err(message) => {
                let _ = self.tx.send(Message::Log(message));
                Submitted::Invalid
            }
        }
    }

    fn company_profile(&self) -> String {
        self.gate
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .company()
            .unwrap_or_default()
            .to_string()
    }

    /// Beendet den Sender-Thread. Wartet höchstens so lange, wie eine laufende Anfrage noch
    /// braucht. Was nicht gesendet ist, bleibt im Postausgang.
    pub fn shutdown(mut self) {
        self.stop_worker();
    }

    fn stop_worker(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Message::Stop);
        // Beide Threads bekommen das Signal, bevor auf einen gewartet wird. So dauert das
        // Beenden höchstens eine Anfrage lang, nicht zwei.
        drop(self.heartbeat_stop.take());
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            self.logger.write("Sender-Thread ist abgestürzt");
        }
        if let Some(heartbeat) = self.heartbeat.take()
            && heartbeat.join().is_err()
        {
            self.logger.write("Herzschlag-Thread ist abgestürzt");
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        // Der Thread darf die DLL nie überleben: Das Spiel kann Plugins entladen.
        self.stop_worker();
    }
}

/// Der Herzschlag-Thread: schlägt alle `heartbeat_interval`, bis `stop` fällt.
fn run_heartbeat(
    stop: &mpsc::Receiver<()>,
    telemetry: &Telemetry,
    config: &Config,
    settings: Settings,
    counters: &Counters,
    logger: &Logger,
    gate: &Gate,
) {
    let sender = Sender::new(settings.request_timeout);
    let mut failing = false;
    // Läuft, bis `stop` fällt (Disconnected) oder etwas schickt
    while let Err(RecvTimeoutError::Timeout) = stop.recv_timeout(settings.heartbeat_interval) {
        let beat = catch_unwind(AssertUnwindSafe(|| {
            counters.ticks.fetch_add(1, Ordering::SeqCst);
            // Auch in der Pause lesen: So bleibt jeder Blick in game.log.txt kurz, und ein
            // Profilwechsel steht sofort im Log.
            let (profile, note) = check(gate);
            if let Some(note) = note {
                logger.write(&note);
            }
            // Pausiert oder ohne Kennzeichen: nichts senden
            let Some(mut heartbeat) = telemetry.snapshot().heartbeat() else {
                return;
            };
            // Nur aus dem Firmenprofil
            let Some(name) = profile.company() else {
                counters.blocked.fetch_add(1, Ordering::SeqCst);
                return;
            };
            heartbeat.profile = Some(name.to_string());
            counters.attempts.fetch_add(1, Ordering::SeqCst);
            match sender.heartbeat(config, config.company_id, &heartbeat) {
                Ok(()) => {
                    counters.delivered.fetch_add(1, Ordering::SeqCst);
                    if failing {
                        logger.write("Herzschläge kommen wieder an");
                    }
                    failing = false;
                }
                Err(reason) => {
                    // Nur der erste Fehler kommt ins Log, sonst liefe es alle 10 Sekunden voll
                    if !failing {
                        logger.write(&format!(
                            "Herzschlag nicht angekommen: {reason}. Herzschläge werden verworfen, bis der Server wieder antwortet"
                        ));
                    }
                    failing = true;
                }
            }
        }));
        if beat.is_err() {
            logger.write("Interner Fehler im Herzschlag-Thread, es geht weiter");
        }
    }
}

/// Zustand des Sender-Threads.
struct Worker<'a> {
    config: &'a Config,
    settings: Settings,
    stop: &'a AtomicBool,
    logger: &'a Logger,
    outbox: Outbox,
    sender: Sender,
    next_try: Option<Instant>,
    backoff: Duration,
    failing: bool,
}

fn run(
    rx: mpsc::Receiver<Message>,
    paths: &Paths,
    config: &Config,
    settings: Settings,
    stop: &AtomicBool,
    logger: &Logger,
) {
    let (outbox, notes) = Outbox::open(paths.outbox(), paths.rejected());
    for n in notes {
        logger.write(&n);
    }
    // Ohne Firmenprofil in plugin.json sendet das Plugin nichts, auch den Postausgang nicht.
    // Er wartet, bis der Launcher die Konfiguration neu geschrieben hat.
    let configured = config.profile_name.is_some();
    let mut w = Worker {
        config,
        settings,
        stop,
        logger,
        next_try: (configured && !outbox.is_empty()).then(Instant::now),
        outbox,
        sender: Sender::new(settings.request_timeout),
        backoff: settings.retry_initial,
        failing: false,
    };
    loop {
        let wait = w.next_try.map_or(Duration::from_secs(3600), |t| {
            t.saturating_duration_since(Instant::now())
        });
        let message = rx.recv_timeout(wait);
        let keep_running =
            catch_unwind(AssertUnwindSafe(|| w.handle(message))).unwrap_or_else(|_| {
                logger.write("Interner Fehler im Sender-Thread, es geht weiter");
                true
            });
        if !keep_running {
            break;
        }
    }
    logger.write("SpedHub-Plugin beendet");
}

impl Worker<'_> {
    /// Verarbeitet eine Nachricht und sendet, wenn es Zeit ist. `false` heißt: beenden.
    fn handle(&mut self, message: Result<Message, RecvTimeoutError>) -> bool {
        match message {
            Ok(Message::Entry(entry)) => {
                let id = entry.booking.event_id.clone();
                if let Err(e) = self.outbox.push(entry) {
                    self.logger
                        .write(&format!("{e}. Event {id} bleibt nur im Speicher"));
                }
                if !self.failing {
                    self.next_try = Some(Instant::now());
                }
            }
            Ok(Message::Log(line)) => self.logger.write(&line),
            Ok(Message::Stop) | Err(RecvTimeoutError::Disconnected) => return false,
            Err(RecvTimeoutError::Timeout) => {}
        }
        if self.next_try.is_some_and(|t| t <= Instant::now()) {
            self.flush();
        }
        !self.stop.load(Ordering::SeqCst)
    }

    /// Schickt den Postausgang der Reihe nach. Bricht beim ersten Fehler ab, der auf einen
    /// nicht erreichbaren Server hindeutet.
    fn flush(&mut self) {
        if self.config.profile_name.is_none() {
            self.next_try = None;
            return;
        }
        let mut done = 0;
        let mut retry: Option<String> = None;
        for entry in self.outbox.entries() {
            if self.stop.load(Ordering::SeqCst) {
                break;
            }
            let id = &entry.booking.event_id;
            match self
                .sender
                .send(self.config, entry.company_id, &entry.booking)
            {
                Outcome::Sent { duplicate } => {
                    let extra = if duplicate { ", war schon gebucht" } else { "" };
                    self.logger.write(&format!(
                        "Gesendet: {} {} Euro ({id}{extra})",
                        entry.booking.kind, entry.booking.amount
                    ));
                }
                Outcome::Rejected(reason) => {
                    self.logger.write(&format!(
                        "Server lehnt {id} ab: {reason}. Nicht wieder gesendet"
                    ));
                    self.outbox.reject(entry, &reason);
                }
                Outcome::Retry(reason) => {
                    retry = Some(reason);
                    break;
                }
            }
            done += 1;
        }
        if let Err(e) = self.outbox.remove_first(done) {
            self.logger.write(&e);
        }
        match retry {
            Some(reason) => {
                if !self.failing {
                    self.logger.write(&format!(
                        "{reason}. {} Events bleiben im Postausgang",
                        self.outbox.entries().len()
                    ));
                }
                self.failing = true;
                self.next_try = Some(Instant::now() + self.backoff);
                self.backoff = (self.backoff * 2).min(self.settings.retry_max);
            }
            None if self.stop.load(Ordering::SeqCst) => {}
            None => {
                if self.failing {
                    self.logger.write("Server wieder erreichbar");
                }
                self.failing = false;
                self.backoff = self.settings.retry_initial;
                self.next_try = None;
            }
        }
    }
}
