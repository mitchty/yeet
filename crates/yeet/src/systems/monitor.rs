use bevy::prelude::*;

use super::netcode::protocol::{
    ReplicatedCompletionTime, ReplicatedDest, ReplicatedIoProgress, ReplicatedSimpleCopy,
    ReplicatedSource, ReplicatedSyncComplete, ReplicatedSyncStartTime, ReplicatedSyncStopTime,
    ReplicatedUuid,
};
use super::stats::{Cpu, Mem, Uptime};

pub struct MonitorPlugin;

impl Plugin for MonitorPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                update_display.run_if(bevy::time::common_conditions::on_timer(
                    std::time::Duration::from_secs(1),
                )),
                check_exit,
            ),
        )
        .add_systems(Last, cleanup_on_exit);
    }
}

fn check_exit(mut exit: MessageWriter<bevy::app::AppExit>) {
    use crossterm::event::{Event, KeyCode, KeyModifiers, poll, read};

    if poll(std::time::Duration::from_millis(0)).unwrap_or(false)
        && let Ok(Event::Key(key)) = read()
        && ((key.code == KeyCode::Char('q') || key.code == KeyCode::Char('Q'))
            || (key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)))
    {
        exit.write(bevy::app::AppExit::Success);
    }
}

// Cleanup system that runs on exit to restore terminal state to cooked if we
// were in raw mode originally and since crossterm? I think kills the tty cursor
// restore that too.
fn cleanup_on_exit(mut exit_reader: MessageReader<bevy::app::AppExit>) {
    for _event in exit_reader.read() {
        if let Err(e) = crossterm::terminal::disable_raw_mode() {
            eprintln!(
                "failed to disable raw mode terminal needs manual restoration: {}",
                e
            );
        }

        if let Err(e) = crossterm::execute!(std::io::stdout(), crossterm::cursor::Show) {
            eprintln!("failed to show tty cursor I guess: {}", e);
        }
    }
}

use ansi_term::Style;

struct StyleManager {
    enabled: bool,
}

impl StyleManager {
    fn new(enabled: bool) -> Self {
        StyleManager { enabled }
    }

    fn apply(&self, text: &str, style: Style) -> String {
        if self.enabled {
            style.paint(text).to_string()
        } else {
            text.to_string()
        }
    }
}

