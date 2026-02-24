use anyhow::Result;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::{StreamExt};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState, Wrap},
    Terminal,
};
use std::io::{self, Stdout};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct TableCell {
    pub text: String,
    pub style: Style,
}

impl TableCell {
    pub fn plain<T: Into<String>>(t: T) -> Self {
        Self {
            text: t.into(),
            style: Style::default(),
        }
    }

    pub fn green<T: Into<String>>(t: T) -> Self {
        Self {
            text: t.into(),
            style: Style::default().fg(Color::Green),
        }
    }

    pub fn red<T: Into<String>>(t: T) -> Self {
        Self {
            text: t.into(),
            style: Style::default().fg(Color::Red),
        }
    }

    pub fn yellow<T: Into<String>>(t: T) -> Self {
        Self {
            text: t.into(),
            style: Style::default().fg(Color::Yellow),
        }
    }

    pub fn dim<T: Into<String>>(t: T) -> Self {
        Self {
            text: t.into(),
            style: Style::default().fg(Color::DarkGray),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TableView {
    pub title: String,
    pub help: String,
    pub status: String,
    pub headers: Vec<String>,
    pub widths: Vec<Constraint>,
    pub rows: Vec<Vec<TableCell>>,
}

/// Minimal watch-mode TUI runner.
///
/// - Updates when new `TableView` arrives on `rx`.
/// - Exits on `q` / `Esc` / Ctrl-C.
pub async fn run_table(mut rx: mpsc::Receiver<TableView>) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut latest: Option<TableView> = None;
    let mut table_state = TableState::default();
    table_state.select(Some(0));

    let mut events = EventStream::new();

    let res: Result<()> = loop {
        // Drain updates quickly.
        while let Ok(v) = rx.try_recv() {
            latest = Some(v);
        }

        terminal.draw(|f| {
            let size = f.area();
            let view = latest.clone().unwrap_or_else(|| TableView {
                title: "hl watch".into(),
                help: "q/Esc to quit".into(),
                status: "waiting for data…".into(),
                headers: vec!["".into()],
                widths: vec![Constraint::Percentage(100)],
                rows: vec![],
            });
            draw_table(f, size, &view, &mut table_state);
        })?;

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                break Ok(());
            }
            maybe = events.next() => {
                if let Some(Ok(ev)) = maybe {
                    if handle_key(ev, &mut table_state) {
                        break Ok(());
                    }
                }
            }
            maybe_view = rx.recv() => {
                if maybe_view.is_none() {
                    break Ok(());
                }
                latest = maybe_view;
            }
        }
    };

    restore_terminal(terminal)?;
    res
}

fn handle_key(ev: Event, table_state: &mut TableState) -> bool {
    match ev {
        Event::Key(k) if k.kind == KeyEventKind::Press => {
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => return true,
                KeyCode::Down | KeyCode::Char('j') => {
                    let next = match table_state.selected() {
                        Some(i) => i.saturating_add(1),
                        None => 0,
                    };
                    table_state.select(Some(next));
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    let prev = match table_state.selected() {
                        Some(i) => i.saturating_sub(1),
                        None => 0,
                    };
                    table_state.select(Some(prev));
                }
                _ => {}
            }
        }
        _ => {}
    }
    false
}

fn draw_table(
    f: &mut ratatui::Frame,
    area: Rect,
    view: &TableView,
    table_state: &mut TableState,
) {
    let chunks = Layout::vertical([
        Constraint::Min(2),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(area);

    let header_style = Style::default().add_modifier(Modifier::BOLD);

    let header = Row::new(
        view.headers
            .iter()
            .map(|h| Cell::from(h.clone()).style(header_style)),
    )
    .style(header_style)
    .height(1);

    let rows = view.rows.iter().map(|r| {
        Row::new(r.iter().map(|c| Cell::from(c.text.clone()).style(c.style)))
    });

    let t = Table::new(rows, view.widths.clone())
        .header(header)
        .block(Block::default().title(view.title.clone()).borders(Borders::ALL))
        .highlight_style(Style::default().bg(Color::DarkGray))
        .highlight_symbol("➤ ");

    f.render_stateful_widget(t, chunks[0], table_state);

    let status = Paragraph::new(Line::from(vec![
        Span::styled("status: ", Style::default().fg(Color::DarkGray)),
        Span::raw(view.status.clone()),
    ]))
    .wrap(Wrap { trim: true });
    f.render_widget(status, chunks[1]);

    let help = Paragraph::new(Line::from(vec![
        Span::styled("keys: ", Style::default().fg(Color::DarkGray)),
        Span::raw(view.help.clone()),
    ]))
    .wrap(Wrap { trim: true });
    f.render_widget(help, chunks[2]);
}

fn restore_terminal(mut terminal: Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
