//! Warteschlange und Sender-Thread.
//!
//! Der Spiel-Thread ruft nur [`Agent::submit`] auf. Das übersetzt das Event, vergibt die
//! `event_id` und legt es in einen Kanal, der nie blockiert. Kein Dateizugriff, kein Netz.
//!
//! Der Sender-Thread schreibt jedes Event zuerst in den Postausgang und schickt dann alles,
//! was dort liegt, der Reihe nach an den Server. Ist der Server nicht erreichbar, wartet er
//! und versucht es später wieder, mit wachsender Pause bis zu `retry_max`. Beim nächsten
//! Start des Spiels geht es mit dem Postausgang weiter.

use crate::config::{Config, ConfigError, Paths};
use crate::event::{GameplayData, translate};
use crate::ids::{IdGenerator, StartNote};
use crate::log::Logger;
use crate::outbox::{Entry, Outbox};
use crate::send::{Outcome, Sender};
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            retry_initial: Duration::from_secs(5),
            retry_max: Duration::from_secs(300),
            request_timeout: Duration::from_secs(3),
        }
    }
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
            "Server {}, Firma {}",
            config.server_url, config.company_id
        ));

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
        Ok(Agent {
            tx,
            ids,
            company_id: config.company_id,
            stop,
            worker,
            logger,
        })
    }

    /// Nimmt ein Event aus dem Spiel an. Blockiert nie und greift nicht auf Dateien oder das
    /// Netz zu.
    pub fn submit(&self, data: &GameplayData) -> Submitted {
        match translate(data) {
            Ok(Some(draft)) => {
                let event_id = self.ids.next();
                let entry = Entry {
                    company_id: self.company_id,
                    booking: draft.with_id(event_id.clone()),
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

    /// Beendet den Sender-Thread. Wartet höchstens so lange, wie eine laufende Anfrage noch
    /// braucht. Was nicht gesendet ist, bleibt im Postausgang.
    pub fn shutdown(mut self) {
        self.stop_worker();
    }

    fn stop_worker(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.tx.send(Message::Stop);
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            self.logger.write("Sender-Thread ist abgestürzt");
        }
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        // Der Thread darf die DLL nie überleben: Das Spiel kann Plugins entladen.
        self.stop_worker();
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
    let mut w = Worker {
        config,
        settings,
        stop,
        logger,
        next_try: (!outbox.is_empty()).then(Instant::now),
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
