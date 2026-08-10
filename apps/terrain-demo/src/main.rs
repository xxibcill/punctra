//! Thin command-line presentation host for the durable terrain workflow.

fn main() {
    match terrain_demo::run_cli(std::env::args_os().skip(1)) {
        Ok(output) => print!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
