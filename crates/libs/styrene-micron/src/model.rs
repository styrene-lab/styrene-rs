//! Micron document model.
//!
//! Structured representation of parsed micron markup. Derived from
//! the canonical NomadNet specification (nomad_net_guide.mu).

use std::mem::discriminant;

/// A parsed micron document.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub blocks: Vec<Block>,
}

/// Top-level block elements.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// Section scope with heading, depth, and child blocks.
    /// Created by `>` at SOL. Children are indented by depth × SECTION_INDENT.
    /// Sections nest: a `>>` inside a `>` becomes a child Section.
    Section {
        level: u8,
        heading: Option<Line>,
        children: Vec<ChildBlock>,
    },
    /// A line of inline-formatted text (outside any section).
    Line(Line),
    /// Empty line (paragraph break).
    EmptyLine,
    /// Horizontal divider. Custom char only when source line is exactly 2 chars.
    Divider { symbol: char },
    /// Literal (preformatted) block — content is not parsed for markup.
    Literal { content: String },
    /// Page directive (`#!key=value`).
    Directive { key: String, value: String },
}

/// Blocks that can appear inside a Section.
#[derive(Debug, Clone, PartialEq)]
pub enum ChildBlock {
    /// Nested subsection.
    Section {
        level: u8,
        heading: Option<Line>,
        children: Vec<ChildBlock>,
    },
    /// A line of text within a section.
    Line(Line),
    /// Empty line within a section.
    EmptyLine,
    /// Divider within a section.
    Divider { symbol: char },
    /// Literal block within a section.
    Literal { content: String },
}

/// A line of inline-formatted nodes with alignment.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub nodes: Vec<InlineNode>,
    pub alignment: Alignment,
}

/// Inline content nodes within a line or heading.
#[derive(Debug, Clone, PartialEq)]
pub enum InlineNode {
    /// Styled text span.
    Text { style: StyleSet, text: String },
    /// Line break within a block.
    Newline,
    /// Hyperlink.
    Link {
        style: StyleSet,
        label: Option<String>,
        url: String,
        fields: Vec<String>,
    },
    /// Form field (checkbox, radio, text input).
    Field { style: StyleSet, field: FormField },
}

/// Text alignment. `Default` means "document default" (reset by `` `a ``).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    /// Document default alignment (what `` `a `` and `` `` `` reset to).
    Default,
    Left,
    Center,
    Right,
}

impl Default for Alignment {
    fn default() -> Self {
        Alignment::Default
    }
}

/// Accumulated inline styles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StyleSet(pub Vec<Style>);

/// Individual style modifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Style {
    Bold,
    Italic,
    Underline,
    /// Foreground color as raw 3-char hex (spec level).
    FgColor(String),
    /// Background color as raw 3-char hex (spec level).
    BgColor(String),
}

impl StyleSet {
    /// Check whether the set contains a style of the same variant.
    pub fn has(&self, s: &Style) -> bool {
        self.0.iter().any(|m| discriminant(m) == discriminant(s))
    }

    pub fn has_bold(&self) -> bool {
        self.has(&Style::Bold)
    }
    pub fn has_italic(&self) -> bool {
        self.has(&Style::Italic)
    }
    pub fn has_underline(&self) -> bool {
        self.has(&Style::Underline)
    }

    pub fn fg_color(&self) -> Option<&str> {
        self.0.iter().find_map(|s| match s {
            Style::FgColor(c) => Some(c.as_str()),
            _ => None,
        })
    }

    pub fn bg_color(&self) -> Option<&str> {
        self.0.iter().find_map(|s| match s {
            Style::BgColor(c) => Some(c.as_str()),
            _ => None,
        })
    }

    pub(crate) fn set(&mut self, style: Style) {
        self.unset(&style);
        self.0.push(style);
    }

    pub(crate) fn unset(&mut self, style: &Style) {
        self.0.retain(|s| discriminant(s) != discriminant(style));
    }

    pub(crate) fn toggle(&mut self, style: Style) {
        if self.has(&style) {
            self.unset(&style);
        } else {
            self.0.push(style);
        }
    }

    pub(crate) fn snapshot(&self) -> Self {
        Self(self.0.clone())
    }

    pub(crate) fn reset(&mut self) {
        self.0.clear();
    }
}

/// Form field types.
#[derive(Debug, Clone, PartialEq)]
pub enum FormField {
    Text {
        name: String,
        value: String,
        width: u8,
    },
    Password {
        name: String,
        value: String,
        width: u8,
    },
    Checkbox {
        name: String,
        value: String,
        checked: bool,
    },
    Radio {
        name: String,
        value: String,
        checked: bool,
    },
}
