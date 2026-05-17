use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::state::AppState;

pub fn draw(f: &mut Frame, state: &AppState) {
    let area = f.size();

    // -----------------------------------------------------------------------
    // Root layout: top bar (3) | body (fill) | input bar (3)
    // -----------------------------------------------------------------------
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(area);

    draw_top_bar(f, root[0], state);

    // Body: left 60% | right 40%
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(root[1]);

    draw_left_column(f, body[0], state);
    draw_right_column(f, body[1], state);
    draw_input_bar(f, root[2], state);
}

// ---------------------------------------------------------------------------
// Top bar
// ---------------------------------------------------------------------------

fn draw_top_bar(f: &mut Frame, area: Rect, state: &AppState) {
    let olsr_str = match state.olsr_converged_in_ms {
        Some(ms) => format!("{ms}"),
        None => "---".to_string(),
    };
    let q = &state.queues;
    let text = format!(
        " NODE: {}  EPOCH: {}  CH: {}  OLSR: {}ms  Q: C={} N={} B={}",
        state.node_id, state.epoch, state.channel, olsr_str,
        q.critical, q.normal, q.bulk,
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled("TACTICAL MESH", Style::default().fg(Color::Cyan)));

    let para = Paragraph::new(text).block(block);
    f.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// Left column
// ---------------------------------------------------------------------------

fn draw_left_column(f: &mut Frame, area: Rect, state: &AppState) {
    if state.topology_edges.is_empty() {
        // routing 30% | neighbors 15% | targets 30% | counters (remainder)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Percentage(15),
                Constraint::Percentage(30),
                Constraint::Min(0),
            ])
            .split(area);

        draw_routing_table(f, chunks[0], state);
        draw_neighbors(f, chunks[1], state);
        draw_targets(f, chunks[2], state);
        draw_attack_counters(f, chunks[3], state);
    } else {
        // routing 25% | neighbors 15% | targets 25% | counters 15% | topology (remainder)
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(15),
                Constraint::Percentage(25),
                Constraint::Percentage(15),
                Constraint::Min(0),
            ])
            .split(area);

        draw_routing_table(f, chunks[0], state);
        draw_neighbors(f, chunks[1], state);
        draw_targets(f, chunks[2], state);
        draw_attack_counters(f, chunks[3], state);
        draw_topology(f, chunks[4], state);
    }
}

