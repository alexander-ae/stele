fn main() {
    if let Err(error) = stele::cli::run() {
        eprintln!("stele: {error:#}");
        std::process::exit(1);
    }
}
