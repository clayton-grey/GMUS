use std::io;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{
    self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture, Event,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use rusqlite::Connection;

use crate::config::AppPaths;
use crate::player::PlaybackState;

use super::renderer::render;
use super::App;

const INTEGRATION_TICK: Duration = Duration::from_millis(75);

pub fn run(conn: &Connection, paths: &AppPaths) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let mut app = match App::new(conn, paths) {
        Ok(app) => app,
        Err(error) => {
            let _ = restore_terminal(&mut terminal);
            return Err(error);
        }
    };
    let result = run_loop(&mut terminal, conn, &mut app);
    let shutdown_result = app.shutdown(conn);
    let restore_result = restore_terminal(&mut terminal);
    result.and(shutdown_result).and(restore_result)
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    conn: &Connection,
    app: &mut App,
) -> Result<()> {
    let mut needs_draw = true;
    let mut last_render_position_s = None;
    let mut next_tick = Instant::now();
    let mut next_integration_tick = Instant::now();

    loop {
        let now = Instant::now();
        if now >= next_integration_tick {
            app.integration.tick();
            let handled_integration_command = app.handle_integration_commands(conn)?;
            needs_draw |= handled_integration_command;
            if handled_integration_command {
                next_tick = now;
            }
            next_integration_tick = Instant::now() + INTEGRATION_TICK;
        }

        if now >= next_tick {
            needs_draw |= app.expire_transient_status();
            needs_draw |= app.update_playback(conn)?;

            if app.current.is_some() {
                let position_s = app.current_position_ms() / 1000;
                if app.logical_state() == PlaybackState::Playing
                    && last_render_position_s != Some(position_s)
                {
                    needs_draw = true;
                }
            }

            next_tick = Instant::now() + app.tick_interval();
        }

        if needs_draw {
            terminal.draw(|frame| render(frame, app))?;
            last_render_position_s = app
                .current
                .as_ref()
                .map(|_| app.current_position_ms() / 1000);
            needs_draw = false;
        }

        if app.poll_library_job(conn)? {
            needs_draw = true;
            next_tick = Instant::now();
            continue;
        }

        let input_wait = next_tick
            .min(next_integration_tick)
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);
        if event::poll(input_wait)? {
            match event::read()? {
                Event::Key(key) => {
                    if app.handle_key(conn, key)? {
                        break;
                    }
                    needs_draw = true;
                    next_tick = Instant::now();
                }
                Event::FocusGained => {
                    terminal.clear()?;
                    needs_draw = true;
                    next_tick = Instant::now();
                }
                Event::Resize(_, _) => needs_draw = true,
                Event::Mouse(mouse) => {
                    let size = terminal.size()?;
                    if app.handle_mouse(mouse, size.width, size.height) {
                        needs_draw = true;
                        next_tick = Instant::now();
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableFocusChange,
        EnableMouseCapture
    )?;
    Terminal::new(CrosstermBackend::new(stdout)).map_err(Into::into)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        DisableFocusChange,
        LeaveAlternateScreen
    )?;
    terminal.show_cursor()?;
    Ok(())
}