// Update display with ANSI colors
#[allow(clippy::type_complexity)]
fn update_display(
    query: Query<
        (
            &ReplicatedSource,
            &ReplicatedDest,
            &ReplicatedUuid,
            Option<&ReplicatedIoProgress>,
            Option<&ReplicatedSyncComplete>,
            Option<&ReplicatedCompletionTime>,
            Option<&ReplicatedSyncStartTime>,
            Option<&ReplicatedSyncStopTime>,
        ),
        With<ReplicatedSimpleCopy>,
    >,
    stats_query: Query<(&Uptime, &Mem, &Cpu)>,
) {
    // Detect if stdout is in raw mode by checking if it's a TTY
    // When using crossterm for input, we're in raw mode and need \r\n
    let nl = if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        "\r\n"
    } else {
        "\n"
    };

    let manager = if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
        StyleManager::new(true)
    } else {
        StyleManager::new(false)
    };

    // TODO: better output display not sure how to do this better now. Is there
    // a way to map components in Bevy into a struct I can have a Display setup
    // on?
    let mut output = String::new();

    // TODO: wtf works, this hack isn't great and I probably should use some
    // sorta crate for the escape code bs.
    //
    // What I need to brain out is how I can get tracing-indicatif to respect
    // the log lines somehow. Its not pretty for now but whatever it works.
    //    print!("\x1B[2J\x1B[1;1H");
    //    print!("{esc}c", esc = 27 as char);

    // Build stats line
    if let Ok((uptime, mem, cpu)) = stats_query.single()
        && **uptime > 0
    {
        let uptime_fmt = humantime::format_duration(std::time::Duration::from_secs(**uptime));
        let memory = humansize::format_size(**mem, humansize::BINARY);
        output.push_str(&manager.apply(
            &format!(
                "daemon stats: up: {} mem: {} cpu: {:.1}%{nl}",
                uptime_fmt, memory, **cpu
            ),
            ansi_term::Style::default().bold(),
        ));
    } else {
        output.push_str(&manager.apply(
            &format!("daemon stats not available?{nl}"),
            ansi_term::Style::default(),
        ));
    }

    let mut in_progress = Vec::new();
    let mut completed = Vec::new();

    for (source, dest, uuid, io_progress, complete, completion_time, start_time, stop_time) in
        query.iter()
    {
        if complete.is_some() {
            completed.push((source, dest, uuid, completion_time, start_time, stop_time));
        } else {
            in_progress.push((source, dest, uuid, io_progress, start_time));
        }
    }

    let total = in_progress.len() + completed.len();

    if total == 0 {
        output.push_str(&manager.apply(
            &format!("no syncs{nl}"),
            ansi_term::Style::default().fg(ansi_term::Colour::Yellow),
        ));
    } else {
        if in_progress.is_empty() {
            output.push_str(&manager.apply(
                &format!("no active syncs{nl}"),
                ansi_term::Style::default().fg(ansi_term::Colour::Yellow),
            ));
        } else {
            output.push_str(&manager.apply(
                &format!("active syncs: {}{nl}", in_progress.len()),
                ansi_term::Style::default().fg(ansi_term::Colour::Green),
            ));
        }
        if !completed.is_empty() {
            output.push_str(&manager.apply(
                &format!("completed syncs: {}{nl}", completed.len()),
                ansi_term::Style::default().fg(ansi_term::Colour::Green),
            ));
        }

        // in progress stuff
        for (source, dest, uuid, io_progress, start_time) in in_progress {
            let uuid_str = uuid::Uuid::from_u128(uuid.0);
            let running_for = if let Some(st) = start_time {
                let current_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let elapsed = current_secs.saturating_sub(st.started_secs);
                format!(
                    "{}",
                    humantime::format_duration(std::time::Duration::from_secs(elapsed))
                )
            } else {
                "?".to_string()
            };

            // output.push_str(&manager.apply(
            //     "in progress",
            //     ansi_term::Style::default().fg(ansi_term::Colour::Green),
            // ));

            // Add progress information if available
            let progress_str = if let Some(progress) = io_progress {
                if progress.files_found > 0 {
                    let bytes_written_str =
                        humansize::format_size(progress.bytes_written, humansize::BINARY);
                    let total_size_str =
                        humansize::format_size(progress.total_size, humansize::BINARY);

                    let mut extras = Vec::new();
                    if progress.error_count > 0 {
                        extras.push(format!("{} errors", progress.error_count));
                    }
                    if progress.skipped_count > 0 {
                        extras.push(format!("{} skipped", progress.skipped_count));
                    }

                    // Format throughput
                    let throughput_str = if progress.throughput_bps > 0.0 {
                        format!(
                            " @ {}/s",
                            humansize::format_size(
                                progress.throughput_bps as u64,
                                humansize::BINARY
                            )
                        )
                    } else {
                        String::new()
                    };

                    let extras_str = if !extras.is_empty() {
                        format!(" {}", extras.join(", "))
                    } else {
                        String::new()
                    };

                    format!(
                        " files: {}/{} dirs: {}/{} {}/{} ~{:.1}%{}{}",
                        progress.files_written,
                        progress.files_found,
                        progress.dirs_written,
                        progress.dirs_found,
                        bytes_written_str,
                        total_size_str,
                        progress.completion_percent,
                        throughput_str,
                        extras_str
                    )
                } else {
                    " [scanning...]".to_string()
                }
            } else {
                String::new()
            };

            output.push_str(&manager.apply(
                &format!(
                    " {}{nl} {}{nl}   {} → {} {}{nl}",
                    uuid_str,
                    progress_str,
                    source.0.display(),
                    dest.0.display(),
                    running_for
                ),
                ansi_term::Style::default(),
            ));
        }

        // completed crap
        for (source, dest, uuid, completion_time, start_time, stop_time) in completed {
            let uuid_str = uuid::Uuid::from_u128(uuid.0);
            let time_ago = if let Some(ct) = completion_time {
                let current_secs = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();
                let elapsed = current_secs.saturating_sub(ct.completed_secs);
                format!(
                    "{} ago",
                    humantime::format_duration(std::time::Duration::from_secs(elapsed))
                )
            } else {
                "?".to_string()
            };

            let duration_str = if let (Some(start), Some(stop)) = (start_time, stop_time) {
                let duration_secs = stop.stopped_secs.saturating_sub(start.started_secs);
                format!(
                    "took {}",
                    humantime::format_duration(std::time::Duration::from_secs(duration_secs))
                )
            } else {
                String::new()
            };

            output.push_str(&manager.apply("completed", ansi_term::Style::default()));
            output.push_str(&manager.apply(
                &format!(
                    " {}: {} {} → {} {}{nl}",
                    time_ago,
                    duration_str,
                    source.0.display(),
                    dest.0.display(),
                    uuid_str,
                ),
                ansi_term::Style::default(),
            ));
        }
    }
    output.push_str(&manager.apply("", ansi_term::Style::default()));
    tracing_indicatif::indicatif_println!("{nl}{}", output);
}
