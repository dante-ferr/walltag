use anyhow::Result;
use clap::Parser;
use walltag::{config::ConfigPaths, ui, Cli};

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("walltag=info")),
        )
        .init();

    let cli = Cli::parse();
    let mut config = ConfigPaths::load()?;

    if cli.restore {
        return cli.run_headless(config);
    }

    if let Some(root) = cli.wallpaper_root {
        config.wallpaper_root = root;
        config.expand_paths()?;
    }

    Cli::ensure_wallpaper_root(&config)?;

    ui::launch(config)
}
