fn main() {
    let args: Vec<String> = std::env::args().collect();
    let code =
        fast_static_site::cli::run(&args[1..], &mut std::io::stdout(), &mut std::io::stderr());
    std::process::exit(code);
}
