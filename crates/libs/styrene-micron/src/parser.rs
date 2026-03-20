//! Micron markup parser.
//!
//! Parses NomadNet's micron markup language into the structured
//! [`Document`] model defined in [`crate::model`].
//!
//! Pinned to the canonical NomadNet specification (nomad_net_guide.mu).
//! All block-level tags (`>`, `<`, `-`, `#`, `#!`, `` `= ``) are
//! SOL-only. Sections are nested scopes. Colors are 3-char hex.

use crate::model::*;

/// Parse micron markup source into a [`Document`].
pub fn parse(source: &str) -> Document {
    if source.is_empty() {
        return Document { blocks: vec![] };
    }

    let mut ctx = ParseContext::new();
    let lines: Vec<&str> = source.split('\n').collect();
    let mut i = 0;

    while i < lines.len() {
        i = ctx.process_line(lines[i], &lines, i);
    }

    // Close any open sections
    ctx.close_all_sections();

    Document {
        blocks: ctx.top_blocks,
    }
}

/// Tracks parsing state including section nesting.
struct ParseContext {
    /// Top-level blocks (outside any section).
    top_blocks: Vec<Block>,
    /// Stack of open sections. Each entry: (level, heading, children).
    section_stack: Vec<SectionFrame>,
    /// Current inline style state (persists across lines, except in headings).
    style: StyleSet,
    /// Current alignment.
    alignment: Alignment,
    /// Whether we're in literal mode.
    literal_mode: bool,
    /// Accumulated literal lines.
    literal_lines: Vec<String>,
}

struct SectionFrame {
    level: u8,
    heading: Option<Line>,
    children: Vec<ChildBlock>,
}

impl ParseContext {
    fn new() -> Self {
        Self {
            top_blocks: Vec::new(),
            section_stack: Vec::new(),
            style: StyleSet::default(),
            alignment: Alignment::Default,
            literal_mode: false,
            literal_lines: Vec::new(),
        }
    }

    /// Process a single line. Returns the index of the next line to process.
    fn process_line(&mut self, line: &str, _lines: &[&str], idx: usize) -> usize {
        // Literal mode toggle: `= must be the entire trimmed line and at SOL
        let trimmed = line.trim();
        if trimmed == "`=" {
            if self.literal_mode {
                let content = self.literal_lines.join("\n");
                self.literal_lines.clear();
                self.literal_mode = false;
                self.push_block_or_child(BlockOrChild::Literal { content });
            } else {
                self.literal_mode = true;
            }
            return idx + 1;
        }

        if self.literal_mode {
            // Inside literal: escaped `= becomes `=
            let lit_line = if line == "\\`=" { "`=" } else { line };
            self.literal_lines.push(lit_line.to_string());
            return idx + 1;
        }

        // Determine first character for SOL dispatch.
        // We use the raw line (not trimmed) because SOL means column 0.
        let first_char = line.chars().next();

        match first_char {
            // Page directives: #! at SOL
            Some('#') if line.starts_with("#!") => {
                let directive = &line[2..];
                if let Some((key, value)) = directive.split_once('=') {
                    self.push_block_or_child(BlockOrChild::Directive {
                        key: key.trim().to_string(),
                        value: value.trim().to_string(),
                    });
                }
            }
            // Comments: # at SOL (but not #!)
            Some('#') => {
                // Skip — comments are not rendered
            }
            // Section start: > at SOL
            Some('>') => {
                let level = line.bytes().take_while(|&b| b == b'>').count() as u8;
                let heading_text = &line[level as usize..];

                // Close sections at same or deeper level
                self.close_sections_to(level);

                // Parse heading with isolated style scope
                let heading = if heading_text.is_empty() {
                    None
                } else {
                    let mut heading_style = StyleSet::default();
                    let mut heading_align = Alignment::Default;
                    let nodes =
                        parse_inline(heading_text, &mut heading_style, &mut heading_align);
                    // Heading style does NOT leak to subsequent lines
                    if nodes.is_empty() {
                        None
                    } else {
                        Some(Line {
                            nodes,
                            alignment: heading_align,
                        })
                    }
                };

                // Push new section frame
                self.section_stack.push(SectionFrame {
                    level,
                    heading,
                    children: Vec::new(),
                });
            }
            // Section reset: < at SOL
            Some('<') => {
                self.close_all_sections();
                // Recurse on remainder after <
                let remainder = &line[1..];
                if !remainder.is_empty() {
                    return self.process_line(remainder, _lines, idx);
                }
            }
            // Divider: - at SOL
            Some('-') => {
                // `-∿` is 2 chars but multi-byte; use char count.
                let char_count = line.chars().count();
                let symbol = if char_count == 2 {
                    let ch = line.chars().nth(1).unwrap();
                    if (ch as u32) < 32 { '\u{2500}' } else { ch }
                } else {
                    '\u{2500}'
                };
                self.push_block_or_child(BlockOrChild::Divider { symbol });
            }
            // Empty line
            None => {
                self.push_block_or_child(BlockOrChild::EmptyLine);
            }
            // Regular text (including lines starting with whitespace)
            _ => {
                if trimmed.is_empty() {
                    self.push_block_or_child(BlockOrChild::EmptyLine);
                } else {
                    let nodes = parse_inline(line, &mut self.style, &mut self.alignment);
                    let line_block = BlockOrChild::Line(Line {
                        nodes,
                        alignment: self.alignment,
                    });
                    self.push_block_or_child(line_block);
                }
            }
        }

        idx + 1
    }

