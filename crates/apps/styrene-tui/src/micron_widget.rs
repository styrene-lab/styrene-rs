//! Micron → Ratatui renderer.
//!
//! Converts a parsed `styrene_micron::Document` into ratatui `Text`
//! with proper styling, section indentation, and alignment.

use ratatui::style::{Color, Modifier, Style as RStyle};
use ratatui::text::{Line, Span, Text};

use styrene_micron::{
    Alignment, Block, ChildBlock, Document, FormField, InlineNode, StyleSet,
    Line as MicronLine,
};

/// Section indentation in spaces per level (matches NomadNet SECTION_INDENT=2).
const SECTION_INDENT: usize = 2;

/// Heading background colors per level (dark theme, matching NomadNet STYLES_DARK).
const HEADING_STYLES: [(Color, Color); 3] = [
    (Color::Rgb(0x22, 0x22, 0x22), Color::Rgb(0xbb, 0xbb, 0xbb)), // h1: fg=222, bg=bbb
    (Color::Rgb(0x11, 0x11, 0x11), Color::Rgb(0x99, 0x99, 0x99)), // h2: fg=111, bg=999
    (Color::Rgb(0x00, 0x00, 0x00), Color::Rgb(0x77, 0x77, 0x77)), // h3: fg=000, bg=777
];

/// Render a micron Document to ratatui Text.
pub fn render_document(doc: &Document, width: u16) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();

    for block in &doc.blocks {
        render_block(block, 0, width, &mut lines);
    }

    Text::from(lines)
}

fn render_block(block: &Block, depth: usize, width: u16, lines: &mut Vec<Line<'static>>) {
    match block {
        Block::Section {
            level,
            heading,
            children,
        } => {
            let section_depth = depth + 1;
            // Render heading
            if let Some(heading_line) = heading {
                let heading_idx = (*level as usize).saturating_sub(1).min(2);
                let (fg, bg) = HEADING_STYLES[heading_idx];
                let indent = indent_str(depth);
                let mut spans = vec![Span::raw(indent)];
                // Add a leading space for heading text
                spans.push(Span::styled(
                    " ",
                    RStyle::default().fg(fg).bg(bg),
                ));
                for node in &heading_line.nodes {
                    let base = RStyle::default().fg(fg).bg(bg);
                    render_inline_node(node, base, &mut spans);
                }
                // Pad heading to width
                spans.push(Span::styled(" ", RStyle::default().fg(fg).bg(bg)));
                lines.push(Line::from(spans));
            }
            // Render children
            for child in children {
                render_child_block(child, section_depth, width, lines);
            }
        }
        Block::Line(line) => {
            render_line(line, 0, lines);
        }
        Block::EmptyLine => {
            lines.push(Line::from(""));
        }
        Block::Divider { symbol } => {
            let fill: String = std::iter::repeat(*symbol)
                .take(width as usize)
                .collect();
            lines.push(Line::from(Span::styled(
                fill,
                RStyle::default().fg(Color::DarkGray),
            )));
        }
        Block::Literal { content } => {
            for lit_line in content.lines() {
                lines.push(Line::from(Span::styled(
                    lit_line.to_string(),
                    RStyle::default()
                        .fg(Color::DarkGray)
                        .add_modifier(Modifier::ITALIC),
                )));
            }
        }
        Block::Directive { .. } => {
            // Directives are metadata, not rendered
        }
    }
}

fn render_child_block(
    child: &ChildBlock,
    depth: usize,
    width: u16,
    lines: &mut Vec<Line<'static>>,
) {
    match child {
        ChildBlock::Section {
            level,
            heading,
            children,
        } => {
            // Re-wrap as Block::Section for rendering
            render_block(
                &Block::Section {
                    level: *level,
                    heading: heading.clone(),
                    children: children.clone(),
                },
                depth,
                width,
                lines,
            );
        }
        ChildBlock::Line(line) => {
            render_line(line, depth, lines);
        }
        ChildBlock::EmptyLine => {
            lines.push(Line::from(""));
        }
        ChildBlock::Divider { symbol } => {
            let indent = indent_str(depth);
            let fill_width = (width as usize).saturating_sub(depth * SECTION_INDENT * 2);
            let fill: String = std::iter::repeat(*symbol).take(fill_width).collect();
            lines.push(Line::from(vec![
                Span::raw(indent),
                Span::styled(fill, RStyle::default().fg(Color::DarkGray)),
            ]));
        }
        ChildBlock::Literal { content } => {
            let indent = indent_str(depth);
            for lit_line in content.lines() {
                lines.push(Line::from(vec![
                    Span::raw(indent.clone()),
                    Span::styled(
                        lit_line.to_string(),
                        RStyle::default()
                            .fg(Color::DarkGray)
                            .add_modifier(Modifier::ITALIC),
                    ),
                ]));
            }
        }
    }
}

