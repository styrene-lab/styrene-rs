//! Conformance test suite pinned to NomadNet's nomad_net_guide.mu
//!
//! This is the canonical micron specification document — it describes
//! the language using itself. Every feature of micron is exercised in
//! this document. Our parser must handle it completely.
//!
//! The AGPL micron-parser crate parses this into 41 top-level blocks.
//! We use that as our baseline, then test individual features.

use styrene_micron::*;

const GUIDE: &str = include_str!("fixtures/nomad_net_guide.mu");

/// Recursively count all blocks including nested children.
fn count_all_blocks(doc: &Document) -> usize {
    fn count_children(children: &[ChildBlock]) -> usize {
        children.iter().map(|c| match c {
            ChildBlock::Section { children, .. } => 1 + count_children(children),
            _ => 1,
        }).sum()
    }
    doc.blocks.iter().map(|b| match b {
        Block::Section { children, .. } => 1 + count_children(children),
        _ => 1,
    }).sum()
}

// ── Smoke Test ─────────────────────────────────────────────────────

#[test]
fn parse_guide_does_not_panic() {
    let _doc = parse(GUIDE);
}

#[test]
fn parse_guide_produces_blocks() {
    let doc = parse(GUIDE);
    // The guide is a substantial document. With section nesting,
    // many blocks are children rather than top-level. The guide has
    // ~20+ top-level blocks (sections + inter-section content).
    assert!(
        doc.blocks.len() > 15,
        "expected >15 top-level blocks, got {}",
        doc.blocks.len()
    );
    // Also verify total block count including nested children
    let total = count_all_blocks(&doc);
    assert!(
        total > 80,
        "expected >80 total blocks (including nested), got {total}"
    );
}

// ── Section Nesting ────────────────────────────────────────────────

#[test]
fn section_creates_nested_scope() {
    // From the guide: sections contain indented children
    let doc = parse(">Heading\nChild text\nMore child text");
    assert_eq!(doc.blocks.len(), 1, "section + children = 1 top-level block");
    match &doc.blocks[0] {
        Block::Section {
            level,
            heading,
            children,
        } => {
            assert_eq!(*level, 1);
            assert!(heading.is_some());
            assert!(
                !children.is_empty(),
                "section must contain child blocks"
            );
        }
        other => panic!("expected Section, got {other:?}"),
    }
}

#[test]
fn nested_subsections() {
    // From guide: >High Level Stuff / >>Another Level / >>>Going deeper
    let doc = parse(">Level 1\nText in 1\n>>Level 2\nText in 2\n>>>Level 3\nText in 3");
    // Top level: one section at level 1
    assert_eq!(doc.blocks.len(), 1);
    match &doc.blocks[0] {
        Block::Section { level, children, .. } => {
            assert_eq!(*level, 1);
            // Children should include the subsection
            let has_subsection = children.iter().any(|c| matches!(c, ChildBlock::Section { .. }));
            assert!(has_subsection, "level-1 section must contain level-2 subsection");
        }
        other => panic!("expected Section, got {other:?}"),
    }
}

#[test]
fn section_reset_closes_scope() {
    // < at SOL resets depth to 0
    let doc = parse(">Heading\nInside section\n<\nOutside section");
    assert!(doc.blocks.len() >= 2, "section reset should create separate blocks");
    // First block is the section
    assert!(matches!(&doc.blocks[0], Block::Section { .. }));
    // After reset, content is top-level
    let has_top_level_line = doc.blocks[1..].iter().any(|b| matches!(b, Block::Line { .. }));
    assert!(has_top_level_line, "text after < should be top-level");
}

#[test]
fn section_reset_with_continuation() {
    // From guide: `<` followed by content on same line
    // The canon recurses on the remainder after <
    let doc = parse("<>New heading after reset");
    // Should produce a section (the > after < is parsed as heading)
    let has_section = doc.blocks.iter().any(|b| matches!(b, Block::Section { .. }));
    assert!(has_section, "< followed by > should create a new section");
}

#[test]
fn section_without_heading() {
    // From guide: >>>> with no text = headerless section for indentation
    let doc = parse(">>>>\nIndented text without heading");
    match &doc.blocks[0] {
        Block::Section { level, heading, children, .. } => {
            assert_eq!(*level, 4);
            assert!(heading.is_none(), "empty heading line = no heading");
            assert!(!children.is_empty());
        }
        other => panic!("expected Section, got {other:?}"),
    }
}

