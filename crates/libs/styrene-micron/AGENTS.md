# styrene-micron

Micron markup parser for NomadNet/Reticulum page rendering. Parses NomadNet's micron markup language into a structured document model. Zero dependencies (no external crates).

Pinned to the canonical NomadNet specification (`nomad_net_guide.mu`).

## Module Map

| File | Purpose |
|------|---------|
| `src/lib.rs` | Crate root, re-exports `model::*` and `parser::parse` |
| `src/model.rs` | Document model: Document, Block, ChildBlock, Line, InlineNode, StyleSet, FormField, Alignment |
| `src/parser.rs` | `parse()` function, block-level dispatch, inline formatting parser, link/field parsers |

## Key Types

### Document Model

- **`Document`** -- `{ blocks: Vec<Block> }`
- **`Block`** -- top-level: Section, Line, EmptyLine, Divider, Literal, Directive
- **`ChildBlock`** -- inside sections: Section (nested), Line, EmptyLine, Divider, Literal
- **`Line`** -- `{ nodes: Vec<InlineNode>, alignment: Alignment }`
- **`InlineNode`** -- Text (with StyleSet), Newline, Link (label/url/fields), Field (form input)

### Styling

- **`StyleSet`** -- accumulated styles: `has_bold()`, `has_italic()`, `has_underline()`, `fg_color()`, `bg_color()`. Styles toggle and persist across lines (spec behavior).
- **`Style`** -- Bold, Italic, Underline, FgColor(String), BgColor(String). Colors are 3-char hex.
- **`Alignment`** -- Default, Left, Center, Right.

### Form Fields

- **`FormField`** -- Text, Password, Checkbox, Radio. Each has name, value, and type-specific fields (width, checked).

## Micron Syntax Quick Reference

| Syntax | Meaning |
|--------|---------|
| `>` / `>>` | Section (nesting by count) |
| `<` | Close all sections |
| `-` | Horizontal divider (2-char line = custom symbol) |
| `#` | Comment (not rendered) |
| `#!key=value` | Page directive |
| `` `= `` | Toggle literal/preformatted block |
| `` `! `` | Toggle bold |
| `` `* `` | Toggle italic |
| `` `_ `` | Toggle underline |
| `` `Fabc `` | Set foreground color (3-char hex) |
| `` `f `` | Clear foreground color |
| `` `Babc `` | Set background color (3-char hex) |
| `` `b `` | Clear background color |
| `` `c `` / `` `l `` / `` `r `` / `` `a `` | Center / Left / Right / Default alignment |
| ` `` ` `` ` | Reset all formatting and alignment |
| `` `[label`url`fields] `` | Link |
| `` `<descriptor`value> `` | Form field |
| `\` | Escape next character |

## Public API

```rust
use styrene_micron::{parse, Block, InlineNode, Line, StyleSet};

let doc = parse(">Heading\nBody text");
```

Single entry point: `parse(source: &str) -> Document`.

## Test Commands

```bash
cargo test -p styrene-micron
```

## Gotchas

- Inline styles persist across lines within the same parse context. Only heading styles are isolated (don't leak to subsequent lines).
- Section `>` at column 0 only -- indented `>` is regular text.
- Directives inside sections are silently dropped (converted to EmptyLine).
- Literal blocks: `\`=` inside a literal block becomes a literal `` `= `` (escape mechanism).
- Colors are always 3-char hex, not 6-char. Invalid hex after `\`F` skips the code silently.
- `StyleSet` methods `set()`, `unset()`, `toggle()`, `reset()`, `snapshot()` are `pub(crate)` -- external code reads styles but cannot mutate them.
- No external dependencies at all. Pure Rust, no allocator requirements beyond std.

## Status

Functional. Covers the full NomadNet micron spec: sections, inline formatting, colors, alignment, links, form fields, literal blocks, directives, dividers. Ready for use by any rendering backend (ratatui, Dioxus, HTML).
