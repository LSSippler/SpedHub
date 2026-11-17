use spedhub_server::config::{self, Command, Env};
use spedhub_server::version::VERSION;

#[tokio::main]
async fn main() {
    let cfg = match config::parse(std::env::args().skip(1), &Env::from_process()) {
        Ok(Command::Run(cfg)) => cfg,
        Ok(Command::Version) => {
            println!("spedhub-server {VERSION}");
            return;
        }
        Ok(Command::Help) => {
            println!("{}", config::HELP);
            return;
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    };
    if let Some(dir) = &cfg.create_dir
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        eprintln!("Ordner {} ließ sich nicht anlegen: {e}", dir.display());
        std::process::exit(1);
    }
    let db_path = cfg.db.to_string_lossy().into_owned();

    let db = spedhub_server::db::open(&db_path).expect("Datenbank konnte nicht geöffnet werden");
    let app = spedhub_server::api::router(db.clone());

    // Tägliche Lohnabrechnung: jede Minute nachsehen, ob ein Tag fällig ist. Nach einem Neustart
    // werden verpasste Tage dabei nachgeholt.
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tick.tick().await;
            let db = db.clone();
            let done = tokio::task::spawn_blocking(move || {
                let mut conn = db.lock().unwrap();
                spedhub_server::payroll::run_due_all(&mut conn, spedhub_server::db::now())
            })
            .await;
            match done {
                Ok(Ok(0)) => {}
                Ok(Ok(n)) => println!("Lohnabrechnung: {n} Tag(e) abgerechnet"),
                Ok(Err(e)) => eprintln!("Lohnabrechnung fehlgeschlagen: {e}"),
                Err(e) => eprintln!("Lohnabrechnung abgebrochen: {e}"),
            }
        }
    });
    let addr = cfg.addr;
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Adresse ist belegt oder ungültig");

    println!("SpedHub {VERSION} läuft auf http://{addr}");
    println!("Datenbank: {db_path}");
    println!("Beenden mit Strg+C");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("Serverfehler");
}
