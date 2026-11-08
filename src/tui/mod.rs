//! Interactive terminal UI. Reads latest.json/history.json and refreshes in
//! the background, so it opens instantly and never blocks on the network.

mod app;
mod theme;
mod ui;

use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

use app::{App, OpenTarget, View};

pub fn run() -> Result<()> {
    let theme = theme::Theme::from_cybercore();
    let mut app = App::new()?;
    // ratatui::init installs a panic hook that restores the terminal.
    let mut terminal = ratatui::init();
    let result = (|| -> Result<()> {
        while !app.quit {
            app.poll();
            terminal.draw(|f| ui::draw(f, &mut app, &theme))?;
            if event::poll(Duration::from_millis(150))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                handle_key(&mut app, key.code, key.modifiers);
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        app.quit = true;
        return;
    }
    if app.help {
        app.help = false;
        return;
    }
    match code {
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left if app.view == View::Detail => {
            app.view = View::List
        }
        KeyCode::Esc => app.quit = true,
        KeyCode::Down | KeyCode::Char('j') => app.move_by(1),
        KeyCode::Up | KeyCode::Char('k') => app.move_by(-1),
        KeyCode::Home => app.select_edge(false),
        KeyCode::End => app.select_edge(true),
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right if app.selected().is_some() => {
            app.view = View::Detail
        }
        KeyCode::Char('r') => app.refresh(),
        KeyCode::Char('o') => app.open(OpenTarget::Aur),
        KeyCode::Char('g') => app.open(OpenTarget::GitHub),
        KeyCode::Char('u') => app.open(OpenTarget::Upstream),
        KeyCode::Char('?') => app.help = true,
        _ => {}
    }
}
