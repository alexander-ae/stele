fn main() {
    if let Err(error) = stele::cli::run() {
        eprintln!("stele: {error:#}");
        // 0 is success, 1 a local failure, 2 an unreachable server and 3 a
        // server that answered no. A script can act on the difference.
        std::process::exit(stele::failure::exit_code(&error));
    }
}
