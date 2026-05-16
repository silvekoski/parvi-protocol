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
    // Root layout: top bar (3) | body (rest)
    // -----------------------------------------------------------------------
    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(area);

    draw_top_bar(f, root[0], state);

    // Body: left 60% | right 40%
    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(root[1]);

    draw_left_column(f, body[0], state);
    draw_right_column(f, body[1], state);
}

// ---------------------------------------------------------------------------
// Top bar
// ---------------------------------------------------------------------------

fn draw_top_bar(f: &mut Frame, area: Rect, state: &AppState) {
    let olsr_str = match state.olsr_converged_in_ms {
        Some(ms) => format!("{ms}"),
        None => "---".to_string(),
    };
    let text = format!(
        " NODE: {}  EPOCH: {}  CH: {}  OLSR: {}ms",
        state.node_id, state.epoch, state.channel, olsr_str,
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
        // Split: routing 30% | neighbors 20% | targets 30% | counters 20%
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(30),
                Constraint::Percentage(20),
                Constraint::Percentage(30),
                Constraint::Percentage(20),
            ])
            .split(area);

        draw_routing_table(f, chunks[0], state);
        draw_neighbors(f, chunks[1], state);
        draw_targets(f, chunks[2], state);
        draw_attack_counters(f, chunks[3], state);
    } else {
        // Include topology panel below attack counters
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Percentage(15),
                Constraint::Percentage(25),
                Constraint::Percentage(15),
                Constraint::Percentage(20),
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
    // Queue depths 15% | image 50% | log 35%
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Percentage(50),
            Constraint::Percentage(35),
        ])
        .split(area);

    draw_queue_depths(f, chunks[0], state);
    draw_image(f, chunks[1], state);
    draw_log(f, chunks[2], state);
}

fn draw_queue_depths(f: &mut Frame, area: Rect, state: &AppState) {
    let q = &state.queues;
    let text = format!(
        " QUEUES: CRIT={} NORM={} BULK={}",
        q.critical, q.normal, q.bulk
    );
    let block = default_block("QUEUES");
    let para = Paragraph::new(text).block(block);
    f.render_widget(para, area);
}

fn draw_image(f: &mut Frame, area: Rect, state: &AppState) {
    let content = state
        .image
        .as_ref()
        .map(|i| i.ascii.as_str())
        .unwrap_or("(no image)");

    let block = default_block("IMAGE");
    let para = Paragraph::new(content).block(block);
    f.render_widget(para, area);
}

fn draw_log(f: &mut Frame, area: Rect, state: &AppState) {
    let items: Vec<ListItem> = state
        .log
        .iter()
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

fn default_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(Span::styled(title, Style::default().fg(Color::Cyan)))
}
