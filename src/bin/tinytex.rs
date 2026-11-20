//! The `tinytex` command-line binary: a thin shell over [`tinytex::cli`].

fn main() {
    let host = tinytex::cli::Host::system();
    let io = tinytex::cli::Io::stdio();
    let code = tinytex::cli::run(std::env::args_os(), &host, &io);
    std::process::exit(code);
}