fn render_line(line: &MicronLine, depth: usize, lines: &mut Vec<Line<'static>>) {
    let mut spans: Vec<Span<'static>> = Vec::new();

    if depth > 0 {
        spans.push(Span::raw(indent_str(depth)));
    }

    for node in &line.nodes {
        render_inline_node(node, RStyle::default(), &mut spans);
    }

    let rline = Line::from(spans);
    let rline = match line.alignment {
        Alignment::Center => rline.centered(),
        Alignment::Right => rline.right_aligned(),
        _ => rline,
    };

    lines.push(rline);
}

fn render_inline_node(
    node: &InlineNode,
    base_style: RStyle,
    spans: &mut Vec<Span<'static>>,
) {
    match node {
        InlineNode::Text { style, text } => {
            let rstyle = micron_style_to_ratatui(style, base_style);
            spans.push(Span::styled(text.clone(), rstyle));
        }
        InlineNode::Newline => {
            // Newlines within a block — usually handled at line level
        }
        InlineNode::Link {
            style,
            label,
            url,
            ..
        } => {
            let mut rstyle = micron_style_to_ratatui(style, base_style);
            rstyle = rstyle.add_modifier(Modifier::UNDERLINED);
            let display = label.as_deref().unwrap_or(url.as_str());
            spans.push(Span::styled(display.to_string(), rstyle));
        }
        InlineNode::Field { field, .. } => {
            match field {
                FormField::Text { name, value, width } => {
                    let display = if value.is_empty() {
                        format!("[{name}: …]")
                    } else {
                        let truncated: String = value.chars().take(*width as usize).collect();
                        format!("[{name}: {truncated}]")
                    };
                    spans.push(Span::styled(
                        display,
                        base_style.add_modifier(Modifier::UNDERLINED),
                    ));
                }
                FormField::Password { name, value, .. } => {
                    let masked: String = "•".repeat(value.len().max(3));
                    spans.push(Span::styled(
                        format!("[{name}: {masked}]"),
                        base_style.add_modifier(Modifier::UNDERLINED),
                    ));
                }
                FormField::Checkbox { checked, .. } => {
                    let mark = if *checked { "x" } else { " " };
                    spans.push(Span::styled(
                        format!("[{mark}]"),
                        base_style,
                    ));
                }
                FormField::Radio { checked, .. } => {
                    let mark = if *checked { "•" } else { " " };
                    spans.push(Span::styled(
                        format!("({mark})"),
                        base_style,
                    ));
                }
            }
        }
    }
}

/// Convert a micron StyleSet to a ratatui Style, layered on a base style.
fn micron_style_to_ratatui(style: &StyleSet, base: RStyle) -> RStyle {
    let mut rs = base;

    if style.has_bold() {
        rs = rs.add_modifier(Modifier::BOLD);
    }
    if style.has_italic() {
        rs = rs.add_modifier(Modifier::ITALIC);
    }
    if style.has_underline() {
        rs = rs.add_modifier(Modifier::UNDERLINED);
    }
    if let Some(fg) = style.fg_color() {
        if let Some(color) = parse_3hex(fg) {
            rs = rs.fg(color);
        }
    }
    if let Some(bg) = style.bg_color() {
        if let Some(color) = parse_3hex(bg) {
            rs = rs.bg(color);
        }
    }

    rs
}

/// Parse a 3-char hex color (NomadNet format) to ratatui Color.
/// "f00" → Color::Rgb(0xff, 0x00, 0x00)
fn parse_3hex(s: &str) -> Option<Color> {
    if s.len() != 3 {
        return None;
    }
    let chars: Vec<char> = s.chars().collect();
    let r = u8::from_str_radix(&format!("{}{}", chars[0], chars[0]), 16).ok()?;
    let g = u8::from_str_radix(&format!("{}{}", chars[1], chars[1]), 16).ok()?;
    let b = u8::from_str_radix(&format!("{}{}", chars[2], chars[2]), 16).ok()?;
    Some(Color::Rgb(r, g, b))
}

/// Generate an indentation string for a given depth.
fn indent_str(depth: usize) -> String {
    " ".repeat(depth * SECTION_INDENT)
}