    /// Push a block either as a child of the current section or as top-level.
    fn push_block_or_child(&mut self, block: BlockOrChild) {
        if let Some(frame) = self.section_stack.last_mut() {
            frame.children.push(block.into_child());
        } else {
            self.top_blocks.push(block.into_top());
        }
    }

    /// Close sections down to (but not including) the given level.
    /// A new section at level N closes all open sections at level >= N.
    fn close_sections_to(&mut self, new_level: u8) {
        while let Some(frame) = self.section_stack.last() {
            if frame.level >= new_level {
                let frame = self.section_stack.pop().unwrap();
                let section = ChildBlock::Section {
                    level: frame.level,
                    heading: frame.heading,
                    children: frame.children,
                };
                // Push as child of parent section, or as top-level
                if let Some(parent) = self.section_stack.last_mut() {
                    parent.children.push(section);
                } else {
                    self.top_blocks.push(child_section_to_top(section));
                }
            } else {
                break;
            }
        }
    }

    /// Close all open sections.
    fn close_all_sections(&mut self) {
        while let Some(frame) = self.section_stack.pop() {
            let section = ChildBlock::Section {
                level: frame.level,
                heading: frame.heading,
                children: frame.children,
            };
            if let Some(parent) = self.section_stack.last_mut() {
                parent.children.push(section);
            } else {
                self.top_blocks.push(child_section_to_top(section));
            }
        }
    }
}

/// Convert a ChildBlock::Section to a top-level Block::Section.
fn child_section_to_top(child: ChildBlock) -> Block {
    match child {
        ChildBlock::Section {
            level,
            heading,
            children,
        } => Block::Section {
            level,
            heading,
            children,
        },
        _ => unreachable!(),
    }
}

/// Intermediate type for blocks before we know if they're top-level or children.
enum BlockOrChild {
    Line(Line),
    EmptyLine,
    Divider { symbol: char },
    Literal { content: String },
    Directive { key: String, value: String },
}

impl BlockOrChild {
    fn into_top(self) -> Block {
        match self {
            Self::Line(l) => Block::Line(l),
            Self::EmptyLine => Block::EmptyLine,
            Self::Divider { symbol } => Block::Divider { symbol },
            Self::Literal { content } => Block::Literal { content },
            Self::Directive { key, value } => Block::Directive { key, value },
        }
    }

    fn into_child(self) -> ChildBlock {
        match self {
            Self::Line(l) => ChildBlock::Line(l),
            Self::EmptyLine => ChildBlock::EmptyLine,
            Self::Divider { symbol } => ChildBlock::Divider { symbol },
            Self::Literal { content } => ChildBlock::Literal { content },
            // Directives inside sections: store as-is (not rendered)
            Self::Directive { .. } => ChildBlock::EmptyLine, // Directives don't nest meaningfully
        }
    }
}

// ── Inline Parser ──────────────────────────────────────────────────

