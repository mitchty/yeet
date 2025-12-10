// Shamelessly stolen from https://github.com/scurvydoggo/bevy_stdin only cause
// when I added it as a dependency it came along with 300 dependencies and
// yeeted in alsa and other stuff that really shouldn't apply to reading a tty for keyboard character codes.
//
// Adding crossbeam_channel/crossterm in directly makes this work ezpz and only
// adds minimal deps aka not alsa and other shared library stuff I don't want.
//
// This specific code should be considered MIT licensed.

//! Terminal input for the [Bevy game engine](https://bevy.org/), using
//! [crossterm](https://docs.rs/crossterm/latest/crossterm/) for cross-platform support.
//!
//! Input is exposed via resources: `ButtonInput<KeyCode>` and `ButtonInput<KeyModifiers>`.

use bevy::{app::AppExit, input::ButtonInput, prelude::*};
use crossbeam_channel::{Receiver, bounded};
use crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::thread;
use std::time::Duration;

/// Adds terminal input to an App
pub struct StdinPlugin;

/// Restore terminal state on shutdown
impl Drop for StdinPlugin {
    fn drop(&mut self) {
        // We only disable raw mode if we're in a tty terminal not daemon context
        if atty::is(atty::Stream::Stdin) {
            let _ = crossterm::terminal::disable_raw_mode();
        }
    }
}

#[derive(Message, Deref)]
struct StdinEvent(KeyEvent);

#[derive(Resource, Deref)]
struct StreamReceiver(Receiver<StdinEvent>);

impl Plugin for StdinPlugin {
    fn build(&self, app: &mut App) {
        let is_tty = atty::is(atty::Stream::Stdin);

        app.insert_resource(ButtonInput::<KeyCode>::default());
        app.insert_resource(ButtonInput::<KeyModifiers>::default());

        // If we're running as a daemon in say systemd/launchd stdin won't be a tty, no need for this
        if is_tty {
            app.add_systems(Startup, setup);
            app.add_systems(PreUpdate, read_stream);
            app.add_systems(Update, ctrl_c);
        }
    }
}

fn setup(mut commands: Commands) {
    let (tx, rx) = bounded::<StdinEvent>(1);
    commands.insert_resource(StreamReceiver(rx));

    // Raw mode is necessary to read key events without waiting for Enter
    if !atty::is(atty::Stream::Stdin) {
        warn!("setup called without TTY - skipping raw mode, possible bug?");
        return;
    }

    crossterm::terminal::enable_raw_mode().expect("Failed to enable raw mode");

    thread::spawn(move || {
        let timeout = Duration::from_millis(100);
        loop {
            if event::poll(timeout).expect("Failed to poll stdin") {
                let e = event::read().expect("Failed to read stdin event");
                if let event::Event::Key(key) = e {
                    tx.send(StdinEvent(key))
                        .expect("Failed to transmit key event");
                }
            }
        }
    });
}

// This system reads from the channel and submits key events to bevy
fn read_stream(
    stdin_keys: Res<StreamReceiver>,
    mut key_input: ResMut<ButtonInput<KeyCode>>,
    mut modifier_input: ResMut<ButtonInput<KeyModifiers>>,
) {
    key_input.reset_all();
    modifier_input.reset_all();

    for key in stdin_keys.try_iter() {
        match key.kind {
            KeyEventKind::Press => {
                key_input.press(key.code);
                modifier_input.press(key.modifiers);
            }
            KeyEventKind::Release => {
                key_input.release(key.code);
                modifier_input.release(key.modifiers);
            }
            KeyEventKind::Repeat => {}
        }
    }
}

/// Monitor for Ctrl+C and shut down bevy
fn ctrl_c(
    key: Res<ButtonInput<KeyCode>>,
    modifier: Res<ButtonInput<KeyModifiers>>,
    mut ev_exit: MessageWriter<AppExit>,
) {
    if modifier.just_pressed(KeyModifiers::CONTROL) && key.just_pressed(KeyCode::Char('c')) {
        ev_exit.write(AppExit::Success);
    }
}
