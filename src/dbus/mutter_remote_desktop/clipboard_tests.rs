use std::collections::HashMap;

use zbus::zvariant::Value;

use super::clipboard::parse_clipboard_mime_types;

#[test]
fn parses_plain_mime_type_arrays() {
    let options = HashMap::from([(
        "mime-types",
        Value::new(vec![
            "text/plain;charset=utf-8".to_owned(),
            "text/plain".to_owned(),
        ]),
    )]);

    let mime_types = parse_clipboard_mime_types(&options).unwrap().unwrap();

    assert_eq!(mime_types, ["text/plain;charset=utf-8", "text/plain"]);
}

#[test]
fn rejects_empty_mime_type_arrays() {
    let options = HashMap::from([("mime-types", Value::new(Vec::<String>::new()))]);

    let err = parse_clipboard_mime_types(&options).unwrap_err();

    assert!(format!("{err:?}").contains("must not be empty"));
}