/// Parse inline formatting, links, and fields within a single line.
///
/// Modifies `style` and `alignment` in place — formatting toggles persist
/// across lines, matching micron spec behavior.
pub(crate) fn parse_inline(
    text: &str,
    style: &mut StyleSet,
    alignment: &mut Alignment,
) -> Vec<InlineNode> {
    let mut nodes = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        let ch = chars[i];

        // Escape sequence
        if ch == '\\' && i + 1 < len {
            current.push(chars[i + 1]);
            i += 2;
            continue;
        }

        // Backtick formatting codes
        if ch == '`' && i + 1 < len {
            let next = chars[i + 1];

            match next {
                // Double backtick — reset all formatting AND alignment
                '`' => {
                    flush_text(&mut current, style, &mut nodes);
                    style.reset();
                    *alignment = Alignment::Default;
                    i += 2;
                }
                '!' => {
                    flush_text(&mut current, style, &mut nodes);
                    style.toggle(Style::Bold);
                    i += 2;
                }
                '*' => {
                    flush_text(&mut current, style, &mut nodes);
                    style.toggle(Style::Italic);
                    i += 2;
                }
                '_' => {
                    flush_text(&mut current, style, &mut nodes);
                    style.toggle(Style::Underline);
                    i += 2;
                }
                // Foreground color: exactly 3 hex chars
                'F' => {
                    flush_text(&mut current, style, &mut nodes);
                    if i + 4 < len {
                        let color: String = chars[i + 2..i + 5].iter().collect();
                        if color.chars().all(|c| c.is_ascii_hexdigit()) {
                            style.set(Style::FgColor(color));
                            i += 5;
                        } else {
                            i += 2; // invalid color, skip `F
                        }
                    } else {
                        i += 2;
                    }
                }
                'f' => {
                    flush_text(&mut current, style, &mut nodes);
                    style.unset(&Style::FgColor(String::new()));
                    i += 2;
                }
                'B' => {
                    flush_text(&mut current, style, &mut nodes);
                    if i + 4 < len {
                        let color: String = chars[i + 2..i + 5].iter().collect();
                        if color.chars().all(|c| c.is_ascii_hexdigit()) {
                            style.set(Style::BgColor(color));
                            i += 5;
                        } else {
                            i += 2;
                        }
                    } else {
                        i += 2;
                    }
                }
                'b' => {
                    flush_text(&mut current, style, &mut nodes);
                    style.unset(&Style::BgColor(String::new()));
                    i += 2;
                }
                'c' => {
                    flush_text(&mut current, style, &mut nodes);
                    *alignment = Alignment::Center;
                    i += 2;
                }
                'l' => {
                    flush_text(&mut current, style, &mut nodes);
                    *alignment = Alignment::Left;
                    i += 2;
                }
                'r' => {
                    flush_text(&mut current, style, &mut nodes);
                    *alignment = Alignment::Right;
                    i += 2;
                }
                'a' => {
                    flush_text(&mut current, style, &mut nodes);
                    *alignment = Alignment::Default;
                    i += 2;
                }
                // Link: `[label`url`fields]
                '[' => {
                    flush_text(&mut current, style, &mut nodes);
                    let (link, consumed) = parse_link(&chars, i + 2, style);
                    if let Some(node) = link {
                        nodes.push(node);
                    }
                    i += consumed;
                }
                // Form field: `<...>
                '<' => {
                    flush_text(&mut current, style, &mut nodes);
                    let (field, consumed) = parse_field(&chars, i + 2, style);
                    if let Some(node) = field {
                        nodes.push(node);
                    }
                    i += consumed;
                }
                // Literal toggle — handled at block level, skip
                '=' => {
                    i += 2;
                }
                // Unknown code — emit backtick literally
                _ => {
                    current.push(ch);
                    i += 1;
                }
            }
            continue;
        }

        current.push(ch);
        i += 1;
    }

    flush_text(&mut current, style, &mut nodes);
    nodes
}

/// Flush accumulated text into a Text node if non-empty.
fn flush_text(buf: &mut String, style: &StyleSet, nodes: &mut Vec<InlineNode>) {
    if !buf.is_empty() {
        nodes.push(InlineNode::Text {
            style: style.snapshot(),
            text: std::mem::take(buf),
        });
    }
}

