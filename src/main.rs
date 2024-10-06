use clap::Parser;
use std::error::Error;
use std::path::Path;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    exclude: Vec<String>,

    // TODO: This should be default true but for me for now its opt in
    #[arg(long, default_value_t = false)]
    sync: bool,

    // Default is like rsync, sync src -> dir only
    //
    // Plan is to make it bidirectional, not sure what default should be.
    #[arg(long, default_value_t = false)]
    bidirectional: bool,

    source: String,
    dest: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    // TODO: integrate env args with yeet::Config at some point
    //    let args: Vec<String> = env::args().collect();

    let cli = Cli::parse();

    let lhs = Path::new(cli.source.as_str());
    let rhs = Path::new(cli.dest.as_str());

    if cli.sync {
        let conf = yeet::Config {
            excludes: cli.exclude.clone(),
        };
        yeet::sync(lhs, rhs, conf)?;
    }

    dbg!(cli.sync, cli.exclude, lhs, rhs);

    Ok(())
}
