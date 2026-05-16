pub mod render;

use std::io;
use std::time::Duration;

use anyhow::Context;
use crossterm::{
    event::{self, Event, KeyCode},
    terminal, ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};

use crate::state::AppState;

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

pub enum TuiEvent {
    KeyPress(char),
    Quit,
}

impl Tui {
    /// Enable raw mode, enter alternate screen, create Terminal.
    pub fn new() -> anyhow::Result<Self> {
        terminal::enable_raw_mode().context("enable raw mode")?;
        let mut stdout = io::stdout();
        stdout
            .execute(terminal::EnterAlternateScreen)
            .context("enter alternate screen")?;

        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend).context("create terminal")?;

        Ok(Self { terminal })
    }

    /// Draw the current state.
    pub fn render(&mut self, state: &AppState) -> anyhow::Result<()> {
        self.terminal
            .draw(|f| render::draw(f, state))
            .context("terminal draw")?;
        Ok(())
    }

    /// Non-blocking event poll (Duration::ZERO).
    /// - `q` / Esc → `Quit`
    /// - other chars → `KeyPress(c)`
    /// - otherwise → `None`
    pub fn next_event(&mut self) -> anyhow::Result<Option<TuiEvent>> {
        if event::poll(Duration::ZERO).context("event poll")? {
            match event::read().context("event read")? {
                Event::Key(key_event) => match key_event.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(Some(TuiEvent::Quit)),
                    KeyCode::Char(c) => return Ok(Some(TuiEvent::KeyPress(c))),
                    _ => {}
                },
                _ => {}
            }
        }
        Ok(None)
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        // Best-effort cleanup — ignore errors during drop.
        let _ = terminal::disable_raw_mode();
        let _ = self
            .terminal
            .backend_mut()
            .execute(terminal::LeaveAlternateScreen);
    }
}
