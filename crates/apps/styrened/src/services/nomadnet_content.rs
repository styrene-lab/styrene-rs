//! Converts IPC projection contracts to the NomadNet domain without owning policy.
use styrene_ipc::types::{
    PageFormField, PageFormFieldKind, PageFormSubmission, PageLinkTarget, PageParserWarning,
};
use styrene_nomadnet as domain;

pub(super) struct Projection {
    pub text: String,
    pub title: Option<String>,
    pub links: Vec<String>,
    pub fields: Vec<PageFormField>,
    pub link_targets: Vec<PageLinkTarget>,
}

pub(super) fn render_projection(
    document: &styrene_micron::Document,
    warnings: &mut Vec<PageParserWarning>,
) -> Projection {
    let mut domain_warnings = Vec::new();
    let projected = domain::render_projection(document, &mut domain_warnings);
    warnings.extend(domain_warnings.into_iter().map(|warning| {
        let mut dto = PageParserWarning::default();
        dto.code = warning.code;
        dto.message = warning.message;
        dto
    }));
    Projection {
        text: projected.text,
        title: projected.title,
        links: projected.links,
        fields: projected
            .fields
            .into_iter()
            .map(|field| {
                let mut dto = PageFormField::default();
                dto.name = field.name;
                dto.kind = match field.kind {
                    domain::PageFormFieldKind::Text => PageFormFieldKind::Text,
                    domain::PageFormFieldKind::Password => PageFormFieldKind::Password,
                    domain::PageFormFieldKind::Checkbox => PageFormFieldKind::Checkbox,
                    domain::PageFormFieldKind::Radio => PageFormFieldKind::Radio,
                    domain::PageFormFieldKind::Unknown => PageFormFieldKind::Text,
                };
                dto.value = field.value;
                dto.width = field.width;
                dto.checked = field.checked;
                dto
            })
            .collect(),
        link_targets: projected
            .link_targets
            .into_iter()
            .map(|link| {
                let mut dto = PageLinkTarget::default();
                dto.label = link.label;
                dto.target = link.target;
                dto.submitted_fields = link.submitted_fields;
                dto
            })
            .collect(),
    }
}

pub(super) fn encode_submission(
    submission: Option<&PageFormSubmission>,
    fields: &[PageFormField],
    link_fields: &[String],
) -> Result<Vec<u8>, String> {
    let submission = submission.map(|s| domain::PageFormSubmission { values: s.values.clone() });
    let fields = fields
        .iter()
        .map(|field| domain::PageFormField {
            name: field.name.clone(),
            kind: match field.kind {
                PageFormFieldKind::Text => domain::PageFormFieldKind::Text,
                PageFormFieldKind::Password => domain::PageFormFieldKind::Password,
                PageFormFieldKind::Checkbox => domain::PageFormFieldKind::Checkbox,
                PageFormFieldKind::Radio => domain::PageFormFieldKind::Radio,
                _ => domain::PageFormFieldKind::Unknown,
            },
            value: field.value.clone(),
            width: field.width,
            checked: field.checked,
        })
        .collect::<Vec<_>>();
    domain::encode_submission(submission.as_ref(), &fields, link_fields)
}