fn draw_routing_table(f: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .routing_table
        .iter()
        .map(|r| {
            ListItem::new(format!(
                " {:<12} via {:<12} cost={:>4} hops={}",
                r.dest, r.via, r.cost, r.hops
            ))
        })
        .collect();

    let block = default_block("ROUTING TABLE");
    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn draw_neighbors(f: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .neighbors
        .iter()
        .map(|n| {
            ListItem::new(format!(
                " {:<12} rssi: {:>4} dBm, last: {}ms ago",
                n.name, n.rssi, n.last_hello_ms
            ))
        })
        .collect();

    let block = default_block("NEIGHBORS");
    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn draw_targets(f: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .targets
        .iter()
        .map(|t| {
            let state_color = match t.state.as_str() {
                "DESTROYED" => Color::Red,
                "ENGAGED" => Color::Yellow,
                _ => Color::White,
            };
            let assigned = match t.assigned_to {
                Some(n) => format!("node-{n}"),
                None => "unassigned".to_string(),
            };
            let line = Line::from(vec![
                Span::raw(format!(" [{:>4}] ", t.id)),
                Span::raw(format!("{:<12} ", t.kind)),
                Span::styled(format!("{:<10} ", t.state), Style::default().fg(state_color)),
                Span::raw(format!("({:.4},{:.4}) {}", t.lat, t.lon, assigned)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let block = default_block("TARGETS");
    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn draw_attack_counters(f: &mut Frame, area: Rect, state: &AppState) {
    let c = &state.counters;

    let rows: &[(&str, u64)] = &[
        ("bad_sigs_dropped", c.bad_sigs_dropped),
        ("time_window_drops", c.time_window_drops),
        ("replayed_nonces", c.replayed_nonces),
        ("channel_hops", c.channel_hops),
        ("stream_rotations", c.stream_rotations),
        ("spoofed_frames_tx", c.spoofed_frames_tx),
        ("spoofed_frames_dropped", c.spoofed_frames_dropped),
    ];

    let lines: Vec<Line> = rows
        .iter()
        .map(|(label, value)| {
            let val_color = if *value > 0 { Color::Red } else { Color::White };
            Line::from(vec![
                Span::raw(format!(" {:<26}: ", label)),
                Span::styled(value.to_string(), Style::default().fg(val_color)),
            ])
        })
        .collect();

    let block = default_block("ATTACK COUNTERS");
    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
}

// ---------------------------------------------------------------------------
// Right column
// ---------------------------------------------------------------------------

fn draw_right_column(f: &mut Frame, area: Rect, state: &AppState) {
    // image (fills remaining) | log (fixed 35%)
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Percentage(35),
        ])
        .split(area);

    draw_image(f, chunks[0], state);
    draw_log(f, chunks[1], state);
}

fn draw_image(f: &mut Frame, area: Rect, state: &AppState) {
    let title: String = if let Some(ref rx) = state.image_rx {
        let pct = if rx.blocks_total > 0 {
            rx.blocks_done as u32 * 100 / rx.blocks_total as u32
        } else {
            0
        };
        format!(
            "IMAGE  target={} receiving {}/{} blocks ({}%)",
            rx.target_id, rx.blocks_done, rx.blocks_total, pct
        )
    } else if let Some(ref img) = state.image {
        if img.target_id == 0 {
            format!("IMAGE  local  {}×{}", img.width, img.height)
        } else {
            format!("IMAGE  target={}  {}×{}", img.target_id, img.width, img.height)
        }
    } else {
        "IMAGE".to_string()
    };

    let (border_color, title_color) = if state.image_rx.is_some() {
        (Color::Yellow, Color::Yellow)
    } else {
        (Color::DarkGray, Color::Cyan)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(title, Style::default().fg(title_color)));
    let inner = block.inner(area);

    let content: String = if let Some(img) = &state.image {
        pixels_to_block_chars(
            &img.pixels,
            img.width as usize,
            img.height as usize,
            inner.width as usize,
            inner.height as usize,
        )
    } else if state.image_rx.is_some() {
        "(receiving...)".to_string()
    } else {
        "(no image)".to_string()
    };

    let para = Paragraph::new(content).block(block);
    f.render_widget(para, area);
}

/// Scale a greyscale image to `cols×rows` and map each pixel to a block character.
/// 0=black → '█', 255=white → ' '.
fn pixels_to_block_chars(
    pixels: &[u8],
    img_w: usize,
    img_h: usize,
    cols: usize,
    rows: usize,
) -> String {
    const CHARS: [char; 5] = ['█', '▓', '▒', '░', ' '];
    let cols = cols.max(1);
    let rows = rows.max(1);
    let mut out = String::with_capacity(rows * (cols + 1));

    for row in 0..rows {
        for col in 0..cols {
            let px = (col * img_w / cols).min(img_w.saturating_sub(1));
            let py = (row * img_h / rows).min(img_h.saturating_sub(1));
            let brightness = pixels.get(py * img_w + px).copied().unwrap_or(0);
            let idx = (brightness as usize * (CHARS.len() - 1)) / 255;
            out.push(CHARS[idx]);
        }
        out.push('\n');
    }
    out
}

fn draw_log(f: &mut Frame, area: Rect, state: &AppState) {
    // Only show the last N lines that fit in the panel (area - 2 border rows).
    let visible = area.height.saturating_sub(2) as usize;
    let items: Vec<ListItem> = state
        .log
        .iter()
        .rev()
        .take(visible)
        .rev()
        .map(|line| ListItem::new(format!(" {line}")))
        .collect();

    let block = default_block("LOG");
    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn draw_topology(f: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .topology_edges
        .iter()
        .map(|(a, b, cost)| ListItem::new(format!(" {a} -- {b} (cost={cost})")))
        .collect();

    let block = default_block("TOPOLOGY (LSDB)");
    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn draw_input_bar(f: &mut Frame, area: Rect, state: &AppState) {
    let (title, content, border_color) = if state.input_mode {
        let cursor = if (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            / 500)
            % 2
            == 0
        {
            '█'
        } else {
            ' '
        };
        (
            "COMPOSE  [Enter] send  [Esc] cancel",
            format!(" > {}{}", state.input_buf, cursor),
            Color::Yellow,
        )
    } else {
        (
            "COMPOSE",
            " Press / to compose a message".to_string(),
            Color::DarkGray,
        )
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(Span::styled(title, Style::default().fg(Color::Cyan)));

    let style = if state.input_mode {
        Style::default().fg(Color::White)
    } else {
        Style::default().fg(Color::DarkGray)
    };

    let para = Paragraph::new(content).style(style).block(block);
    f.render_widget(para, area);
}

fn default_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(title, Style::default().fg(Color::Cyan)))
}
