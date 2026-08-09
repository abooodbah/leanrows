mod build_support;

fn main() {
    if let Err(error) = build_support::run() {
        eprintln!("LeanRows resource compilation failed: {error}");
        std::process::exit(1);
    }
}
