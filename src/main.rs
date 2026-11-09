use std::process;
use yam::cli::ExitCode;

fn main() {
    match yam::cli::run() {
        Ok(code) => process::exit(code),
        Err(err) => {
            if let Some(exit) = err.downcast_ref::<ExitCode>() {
                if let Some(inner) = &exit.error {
                    eprintln!("Error: {inner:#}");
                }
                process::exit(exit.code);
            }
            // Print the whole cause chain ("failed to parse YAML: <why>").
            eprintln!("Error: {err:#}");
            process::exit(1);
        }
    }
}
