mod app;
mod ui;

use anyhow::Result;

fn main() -> Result<()> {
    // Set up logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    // Run the picker
    let mut app = app::App::new()?;
    app.run()?;

    // If an entry was selected, it's already been activated via IPC
    Ok(())
}
