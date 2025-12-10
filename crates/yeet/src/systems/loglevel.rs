use log::Level;
use std::sync::{Arc, OnceLock};

use bevy::prelude::*;
use tracing_subscriber::{EnvFilter, Registry, prelude::*, reload, reload::Handle};

static LOG_HANDLE: OnceLock<Arc<Handle<EnvFilter, Registry>>> = OnceLock::new();

/// Cloneable Bevy resource that holds an Arc to the reload handle
#[derive(Resource, Clone)]
pub struct LogHandle {
    handle: Arc<Handle<EnvFilter, Registry>>,
}

impl LogHandle {
    //https://docs.rs/tracing-subscriber/latest/tracing_subscriber/reload/index.html
    pub fn set_max_level(&self, level: String) -> Result<()> {
        Ok(self.handle.modify(|filter| {
            // *filter =
            //     tracing_subscriber::EnvFilter::new(tracing_subscriber::filter::LevelFilter::DEBUG)
            *filter = tracing_subscriber::EnvFilter::new(level)
        })?)
        //            .map_err(|e| anyhow::anyhow!(e).into())
    }
}

pub struct LogLevelPlugin {
    pub level: Level,
}

impl Plugin for LogLevelPlugin {
    fn build(&self, app: &mut App) {
        // Only enable raw mode if we're actually running in a TTY
        let is_tty = atty::is(atty::Stream::Stdout);

        if is_tty {
            // Yeeted from https://werat.dev/blog/pretty-rust-backtraces-in-raw-terminal-mode/ need to brain a more long term approach here.
            crossterm::terminal::enable_raw_mode()
                .expect("couldn't switch tty to raw mode from cooked");

            let default_panic = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                _ = crossterm::terminal::disable_raw_mode();
                println!();
                default_panic(info);
            }));
        }

        use std::io::{self, Write};
        use tracing_subscriber::fmt::format::Format;

        let fmt = Format::default();

        let _initial_level = self.level;

        let filter = tracing_subscriber::EnvFilter::from_default_env();
        let (filter, reload_handle) = reload::Layer::new(filter);

        // Only use CRLF wrapper in raw terminal mode, otherwise use normal stdout
        if is_tty {
            struct CRLFStdout(io::Stdout);
            impl Write for CRLFStdout {
                fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                    let mut out = Vec::new();
                    for &b in buf {
                        if b == b'\n' {
                            out.push(b'\r');
                        }
                        out.push(b);
                    }
                    self.0.write_all(&out)?;
                    Ok(buf.len())
                }
                fn flush(&mut self) -> io::Result<()> {
                    self.0.flush()
                }
            }
            let fmt_layer = tracing_subscriber::fmt::layer()
                .event_format(fmt)
                .with_writer(|| CRLFStdout(io::stdout()));

            tracing_subscriber::registry()
                .with(filter)
                .with(fmt_layer)
                .init();
        } else {
            // No need for the shenanigans raw mode wrapper if we're a daemon.
            let fmt_layer = tracing_subscriber::fmt::layer()
                .event_format(fmt)
                .with_writer(io::stdout);

            tracing_subscriber::registry()
                .with(filter)
                .with(fmt_layer)
                .init();
        }

        // tracing_subscriber::fmt()
        //     .event_format(fmt)
        //     .with_writer(io::stdout)
        //     .with_writer(move || CRLFStdout(io::stdout()))
        //     .init();

        // tracing_subscriber::registry()
        //     .with(filter)
        //     .with(fmt::Layer::default())
        //     .init();

        // reload_handle
        //     .modify(|filter| *filter = tracing_subscriber::EnvFilter::new("debug"))
        //     .expect("shit");

        let handle = LOG_HANDLE.get_or_init(|| Arc::new(reload_handle)).clone();

        // handle
        //     .modify(|filter| {
        //         // *filter =
        //         //     tracing_subscriber::EnvFilter::new(tracing_subscriber::filter::LevelFilter::DEBUG)
        //         *filter = tracing_subscriber::EnvFilter::new("debug")
        //     })
        //     .expect("shit2");

        app.insert_resource(LogHandle { handle });
        //        crossterm::terminal::disable_raw_mode().unwrap();
    }
}
