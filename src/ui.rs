use crate::app::App;
use crate::types::{fmt_time, Focus};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, ListState, Padding, Paragraph, Wrap},
    Frame,
};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // search bar
            Constraint::Min(8),    // results + playlist
            Constraint::Length(5), // now playing
            Constraint::Length(1), // status / help line
        ])
        .split(area);

    draw_search(f, app, root[0]);
    draw_main(f, app, root[1]);
    draw_now_playing(f, app, root[2]);
    draw_status(f, app, root[3]);

    if app.show_help {
        draw_help(f, area);
    }
}

fn draw_search(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Search;
    let title = if app.searching {
        " Search  (searching…) "
    } else {
        " Search "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border_style(focused));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let prefix = if focused { "▶ " } else { "  " };
    let cursor = if focused { "_" } else { "" };
    let line = Line::from(vec![
        Span::styled(prefix, Style::default().fg(Color::Yellow)),
        Span::raw(app.query.as_str()),
        Span::styled(cursor, Style::default().add_modifier(Modifier::SLOW_BLINK)),
    ]);
    let p = Paragraph::new(line);
    f.render_widget(p, inner);
}

fn draw_main(f: &mut Frame, app: &App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);
    draw_results(f, app, cols[0]);
    draw_playlist(f, app, cols[1]);
}

fn draw_results(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Results;
    let title = format!(" Results ({}) ", app.results.len());
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border_style(focused));

    let width = block.inner(area).width as usize;
    let items: Vec<ListItem> = app
        .results
        .iter()
        .enumerate()
        .map(|(i, t)| result_row(i, t, width))
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight_style(focused))
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    if !app.results.is_empty() {
        state.select(Some(app.results_sel.min(app.results.len() - 1)));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn result_row<'a>(i: usize, t: &'a crate::types::Track, width: usize) -> ListItem<'a> {
    let dur = t.duration_str();
    let idx = format!("{:>3}. ", i + 1);
    // Reserve space for prefix/duration; truncate title+channel to fit.
    let dur_part = format!("  {}", dur);
    let reserved = idx.len() + dur_part.len() + 2 /* ▶  highlight symbol */;
    let avail = width.saturating_sub(reserved).max(10);
    let mut meta = String::new();
    if !t.channel.is_empty() {
        meta = format!(" — {}", t.channel);
    }
    let text = format!("{}{}", t.title, meta);
    let title = truncate_display(&text, avail);
    let line = Line::from(vec![
        Span::styled(idx, Style::default().fg(Color::DarkGray)),
        Span::raw(title),
        Span::styled(dur_part, Style::default().fg(Color::DarkGray)),
    ]);
    ListItem::new(line)
}

