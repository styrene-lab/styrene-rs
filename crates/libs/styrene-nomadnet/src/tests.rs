#![allow(clippy::unwrap_used)]
use super::*;
#[test]
fn form_projection_redacts_passwords_and_submission_is_native_messagepack() {
    let document =
        styrene_micron::parse("`<name`Ada> `<12!|password`secret> `[Submit`next.mu`name|password]");
    let page = render_projection(&document, &mut Vec::new());
    assert_eq!(page.fields.len(), 2);
    assert_eq!(page.fields[0].value.as_deref(), Some("Ada"));
    assert_eq!(page.fields[1].kind, PageFormFieldKind::Password);
    assert_eq!(page.fields[1].value, None);
    assert_eq!(page.link_targets[0].submitted_fields, ["name", "password"]);

    let mut submission = PageFormSubmission::default();
    submission.values.insert("name".into(), vec!["Grace".into()]);
    submission.values.insert("password".into(), vec!["swordfish".into()]);
    submission.values.insert("opts".into(), vec!["blue".into(), "red".into()]);
    assert!(!format!("{submission:?}").contains("swordfish"));
    let checkbox_red = PageFormField {
        name: "opts".into(),
        kind: PageFormFieldKind::Checkbox,
        value: Some("red".into()),
        ..Default::default()
    };
    let mut checkbox_blue = checkbox_red.clone();
    checkbox_blue.value = Some("blue".into());
    let encoded = encode_submission(
        Some(&submission),
        &[page.fields[0].clone(), page.fields[1].clone(), checkbox_red, checkbox_blue],
        &["mode=safe".into(), "*".into()],
    )
    .expect("native map");
    assert_eq!(
        encoded.iter().map(|byte| format!("{byte:02x}")).collect::<String>(),
        "84a87661725f6d6f6465a473616665aa6669656c645f6e616d65a54772616365ae6669656c645f70617373776f7264a973776f726466697368aa6669656c645f6f707473a87265642c626c7565"
    );
    let decoded = rmpv::decode::read_value(&mut std::io::Cursor::new(encoded)).unwrap();
    assert!(matches!(decoded, rmpv::Value::Map(values) if values.len() == 4));
}

#[test]
fn explicit_submission_without_link_directive_sends_named_fields() {
    let mut submission = PageFormSubmission::default();
    submission.values.insert("name".into(), vec!["rust".into()]);
    submission.values.insert("opts".into(), vec!["red".into(), "blue".into()]);
    let encoded = encode_submission(Some(&submission), &[], &[]).expect("native map");
    let decoded = rmpv::decode::read_value(&mut std::io::Cursor::new(encoded)).unwrap();
    let rmpv::Value::Map(values) = decoded else { panic!("map") };
    assert_eq!(
        values,
        vec![
            (rmpv::Value::from("field_name"), rmpv::Value::from("rust")),
            (rmpv::Value::from("field_opts"), rmpv::Value::from("red,blue")),
        ]
    );
    assert_eq!(encode_submission(None, &[], &[]).expect("nil"), vec![0xc0]);
}

#[test]
fn rejects_non_binary_and_trailing_native_response_bytes() {
    assert_eq!(decode_binary_response(&[0xc4, 2, 65, 66]), Some(b"AB".to_vec()));
    assert_eq!(decode_binary_response(&[0xc4, 2, 65, 66, 0]), None);
    assert_eq!(decode_binary_response(&[0xa2, 65, 66]), None);
    assert_eq!(decode_binary_response(&[0xc4, 3, 65]), None);
}

#[test]
fn invalid_submission_does_not_encode() {
    let mut submission = PageFormSubmission::default();
    submission.values.insert("".into(), vec!["bad".into()]);
    assert!(encode_submission(Some(&submission), &[], &[]).is_err());
    assert!(encode_submission(None, &[], &["mode=bad=extra".into()]).is_err());
}

#[test]
fn projection_preserves_text_links_and_directive_warnings() {
    let document = styrene_micron::parse(">Heading\nA `[link`/page/next.mu]\n");
    let projection = render_projection(&document, &mut Vec::new());
    assert_eq!(projection.title.as_deref(), Some("Heading"));
    assert_eq!(projection.text, "Heading\nA link");
    assert_eq!(projection.links, ["/page/next.mu"]);
    let document =
        Document { blocks: vec![Block::Directive { key: "c".into(), value: "0".into() }] };
    let mut warnings = Vec::new();
    render_projection(&document, &mut warnings);
    assert_eq!(warnings[0].code, "directive_not_rendered");
}

#[test]
fn binary_response_accepts_only_exact_bounded_binary_payloads() {
    for value in [vec![0xc4, 1, 42], vec![0xc5, 0, 1, 42], vec![0xc6, 0, 0, 0, 1, 42]] {
        assert_eq!(decode_binary_response(&value), Some(vec![42]));
    }
    assert_eq!(decode_binary_response(&[0xc4, 0]), Some(vec![]));
    for value in [
        vec![0xc6, 255, 255, 255, 255],
        vec![0xc5, 0],
        vec![0xc4, 0, 42],
        vec![0x91, 0xc4, 0],
        vec![0x80],
    ] {
        assert_eq!(decode_binary_response(&value), None);
    }
}
