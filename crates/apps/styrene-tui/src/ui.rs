//! UI rendering — all Ratatui widget construction lives here.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Padding, Paragraph, Row, Table, Tabs};
use ratatui::Frame;

use crate::micron_widget;
use crate::state::{AppState, Tab};

const ACCENT: Color = Color::Rgb(88, 166, 255); // #58a6ff
const GREEN: Color = Color::Rgb(63, 185, 80); // #3fb950
const MUTED: Color = Color::Rgb(139, 148, 158); // #8b949e
const SURFACE: Color = Color::Rgb(22, 27, 34); // #161b22

pub fn render(frame: &mut Frame, state: &AppState) {
    let [header_area, tabs_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    render_header(frame, header_area);
    render_tabs(frame, tabs_area, state.active_tab);

    match state.active_tab {
        Tab::Identity => render_identity(frame, body_area, state),
        Tab::Mesh => render_mesh(frame, body_area, state),
        Tab::Micron => render_micron(frame, body_area),
    }

    render_footer(frame, footer_area);
}

fn render_header(frame: &mut Frame, area: Rect) {
    let header = Paragraph::new(Line::from(vec![
        Span::styled("⬡ ", Style::default().fg(ACCENT)),
        Span::styled(
            "Styrene Mesh",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  Ratatui TUI spike", Style::default().fg(MUTED)),
    ]))
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .border_style(Style::default().fg(MUTED))
            .padding(Padding::new(1, 0, 0, 0)),
    );

    frame.render_widget(header, area);
}

fn render_tabs(frame: &mut Frame, area: Rect, active: Tab) {
    let titles: Vec<&str> = Tab::ALL.iter().map(|t| t.title()).collect();
    let selected = Tab::ALL.iter().position(|t| *t == active).unwrap_or(0);

    let tabs = Tabs::new(titles)
        .select(selected)
        .highlight_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        .style(Style::default().fg(MUTED))
        .divider("│");

    frame.render_widget(tabs, area);
}

fn render_identity(frame: &mut Frame, area: Rect, state: &AppState) {
    let id = &state.identity;

    let rows = vec![
        Row::new(vec![
            Span::styled("Hash", Style::default().fg(MUTED)),
            Span::styled(&id.hash_hex, Style::default().fg(GREEN)),
        ]),
        Row::new(vec![
            Span::styled("Public Key", Style::default().fg(MUTED)),
            Span::styled(&id.public_key_hex, Style::default().fg(GREEN)),
        ]),
        Row::new(vec![
            Span::styled("Signing Key", Style::default().fg(MUTED)),
            Span::styled(&id.signing_key_hex, Style::default().fg(GREEN)),
        ]),
    ];

    let table = Table::new(rows, [Constraint::Length(14), Constraint::Min(0)])
        .block(
            Block::default()
                .title(" Local Identity ")
                .title_style(Style::default().fg(ACCENT))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(MUTED))
                .style(Style::default().bg(SURFACE))
                .padding(Padding::new(1, 1, 1, 0)),
        )
        .row_highlight_style(Style::default());

    frame.render_widget(table, area);
}

fn render_mesh(frame: &mut Frame, area: Rect, state: &AppState) {
    let mesh = &state.mesh;
    let transport_str = if mesh.transport_active { "Active" } else { "Inactive" };
    let transport_color = if mesh.transport_active { GREEN } else { MUTED };

    let rows = vec![
        Row::new(vec![
            Span::styled("Interfaces", Style::default().fg(MUTED)),
            Span::styled(mesh.interfaces.to_string(), Style::default().fg(ACCENT)),
        ]),
        Row::new(vec![
            Span::styled("Known Paths", Style::default().fg(MUTED)),
            Span::styled(mesh.known_paths.to_string(), Style::default().fg(ACCENT)),
        ]),
        Row::new(vec![
            Span::styled("Announces Seen", Style::default().fg(MUTED)),
            Span::styled(
                mesh.announces_seen.to_string(),
                Style::default().fg(ACCENT),
            ),
        ]),
        Row::new(vec![
            Span::styled("Transport", Style::default().fg(MUTED)),
            Span::styled(transport_str, Style::default().fg(transport_color)),
        ]),
    ];

    let table = Table::new(rows, [Constraint::Length(16), Constraint::Min(0)])
        .block(
            Block::default()
                .title(" Mesh Status ")
                .title_style(Style::default().fg(ACCENT))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(MUTED))
                .style(Style::default().bg(SURFACE))
                .padding(Padding::new(1, 1, 1, 0)),
        );

    frame.render_widget(table, area);
}

fn render_micron(frame: &mut Frame, area: Rect) {
    // Demo: render the micron guide excerpt
    let sample = concat!(
        ">Micron Renderer Demo\n",
        "\n",
        "This tab demonstrates `!styrene-micron`! rendering in `*ratatui`*.\n",
        "\n",
        ">>Formatting\n",
        "Text can be `!bold`!, `*italic`*, or `_underlined`_.\n",
        "You can also `!`*`_combine`_`*`! them.\n",
        "\n",
        ">>Colors\n",
        "Use `Ff00red`f, `F0f0green`f, and `F00fblue`f text.\n",
        "Or `B5d5 highlighted backgrounds `b.\n",
        "\n",
        ">>Sections\n",
        ">>>Nested Section\n",
        "Content is automatically indented by section depth.\n",
        ">>>>Deeper Still\n",
        "Each level adds indentation.\n",
        "\n",
        "-\n",
        "\n",
        ">>Links & Fields\n",
        "`[Example Link`https://example.com]\n",
        "Input: `<username`default_value>\n",
        "`<?|agree|yes`> I agree to the terms\n",
        "\n",
        "`=\n",
        "Literal block: `!not bold`! — markup is preserved as-is\n",
        "`=\n",
        "\n",
        "`cCentered text\n",
        "`rRight-aligned text\n",
        "`aBack to default\n",
    );

    let doc = styrene_micron::parse(sample);
    let content_width = area.width.saturating_sub(2); // account for border
    let text = micron_widget::render_document(&doc, content_width);

    let paragraph = Paragraph::new(text).block(
        Block::default()
            .title(" Micron Renderer ")
            .title_style(Style::default().fg(ACCENT))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(MUTED))
            .style(Style::default().bg(SURFACE))
            .padding(Padding::new(1, 1, 1, 0)),
    );

    frame.render_widget(paragraph, area);
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(" q", Style::default().fg(ACCENT)),
        Span::styled(" quit  ", Style::default().fg(MUTED)),
        Span::styled("Tab", Style::default().fg(ACCENT)),
        Span::styled(" switch  ", Style::default().fg(MUTED)),
        Span::styled("r", Style::default().fg(ACCENT)),
        Span::styled(" regenerate identity", Style::default().fg(MUTED)),
    ]));

    frame.render_widget(footer, area);
}
