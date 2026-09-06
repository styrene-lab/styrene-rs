//! NomadNet content semantics, independent of daemon and IPC runtime.

use std::{collections::BTreeMap, fmt};
use styrene_micron::{Block, ChildBlock, Document, FormField, InlineNode, Line};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PageFormFieldKind {
    #[default]
    Text,
    Password,
    Checkbox,
    Radio,
    Unknown,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageFormField {
    pub name: String,
    pub kind: PageFormFieldKind,
    pub value: Option<String>,
    pub width: Option<u8>,
    pub checked: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageLinkTarget {
    pub label: Option<String>,
    pub target: String,
    pub submitted_fields: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PageParserWarning {
    pub code: String,
    pub message: String,
}
#[derive(Clone, Default, PartialEq, Eq)]
pub struct PageFormSubmission {
    pub values: BTreeMap<String, Vec<String>>,
}
impl fmt::Debug for PageFormSubmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PageFormSubmission")
            .field("field_names", &self.values.keys().collect::<Vec<_>>())
            .field("values", &"[REDACTED]")
            .finish()
    }
}

pub struct Projection {
    pub text: String,
    pub title: Option<String>,
    pub links: Vec<String>,
    pub fields: Vec<PageFormField>,
    pub link_targets: Vec<PageLinkTarget>,
}

pub fn render_projection(document: &Document, warnings: &mut Vec<PageParserWarning>) -> Projection {
    let mut projection = Projection {
        text: String::new(),
        title: None,
        links: Vec::new(),
        fields: Vec::new(),
        link_targets: Vec::new(),
    };
    for block in &document.blocks {
        render_block(block, &mut projection, warnings);
    }
    projection.text = projection.text.trim_end_matches('\n').to_string();
    projection
}

fn render_block(block: &Block, projection: &mut Projection, warnings: &mut Vec<PageParserWarning>) {
    match block {
        Block::Section { heading, children, .. } => {
            if let Some(heading) = heading {
                let text = render_line(heading, projection, warnings);
                if projection.title.is_none() && !text.is_empty() {
                    projection.title = Some(text.clone());
                }
                projection.text.push_str(&text);
                projection.text.push('\n');
            }
            for child in children {
                render_child(child, projection, warnings);
            }
        }
        Block::Line(line) => {
            let rendered = render_line(line, projection, warnings);
            projection.text.push_str(&rendered);
            projection.text.push('\n');
        }
        Block::EmptyLine => projection.text.push('\n'),
        Block::Divider { symbol } => {
            projection.text.extend(std::iter::repeat_n(*symbol, 24));
            projection.text.push('\n');
        }
        Block::Literal { content } => {
            projection.text.push_str(content);
            projection.text.push('\n');
        }
        Block::Directive { key, .. } => warnings.push(warning(
            "directive_not_rendered",
            format!("Micron directive {key:?} is retained in the parse but not rendered"),
        )),
    }
}

fn render_child(
    child: &ChildBlock,
    projection: &mut Projection,
    warnings: &mut Vec<PageParserWarning>,
) {
    match child {
        ChildBlock::Section { heading, children, .. } => {
            if let Some(heading) = heading {
                let rendered = render_line(heading, projection, warnings);
                projection.text.push_str(&rendered);
                projection.text.push('\n');
            }
            for child in children {
                render_child(child, projection, warnings);
            }
        }
        ChildBlock::Line(line) => {
            let rendered = render_line(line, projection, warnings);
            projection.text.push_str(&rendered);
            projection.text.push('\n');
        }
        ChildBlock::EmptyLine => projection.text.push('\n'),
        ChildBlock::Divider { symbol } => {
            projection.text.extend(std::iter::repeat_n(*symbol, 24));
            projection.text.push('\n');
        }
        ChildBlock::Literal { content } => {
            projection.text.push_str(content);
            projection.text.push('\n');
        }
    }
}

fn render_line(
    line: &Line,
    projection: &mut Projection,
    _warnings: &mut Vec<PageParserWarning>,
) -> String {
    let mut rendered = String::new();
    for node in &line.nodes {
        match node {
            InlineNode::Text { text, .. } => rendered.push_str(text),
            InlineNode::Newline => rendered.push('\n'),
            InlineNode::Link { label, url, fields, .. } => {
                projection.links.push(url.clone());
                let target = PageLinkTarget {
                    label: label.clone(),
                    target: url.clone(),
                    submitted_fields: fields.clone(),
                };
                projection.link_targets.push(target);
                rendered.push_str(label.as_deref().unwrap_or(url));
            }
            InlineNode::Field { field, .. } => {
                projection.fields.push(project_field(field));
            }
        }
    }
    rendered
}

fn project_field(field: &FormField) -> PageFormField {
    let mut projected = PageFormField::default();
    match field {
        FormField::Text { name, value, width } => {
            projected.name = name.clone();
            projected.kind = PageFormFieldKind::Text;
            projected.value = Some(value.clone());
            projected.width = Some(*width);
        }
        FormField::Password { name, width, .. } => {
            projected.name = name.clone();
            projected.kind = PageFormFieldKind::Password;
            projected.width = Some(*width);
        }
        FormField::Checkbox { name, value, checked } => {
            projected.name = name.clone();
            projected.kind = PageFormFieldKind::Checkbox;
            projected.value = Some(value.clone());
            projected.checked = *checked;
        }
        FormField::Radio { name, value, checked } => {
            projected.name = name.clone();
            projected.kind = PageFormFieldKind::Radio;
            projected.value = Some(value.clone());
            projected.checked = *checked;
        }
    }
    projected
}

pub fn encode_submission(
    submission: Option<&PageFormSubmission>,
    fields: &[PageFormField],
    link_fields: &[String],
) -> Result<Vec<u8>, String> {
    if submission.is_none() && link_fields.is_empty() {
        return Ok(vec![0xc0]);
    }
    let submission = submission.cloned().unwrap_or_default();
    if submission.values.len() > 128 {
        return Err("submitted field count exceeds 128".into());
    }
    for (name, values) in &submission.values {
        if name.is_empty()
            || name.len() > 128
            || values.len() > 128
            || values.iter().any(|value| value.len() > 16 * 1024 || value.contains('\0'))
        {
            return Err("submitted field state exceeds its bound".into());
        }
    }

    let all_fields = link_fields.iter().any(|field| field == "*");
    let mut selected = Vec::new();
    let mut entries = Vec::new();
    for directive in link_fields {
        if directive == "*" {
            continue;
        }
        if let Some((name, value)) = directive.split_once('=') {
            if name.is_empty() || value.contains('=') || name.len() > 124 || value.len() > 16 * 1024
            {
                return Err("invalid NomadNet link assignment".into());
            }
            entries.push((rmpv::Value::from(format!("var_{name}")), rmpv::Value::from(value)));
        } else {
            selected.push(directive.as_str());
        }
    }

    // A submission without any link field directive is an explicit request to
    // send exactly these values (CLI and harness form posts). Interactive links
    // only attach a submission when the Micron link declares fields, so this
    // never widens what an ordinary link sends.
    if link_fields.is_empty() {
        for (name, values) in &submission.values {
            entries.push((
                rmpv::Value::from(format!("field_{name}")),
                rmpv::Value::from(values.join(",")),
            ));
        }
    }

    let mut emitted = std::collections::HashSet::new();
    for field in fields {
        if !(all_fields || selected.iter().any(|name| *name == field.name)) {
            continue;
        }
        let Some(values) = submission.values.get(&field.name) else { continue };
        let value = match field.kind {
            PageFormFieldKind::Text | PageFormFieldKind::Password => values.first().cloned(),
            PageFormFieldKind::Radio => {
                field.value.as_ref().filter(|value| values.contains(value)).cloned()
            }
            PageFormFieldKind::Checkbox => {
                if emitted.contains(&field.name) {
                    continue;
                }
                let checked = fields
                    .iter()
                    .filter(|candidate| {
                        candidate.name == field.name
                            && candidate.kind == PageFormFieldKind::Checkbox
                    })
                    .filter_map(|candidate| candidate.value.as_ref())
                    .filter(|value| values.contains(value))
                    .cloned()
                    .collect::<Vec<_>>();
                (!checked.is_empty()).then(|| checked.join(","))
            }
            PageFormFieldKind::Unknown => None,
        };
        if let Some(value) = value {
            emitted.insert(field.name.clone());
            entries.push((
                rmpv::Value::from(format!("field_{}", field.name)),
                rmpv::Value::from(value),
            ));
        }
    }
    let mut encoded = Vec::new();
    rmpv::encode::write_value(&mut encoded, &rmpv::Value::Map(entries))
        .map_err(|error| format!("encode native submitted fields: {error}"))?;
    Ok(encoded)
}

fn warning(code: &str, message: String) -> PageParserWarning {
    PageParserWarning { code: code.to_string(), message }
}

pub fn decode_binary_response(response: &[u8]) -> Option<Vec<u8>> {
    let mut cursor = std::io::Cursor::new(response);
    let value = rmpv::decode::read_value(&mut cursor).ok()?;
    if usize::try_from(cursor.position()).ok() != Some(response.len()) {
        return None;
    }
    match value {
        rmpv::Value::Binary(bytes) => Some(bytes),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
