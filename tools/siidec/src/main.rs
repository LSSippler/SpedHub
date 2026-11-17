use std::{env, fs, process};
fn main() {
    let a: Vec<String> = env::args().collect();
    if a.len() != 3 {
        eprintln!("usage: siidec <in> <out>");
        process::exit(2);
    }
    let data = fs::read(&a[1]).expect("read");
    match decrypt_truck::decrypt_bin_file(&data) {
        Ok(out) => {
            fs::write(&a[2], &out).expect("write");
            println!("ok {} -> {} bytes", data.len(), out.len());
        }
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}