fn draw_playlist(f: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Focus::Playlist;
    let title = format!(
        " Playlist ({}/{}) ",
        app.current.map(|i| i + 1).unwrap_or(0),
        app.playlist.len()
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(border_style(focused));

    let width = block.inner(area).width as usize;
    let items: Vec<ListItem> = app
        .playlist
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let is_current = app.current == Some(i);
            let marker = if is_current { "♪ " } else { "  " };
            let idx = format!("{:>3}. ", i + 1);
            let dur = format!("  {}", t.duration_str());
            let reserved = marker.len() + idx.len() + dur.len() + 2;
            let avail = width.saturating_sub(reserved).max(10);
            let title = truncate_display(&t.title, avail);
            let title_style = if is_current {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let line = Line::from(vec![
                Span::styled(
                    marker,
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(idx, Style::default().fg(Color::DarkGray)),
                Span::styled(title, title_style),
                Span::styled(dur, Style::default().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(block)
        .highlight_style(highlight_style(focused))
        .highlight_symbol("▶ ");

    let mut state = ListState::default();
    if !app.playlist.is_empty() {
        state.select(Some(app.playlist_sel.min(app.playlist.len() - 1)));
    }
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_now_playing(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Now Playing ")
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);

    let (title_text, channel_text) = match app.current.and_then(|i| app.playlist.get(i)) {
        Some(t) => (t.title.clone(), t.channel.clone()),
        None => ("—".into(), String::new()),
    };

    let state_glyph = if app.current.is_none() {
        "■"
    } else if app.loading {
        "…"
    } else if app.paused {
        "❚❚"
    } else {
        "▶"
    };

    let header = Line::from(vec![
        Span::styled(
            format!("{} ", state_glyph),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(title_text, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(channel_text, Style::default().fg(Color::DarkGray)),
    ]);
    f.render_widget(Paragraph::new(header), rows[0]);

    let (pos, dur) = (app.time_pos, app.duration);
    let ratio = if dur.is_finite() && dur > 0.0 && pos.is_finite() && pos >= 0.0 {
        (pos / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let label = format!(
        "{} / {}",
        if pos.is_finite() && pos >= 0.0 {
            fmt_time(pos)
        } else {
            "--:--".into()
        },
        if dur.is_finite() && dur > 0.0 {
            fmt_time(dur)
        } else {
            "--:--".into()
        }
    );
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(Color::Cyan).bg(Color::Black))
        .ratio(ratio)
        .label(label);
    f.render_widget(gauge, rows[1]);

    let info = Line::from(vec![
        Span::raw("vol "),
        Span::styled(
            format!("{:>3.0}%", app.volume),
            Style::default().fg(Color::Yellow),
        ),
        Span::raw("    loop "),
        Span::styled(app.loop_mode.label(), Style::default().fg(Color::Magenta)),
        Span::raw("    "),
        Span::styled(
            if app.current.is_none() {
                "idle"
            } else if app.loading {
                "loading…"
            } else if app.paused {
                "paused"
            } else {
                "playing"
            },
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(Paragraph::new(info), rows[2]);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let help = " /:search  Tab:focus  j/k:move  Enter:play  a:add  m:more  dd:del  Space/p:pause  n/N:next/prev  </>/seek  l:loop  +/-:vol  ?:help  q:quit ";
    let line = if let Some(s) = app.status_text() {
        Line::from(vec![Span::styled(
            format!(" {} ", s),
            Style::default().fg(Color::Black).bg(Color::Yellow),
        )])
    } else if let Some(pending_couint) = app.pending_count {
        Line::from(Span::styled(
            format!("{}", pending_couint),
            Style::default().fg(Color::DarkGray),
        ))
    } else {
        Line::from(Span::styled(help, Style::default().fg(Color::DarkGray)))
    };
    f.render_widget(Paragraph::new(line).alignment(Alignment::Left), area);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let w = area.width.saturating_sub(8).min(78);
    let h = area.height.saturating_sub(4).min(22);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help — vim-ish keys ")
        .padding(Padding::uniform(1));

    let lines = vec![
        Line::from("  /                 focus search box"),
        Line::from("  Tab               cycle focus (Results ↔ Playlist)"),
        Line::from(""),
        Line::from("  j / k    Down/Up  move selection"),
        Line::from("  gg  /  G          jump top / bottom"),
        Line::from("  Ctrl-d / Ctrl-u   page down / up"),
        Line::from(""),
        Line::from("  Enter             results: add+play  |  playlist: play"),
        Line::from("  a                 add highlighted result to playlist"),
        Line::from("  A                 add all results to playlist"),
        Line::from("  m                 load 30 more results for the same query"),
        Line::from("  dd                delete from playlist"),
        Line::from("  C                 clear playlist"),
        Line::from(""),
        Line::from("  Space  /  p       play / pause"),
        Line::from("  n  /  N           next / previous"),
        Line::from("  >  /  <           seek +10s / -10s  (hold: +60s / -60s)"),
        Line::from("  l                 cycle loop  (off → list → one → off)"),
        Line::from("  r                 toggle repeat-one"),
        Line::from("  + / -             volume up / down"),
        Line::from(""),
        Line::from("  ?                 toggle this help     q / Esc  quit"),
    ];

    f.render_widget(Clear, rect);
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        rect,
    );
}

fn border_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn highlight_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .bg(Color::Rgb(40, 50, 70))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().bg(Color::Rgb(25, 28, 35))
    }
}

fn truncate_display(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let take = max.saturating_sub(1);
        let mut out: String = s.chars().take(take).collect();
        out.push('…');
        out
    }
}
