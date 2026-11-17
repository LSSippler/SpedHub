use std::{env, net::SocketAddr};

#[tokio::main]
async fn main() {
    let db_path = env::var("SPEDHUB_DB").unwrap_or_else(|_| "spedhub.db".into());
    let addr: SocketAddr = env::var("SPEDHUB_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8787".into())
        .parse()
        .expect("SPEDHUB_ADDR muss wie 127.0.0.1:8787 aussehen");

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
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("Adresse ist belegt oder ungültig");

    println!("SpedHub läuft auf http://{addr}");
    println!("Datenbank: {db_path}");
    println!("Beenden mit Strg+C");

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
        .expect("Serverfehler");
}