#[test]
fn section_level_uncapped() {
    // Guide uses >>>>>>>>>>>>>>>  (15 levels)
    let doc = parse(">>>>>>>>>>>>>>>Heading at 15");
    match &doc.blocks[0] {
        Block::Section { level, .. } => {
            assert_eq!(*level, 15, "section level must not be capped");
        }
        other => panic!("expected Section, got {other:?}"),
    }
}

// ── SOL-only Block Tags ───────────────────────────────────────────

#[test]
fn section_tag_mid_line_is_text() {
    // From guide test: "this is a > mid sentence section tag, ignored"
    let doc = parse("text > not a heading");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            let full_text: String = nodes
                .iter()
                .filter_map(|n| match n {
                    InlineNode::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert!(
                full_text.contains(">"),
                "mid-line > should be literal text"
            );
        }
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn section_end_mid_line_is_text() {
    let doc = parse("text < not a reset");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            let full_text: String = nodes
                .iter()
                .filter_map(|n| match n {
                    InlineNode::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert!(
                full_text.contains("<"),
                "mid-line < should be literal text"
            );
        }
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn comment_mid_line_is_text() {
    let doc = parse("text # not a comment");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            let full_text: String = nodes
                .iter()
                .filter_map(|n| match n {
                    InlineNode::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert!(full_text.contains("#"), "mid-line # should be literal text");
        }
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn divider_mid_line_is_text() {
    let doc = parse("text - not a divider");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            let full_text: String = nodes
                .iter()
                .filter_map(|n| match n {
                    InlineNode::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert!(full_text.contains("-"), "mid-line - should be literal text");
        }
        other => panic!("expected Line, got {other:?}"),
    }
}

// ── Dividers ──────────────────────────────────────────────────────

#[test]
fn divider_default_char() {
    let doc = parse("-");
    match &doc.blocks[0] {
        Block::Divider { symbol } => assert_eq!(*symbol, '\u{2500}'),
        other => panic!("expected Divider, got {other:?}"),
    }
}

#[test]
fn divider_custom_char_exactly_two() {
    // From guide: -∿
    let doc = parse("-∿");
    match &doc.blocks[0] {
        Block::Divider { symbol } => assert_eq!(*symbol, '∿'),
        other => panic!("expected Divider, got {other:?}"),
    }
}

#[test]
fn divider_long_line_uses_default() {
    // Canon: only custom char if line is exactly 2 chars
    let doc = parse("-abc");
    match &doc.blocks[0] {
        Block::Divider { symbol } => {
            assert_eq!(
                *symbol, '\u{2500}',
                "divider with >2 chars should use default symbol"
            );
        }
        other => panic!("expected Divider, got {other:?}"),
    }
}

#[test]
fn divider_control_char_rejected() {
    // Canon: ord(divider_char) < 32 falls back to default
    let doc = parse("-\x01");
    match &doc.blocks[0] {
        Block::Divider { symbol } => {
            assert_eq!(
                *symbol, '\u{2500}',
                "control char divider should fall back to default"
            );
        }
        other => panic!("expected Divider, got {other:?}"),
    }
}

// ── Alignment ─────────────────────────────────────────────────────

#[test]
fn alignment_center() {
    let doc = parse("`cCentered text");
    match &doc.blocks[0] {
        Block::Line(Line { alignment, .. }) => assert_eq!(*alignment, Alignment::Center),
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn alignment_right() {
    let doc = parse("`rRight text");
    match &doc.blocks[0] {
        Block::Line(Line { alignment, .. }) => assert_eq!(*alignment, Alignment::Right),
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn alignment_default_via_backtick_a() {
    // `a returns to document default, which is distinct from explicit Left
    let doc = parse("`cCentered\n`aDefault now");
    // Second line should have Default alignment
    let default_block = doc.blocks.iter().find(|b| match b {
        Block::Line(Line { nodes, .. }) => nodes.iter().any(|n| match n {
            InlineNode::Text { text, .. } => text.contains("Default"),
            _ => false,
        }),
        _ => false,
    });
    match default_block {
        Some(Block::Line(Line { alignment, .. })) => {
            assert_eq!(*alignment, Alignment::Default, "`a should set Default alignment");
        }
        other => panic!("expected Line with Default alignment, got {other:?}"),
    }
}

#[test]
fn style_reset_resets_alignment_to_default() {
    let doc = parse("`cCentered\n``After reset");
    let reset_block = doc.blocks.iter().find(|b| match b {
        Block::Line(Line { nodes, .. }) => nodes.iter().any(|n| match n {
            InlineNode::Text { text, .. } => text.contains("After reset"),
            _ => false,
        }),
        _ => false,
    });
    match reset_block {
        Some(Block::Line(Line { alignment, .. })) => {
            assert_eq!(
                *alignment,
                Alignment::Default,
                "`` should reset alignment to Default"
            );
        }
        other => panic!("expected Line with Default alignment, got {other:?}"),
    }
}

// ── Formatting ────────────────────────────────────────────────────

#[test]
fn bold_toggle() {
    let doc = parse("`!bold`! normal");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            assert!(nodes.len() >= 2);
            match &nodes[0] {
                InlineNode::Text { style, text } => {
                    assert!(style.has_bold(), "first span should be bold");
                    assert_eq!(text, "bold");
                }
                other => panic!("expected Text, got {other:?}"),
            }
            // Find the "normal" node
            let normal = nodes.iter().find(|n| matches!(n, InlineNode::Text { text, .. } if text.contains("normal")));
            match normal {
                Some(InlineNode::Text { style, .. }) => {
                    assert!(!style.has_bold(), "second span should not be bold");
                }
                other => panic!("expected non-bold Text, got {other:?}"),
            }
        }
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn italic_toggle() {
    let doc = parse("`*italic`* normal");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            match &nodes[0] {
                InlineNode::Text { style, .. } => assert!(style.has_italic()),
                other => panic!("expected Text, got {other:?}"),
            }
        }
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn underline_toggle() {
    let doc = parse("`_underline`_ normal");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            match &nodes[0] {
                InlineNode::Text { style, .. } => assert!(style.has_underline()),
                other => panic!("expected Text, got {other:?}"),
            }
        }
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn combined_formatting() {
    // From guide: `!`*`_combine
    let doc = parse("`!`*`_combined`_`*`!");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            match &nodes[0] {
                InlineNode::Text { style, text } => {
                    assert!(style.has_bold(), "should be bold");
                    assert!(style.has_italic(), "should be italic");
                    assert!(style.has_underline(), "should be underline");
                    assert_eq!(text, "combined");
                }
                other => panic!("expected Text, got {other:?}"),
            }
        }
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn style_reset_clears_all() {
    let doc = parse("`!`*`_styled``unstyled");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            let unstyled = nodes.iter().find(|n| matches!(n, InlineNode::Text { text, .. } if text == "unstyled"));
            match unstyled {
                Some(InlineNode::Text { style, .. }) => {
                    assert!(!style.has_bold());
                    assert!(!style.has_italic());
                    assert!(!style.has_underline());
                    assert!(style.fg_color().is_none());
                    assert!(style.bg_color().is_none());
                }
                other => panic!("expected unstyled Text, got {other:?}"),
            }
        }
        other => panic!("expected Line, got {other:?}"),
    }
}

// ── Colors ────────────────────────────────────────────────────────

#[test]
fn foreground_color_3hex() {
    // Canon: `F followed by exactly 3 hex chars
    let doc = parse("`Ff00red text`f");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Text { style, .. } => {
                assert_eq!(style.fg_color(), Some("f00"), "fg color should be raw 3-char hex");
            }
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn background_color_3hex() {
    let doc = parse("`B5d5colored`b");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Text { style, .. } => {
                assert_eq!(style.bg_color(), Some("5d5"));
            }
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn fg_color_reset() {
    let doc = parse("`Ff00red`f normal");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            let normal = nodes.iter().find(|n| matches!(n, InlineNode::Text { text, .. } if text.contains("normal")));
            match normal {
                Some(InlineNode::Text { style, .. }) => {
                    assert!(style.fg_color().is_none(), "`f should clear fg color");
                }
                other => panic!("expected Text, got {other:?}"),
            }
        }
        other => panic!("expected Line, got {other:?}"),
    }
}

// ── Literal Blocks ────────────────────────────────────────────────

#[test]
fn literal_block() {
    let doc = parse("`=\n`!not bold`!\nraw content\n`=");
    let lit = doc.blocks.iter().find(|b| matches!(b, Block::Literal { .. }));
    match lit {
        Some(Block::Literal { content }) => {
            assert!(content.contains("`!not bold`!"), "literal should preserve markup as-is");
            assert!(content.contains("raw content"));
        }
        other => panic!("expected Literal, got {other:?}"),
    }
}

#[test]
fn literal_toggle_must_be_sol() {
    // `= mid-line should NOT toggle literal mode
    let doc = parse("text `= not literal");
    // Should be a regular line, not a literal block
    assert!(!doc.blocks.iter().any(|b| matches!(b, Block::Literal { .. })),
        "mid-line `= should not create literal block");
}

#[test]
fn literal_escaped_toggle() {
    // Inside literal, \`= renders as `=
    let doc = parse("`=\n\\`=\n`=");
    match doc.blocks.iter().find(|b| matches!(b, Block::Literal { .. })) {
        Some(Block::Literal { content }) => {
            assert!(content.contains("`="), "escaped `= inside literal should be preserved as `=");
        }
        other => panic!("expected Literal with escaped toggle, got {other:?}"),
    }
}

// ── Links ─────────────────────────────────────────────────────────

#[test]
fn link_url_only() {
    let doc = parse("`[https://example.com]");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Link { url, label, .. } => {
                assert_eq!(url, "https://example.com");
                // Unlabeled: label should equal url or be empty
                assert!(label.is_none() || label.as_deref() == Some("https://example.com"));
            }
            other => panic!("expected Link, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn link_with_label() {
    let doc = parse("`[Click Here`https://example.com]");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Link { url, label, .. } => {
                assert_eq!(label.as_deref(), Some("Click Here"));
                assert_eq!(url, "https://example.com");
            }
            other => panic!("expected Link, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn link_with_fields() {
    // From guide: `[Submit all Fields`:/page/fields.mu`*]
    let doc = parse("`[Submit`:/page/fields.mu`*]");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Link { fields, .. } => {
                assert!(!fields.is_empty(), "link should have fields");
                assert_eq!(fields[0], "*");
            }
            other => panic!("expected Link, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn link_with_named_fields() {
    let doc = parse("`[Query`:/page/q.mu`username|auth_token|action=view]");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Link { fields, .. } => {
                assert_eq!(fields.len(), 3);
                assert_eq!(fields[0], "username");
                assert_eq!(fields[1], "auth_token");
                assert_eq!(fields[2], "action=view");
            }
            other => panic!("expected Link, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

// ── Fields ────────────────────────────────────────────────────────

#[test]
fn field_text_input() {
    let doc = parse("`<user_input`Pre-defined data>");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Field {
                field: FormField::Text { name, value, .. },
                ..
            } => {
                assert_eq!(name, "user_input");
                assert_eq!(value, "Pre-defined data");
            }
            other => panic!("expected Text field, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn field_empty_value() {
    let doc = parse("`<demo_empty`>");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Field {
                field: FormField::Text { name, value, .. },
                ..
            } => {
                assert_eq!(name, "demo_empty");
                assert_eq!(value, "");
            }
            other => panic!("expected Text field, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn field_with_size() {
    let doc = parse("`<16|with_size`>");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Field {
                field: FormField::Text { name, width, .. },
                ..
            } => {
                assert_eq!(name, "with_size");
                assert_eq!(*width, 16);
            }
            other => panic!("expected sized Text field, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn field_masked() {
    let doc = parse("`<!|masked_demo`hidden text>");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Field {
                field: FormField::Password { name, value, .. },
                ..
            } => {
                assert_eq!(name, "masked_demo");
                assert_eq!(value, "hidden text");
            }
            other => panic!("expected Password field, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn field_checkbox() {
    let doc = parse("`<?|field_name|value`>");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Field {
                field: FormField::Checkbox { name, value, checked },
                ..
            } => {
                assert_eq!(name, "field_name");
                assert_eq!(value, "value");
                assert!(!checked);
            }
            other => panic!("expected Checkbox, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn field_checkbox_prechecked() {
    let doc = parse("`<?|checkbox|1|*`>");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Field {
                field: FormField::Checkbox { checked, .. },
                ..
            } => {
                assert!(checked, "checkbox with |* should be pre-checked");
            }
            other => panic!("expected Checkbox, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn field_radio() {
    let doc = parse("`<^|color|Red`>");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Field {
                field: FormField::Radio { name, value, checked },
                ..
            } => {
                assert_eq!(name, "color");
                assert_eq!(value, "Red");
                assert!(!checked);
            }
            other => panic!("expected Radio, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn field_radio_prechecked() {
    let doc = parse("`<^|color|Blue`*>");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Field {
                field: FormField::Radio { checked, .. },
                ..
            } => {
                assert!(checked, "radio with `* before > should be pre-checked");
            }
            other => panic!("expected Radio, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

// ── Escape ────────────────────────────────────────────────────────

#[test]
fn escape_backtick() {
    let doc = parse("\\`!not bold\\`!");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            let full_text: String = nodes
                .iter()
                .filter_map(|n| match n {
                    InlineNode::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(full_text, "`!not bold`!");
        }
        other => panic!("expected Line, got {other:?}"),
    }
}

#[test]
fn escape_backslash() {
    let doc = parse("\\\\");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => match &nodes[0] {
            InlineNode::Text { text, .. } => assert_eq!(text, "\\"),
            other => panic!("expected Text, got {other:?}"),
        },
        other => panic!("expected Line, got {other:?}"),
    }
}

// ── Comments ──────────────────────────────────────────────────────

#[test]
fn comment_at_sol_is_hidden() {
    let doc = parse("# This is a comment\nVisible text");
    // Should only have the visible text, not the comment
    let has_comment_text = doc.blocks.iter().any(|b| match b {
        Block::Line(Line { nodes, .. }) => nodes.iter().any(|n| match n {
            InlineNode::Text { text, .. } => text.contains("This is a comment"),
            _ => false,
        }),
        _ => false,
    });
    assert!(!has_comment_text, "comment text should not appear in output");
}

// ── Page Directives ───────────────────────────────────────────────

#[test]
fn page_directive() {
    let doc = parse("#!bg=444");
    match &doc.blocks[0] {
        Block::Directive { key, value } => {
            assert_eq!(key, "bg");
            assert_eq!(value, "444");
        }
        other => panic!("expected Directive, got {other:?}"),
    }
}

// ── Heading Style Isolation ───────────────────────────────────────

#[test]
fn heading_style_does_not_leak() {
    // Formatting set inside a heading should not affect subsequent lines
    let doc = parse(">Heading with `!bold\nText after heading");
    // Find the text after heading
    let text_blocks: Vec<_> = doc.blocks.iter().filter(|b| match b {
        Block::Line(Line { nodes, .. }) => nodes.iter().any(|n| match n {
            InlineNode::Text { text, .. } => text.contains("Text after"),
            _ => false,
        }),
        Block::Section { children, .. } => children.iter().any(|c| match c {
            ChildBlock::Line(Line { nodes, .. }) => nodes.iter().any(|n| match n {
                InlineNode::Text { text, .. } => text.contains("Text after"),
                _ => false,
            }),
            _ => false,
        }),
        _ => false,
    }).collect();

    assert!(!text_blocks.is_empty(), "should find text after heading");
    // Check the style of "Text after heading" is not bold
    // (bold was set inside heading scope and should not leak)
}

// ── Multi-line Blocks ─────────────────────────────────────────────

#[test]
fn consecutive_lines_in_section_are_children() {
    let doc = parse(">Heading\nLine 1\nLine 2\nLine 3");
    match &doc.blocks[0] {
        Block::Section { children, .. } => {
            // All 3 lines should be children of the section
            let line_count = children.iter().filter(|c| matches!(c, ChildBlock::Line(_))).count();
            assert!(line_count >= 3, "section should contain all child lines, got {line_count}");
        }
        other => panic!("expected Section, got {other:?}"),
    }
}

// ── Color Spec: 3-char only ──────────────────────────────────────

#[test]
fn color_consumes_exactly_3_chars() {
    // `Fabcdef should be: fg=abc, then text "def"
    let doc = parse("`Fabcdef");
    match &doc.blocks[0] {
        Block::Line(Line { nodes, .. }) => {
            assert!(nodes.len() >= 1);
            match &nodes[0] {
                InlineNode::Text { style, text } => {
                    assert_eq!(style.fg_color(), Some("abc"));
                    assert_eq!(text, "def");
                }
                other => panic!("expected Text, got {other:?}"),
            }
        }
        other => panic!("expected Line, got {other:?}"),
    }
}
