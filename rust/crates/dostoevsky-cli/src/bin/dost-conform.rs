//! Drives this engine for the conformance suite and the differential test. See `dostoevsky_cli::conform`.

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    if let Err(e) = dostoevsky_cli::conform::serve(&mut stdin.lock(), &mut stdout.lock()) {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