/// Parse a link: `` `[label`url`fields] ``.
///
/// Canon: unlabeled = just url, labeled = label`url, with fields = label`url`field1|field2.
fn parse_link(
    chars: &[char],
    start: usize,
    style: &StyleSet,
) -> (Option<InlineNode>, usize) {
    let rest: String = chars[start..].iter().collect();
    let Some(end_offset) = rest.find(']') else {
        return (None, 2);
    };

    let inner = &rest[..end_offset];
    let parts: Vec<&str> = inner.split('`').collect();

    let (label, url, fields) = match parts.len() {
        // Just URL
        1 => (None, parts[0].to_string(), vec![]),
        // label`url
        2 => (Some(parts[0].to_string()), parts[1].to_string(), vec![]),
        // label`url`fields (fields are pipe-separated)
        _ => {
            let field_str = parts[2..].join("`");
            let fields: Vec<String> = field_str.split('|').map(|s| s.to_string()).collect();
            (Some(parts[0].to_string()), parts[1].to_string(), fields)
        }
    };

    let node = InlineNode::Link {
        style: style.snapshot(),
        label,
        url,
        fields,
    };

    // consumed: `[ (2) + inner + ] (1)
    (Some(node), 2 + end_offset + 1)
}

/// Parse a form field: `` `<descriptor`value> ``.
fn parse_field(
    chars: &[char],
    start: usize,
    style: &StyleSet,
) -> (Option<InlineNode>, usize) {
    let rest: String = chars[start..].iter().collect();
    let Some(end_offset) = rest.find('>') else {
        return (None, 2);
    };

    let inner = &rest[..end_offset];
    let backtick_pos = inner.find('`');

    let (descriptor, default_value) = match backtick_pos {
        Some(pos) => (&inner[..pos], inner[pos + 1..].to_string()),
        None => (inner, String::new()),
    };

    let field = if descriptor.contains('|') {
        let segments: Vec<&str> = descriptor.split('|').collect();
        let flags = segments[0];
        let name = segments.get(1).unwrap_or(&"").to_string();

        if flags.contains('?') {
            let value = segments.get(2).unwrap_or(&"").to_string();
            let checked = segments.get(3) == Some(&"*");
            FormField::Checkbox {
                name,
                value,
                checked,
            }
        } else if flags.contains('^') {
            let value = segments.get(2).unwrap_or(&"").to_string();
            let checked = default_value == "*"
                || segments.get(3) == Some(&"*");
            FormField::Radio {
                name,
                value,
                checked,
            }
        } else if flags.contains('!') {
            let width_str = flags.replace('!', "");
            let width = width_str.parse().unwrap_or(24);
            FormField::Password {
                name,
                value: default_value,
                width,
            }
        } else {
            let width = flags.parse().unwrap_or(24);
            FormField::Text {
                name,
                value: default_value,
                width,
            }
        }
    } else {
        FormField::Text {
            name: descriptor.to_string(),
            value: default_value,
            width: 24,
        }
    };

    let node = InlineNode::Field {
        style: style.snapshot(),
        field,
    };

    (Some(node), 2 + end_offset + 1)
}

// ── Unit Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty() {
        let doc = parse("");
        assert!(doc.blocks.is_empty());
    }

    #[test]
    fn style_persists_across_lines() {
        let doc = parse("`!bold starts\nstill bold`!");
        // Both lines should be inside no section, but bold should persist.
        // First line
        match &doc.blocks[0] {
            Block::Line(Line { nodes, .. }) => {
                assert!(
                    matches!(&nodes[0], InlineNode::Text { style, .. } if style.has_bold())
                );
            }
            _ => panic!("expected line"),
        }
        // Second line
        match &doc.blocks[1] {
            Block::Line(Line { nodes, .. }) => {
                assert!(
                    matches!(&nodes[0], InlineNode::Text { style, .. } if style.has_bold())
                );
            }
            _ => panic!("expected line"),
        }
    }

    #[test]
    fn parse_color_3char() {
        let doc = parse("`Fabctext`f");
        match &doc.blocks[0] {
            Block::Line(Line { nodes, .. }) => match &nodes[0] {
                InlineNode::Text { style, text } => {
                    assert_eq!(style.fg_color(), Some("abc"));
                    assert_eq!(text, "text");
                }
                _ => panic!("expected text"),
            },
            _ => panic!("expected line"),
        }
    }
}
