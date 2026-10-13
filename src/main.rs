//! `os-int` binary entry point.

#[tokio::main]
async fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    os_int::cli::run().await
}
