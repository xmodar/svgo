fn main() {
    std::process::exit(_svgo::cli_main(std::env::args().skip(1).collect()));
}
