use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Gauge, Paragraph};
use ratatui::Frame;

use crate::playback::state::PlaybackState;
use crate::tui::app::{App, SEEK_SCALES};

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Status
            Constraint::Length(3), // Buffer gauge
            Constraint::Length(4), // Level meters
            Constraint::Length(3), // Device info
            Constraint::Length(3), // Keys
            Constraint::Min(0),    // Spacer
        ])
        .split(area);

    draw_status(frame, chunks[0], app);
    draw_buffer_gauge(frame, chunks[1], app);
    draw_levels(frame, chunks[2], app);
    draw_device_info(frame, chunks[3], app);
    draw_keys(frame, chunks[4], app);

    if app.show_help {
        draw_help_overlay(frame, area);
    }
}

fn draw_status(frame: &mut Frame, area: Rect, app: &App) {
    let state = app.controller.state();
    let delay_s = app.controller.delay_ms() / 1000.0;
    let usage = app.controller.buffer_usage() * 100.0;
    let scale_label = SEEK_SCALES[app.seek_scale_index].1;

    let state_style = match state {
        PlaybackState::Live => Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
        PlaybackState::Paused => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        PlaybackState::TimeShifted => Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    };

    let line = Line::from(vec![
        Span::raw("  State: "),
        Span::styled(format!("{} {}", state.symbol(), state.label()), state_style),
        Span::raw(format!(
            "{:width$}Delay: {delay_s:>6.3}s",
            "",
            width = 14 - state.label().len()
        )),
        Span::raw(format!("   Buf: {usage:>3.0}%")),
        Span::raw(format!("   Vol: {:>3.0}%", app.controller.volume() * 100.0)),
        Span::raw(format!("   Step: {scale_label:>4}")),
    ]);

    let block = Block::default().borders(Borders::ALL).title(" Shifter ");
    let paragraph = Paragraph::new(line).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_buffer_gauge(frame: &mut Frame, area: Rect, app: &App) {
    let usage = app.controller.buffer_usage();
    let buf_max = app.buffer_seconds as f64;
    let delay_s = (app.controller.delay_ms() / 1000.0).min(buf_max);

    let color = if usage > 0.9 {
        Color::Red
    } else if usage > 0.7 {
        Color::Yellow
    } else {
        Color::Blue
    };

    let gauge = Gauge::default()
        .block(Block::default().borders(Borders::ALL).title(" Buffer "))
        .gauge_style(Style::default().fg(color).bg(Color::DarkGray))
        .ratio(usage.clamp(0.0, 1.0))
        .label(format!("{delay_s:.1}s / {buf_max:.0}s"));

    frame.render_widget(gauge, area);
}

fn draw_levels(frame: &mut Frame, area: Rect, app: &App) {
    let (peak_l, peak_r) = app.controller.peak_levels();

    let block = Block::default().borders(Borders::ALL).title(" Levels ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    draw_meter(frame, rows[0], "L", peak_l);
    draw_meter(frame, rows[1], "R", peak_r);
}

fn draw_meter(frame: &mut Frame, area: Rect, label: &str, peak: f32) {
    let db = if peak > 0.0001 {
        20.0 * peak.log10()
    } else {
        -96.0
    };

    // Map -60dB..0dB to 0.0..1.0
    let ratio = ((db + 60.0) / 60.0).clamp(0.0, 1.0) as f64;

    let color = if db > -3.0 {
        Color::Red
    } else if db > -12.0 {
        Color::Yellow
    } else {
        Color::Green
    };

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(8),
        ])
        .split(area);

    let lbl = Paragraph::new(format!(" {label}"));
    frame.render_widget(lbl, cols[0]);

    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(color).bg(Color::DarkGray))
        .ratio(ratio);
    frame.render_widget(gauge, cols[1]);

    let db_text = Paragraph::new(format!(" {db:>5.0} dB"));
    frame.render_widget(db_text, cols[2]);
}

fn draw_device_info(frame: &mut Frame, area: Rect, app: &App) {
    let line = Line::from(format!(
        "  In: {}    Out: {}",
        app.input_device_name, app.output_device_name
    ));

    let block = Block::default().borders(Borders::ALL).title(" Devices ");
    let paragraph = Paragraph::new(line).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_keys(frame: &mut Frame, area: Rect, app: &App) {
    let _ = app;
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let line = Line::from(vec![
        Span::raw("  "),
        Span::styled("Space", bold),
        Span::raw(":pause  "),
        Span::styled("\u{2190}/\u{2192}", bold),
        Span::raw(":seek  "),
        Span::styled("1-9", bold),
        Span::raw(":scale  "),
        Span::styled("\u{2191}/\u{2193}", bold),
        Span::raw(":vol  "),
        Span::styled("L", bold),
        Span::raw(":live  "),
        Span::styled("H", bold),
        Span::raw(":help  "),
        Span::styled("Q", bold),
        Span::raw(":quit"),
    ]);

    let block = Block::default().borders(Borders::ALL).title(" Keys ");
    let paragraph = Paragraph::new(line).block(block);
    frame.render_widget(paragraph, area);
}

fn draw_help_overlay(frame: &mut Frame, area: Rect) {
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("  Space       ", bold),
            Span::raw("Pause / Resume playback"),
        ]),
        Line::from(vec![
            Span::styled("  \u{2190} / \u{2192}       ", bold),
            Span::raw("Seek backward / forward by current step"),
        ]),
        Line::from(vec![
            Span::styled("  1-9         ", bold),
            Span::raw("Seek step: 1ms 10ms 100ms 500ms 1s 2s 5s 10s 30s"),
        ]),
        Line::from(vec![
            Span::styled("  \u{2191} / \u{2193}       ", bold),
            Span::raw("Volume up / down (5% steps, max 150%)"),
        ]),
        Line::from(vec![
            Span::styled("  L           ", bold),
            Span::raw("Jump to live"),
        ]),
        Line::from(vec![
            Span::styled("  H           ", bold),
            Span::raw("Toggle this help"),
        ]),
        Line::from(vec![
            Span::styled("  Q           ", bold),
            Span::raw("Quit"),
        ]),
        Line::from(""),
    ];

    let height = lines.len() as u16 + 2; // +2 for border
    let width = 78;
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect::new(x, y, width.min(area.width), height.min(area.height));

    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help ")
        .border_style(Style::default().fg(Color::Cyan));
    let paragraph = Paragraph::new(lines).block(block);
    frame.render_widget(paragraph, popup);
}
