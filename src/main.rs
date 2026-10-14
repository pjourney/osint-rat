//! `os-int` binary entry point.

#[tokio::main]
async fn main() {
    if let Err(error) = color_eyre::config::HookBuilder::default()
        .display_location_section(false)
        .install()
    {
        eprintln!("{error}");
        std::process::exit(1);
    }
    if let Err(error) = os_int::cli::run().await {
        eprintln!("{}", os_int::cli::format_cli_error(&error));
        std::process::exit(1);
    }
}
