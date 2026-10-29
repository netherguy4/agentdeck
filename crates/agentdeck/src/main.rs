use clap::Parser;

#[derive(Parser)]
#[command(name = "agentdeck", version, about)]
struct Cli {}

fn main() -> anyhow::Result<()> {
    Cli::parse();
    Ok(())
}
