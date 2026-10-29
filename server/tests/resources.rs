use std::path::Path;
use std::{env, fs, process};

use android_resources_lsp::resources::{Kind, Resource, Value, indexed, parse, scan};
use lsp_types::{Position, Range};

const STRINGS: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<resources xmlns:xliff="urn:oasis:names:tc:xliff:document:1.2">
    <!-- A comment -->
    <string name="title">Alerte</string>
    <string name="escaped">L\'alerte de \"Jo\"\nest   partie</string>
    <string name="quoted">"  deux  espaces "</string>
    <string name="tagged">Bonjour <xliff:g id="name">%1$s</xliff:g> !</string>
    <string name="empty"></string>
    <plurals name="alerts">
        <item quantity="one">%d alerte</item>
        <item quantity="other">%d alertes</item>
        <item>no quantity</item>
    </plurals>
    <color name="red">#FF0000</color>
    <string>no name</string>
</resources>
"#;

fn text(name: &str) -> Value {
    let resources = parse(STRINGS);
    let resource = resources.iter().find(|r| r.name == name).unwrap();
    assert_eq!(resource.kind(), Kind::String);
    match &resource.value {
        Value::Text(text) => Value::Text(text.clone()),
        other => panic!("{name} is {other:?}"),
    }
}

#[test]
fn reads_strings_as_the_app_shows_them() {
    assert_eq!(text("title"), Value::Text("Alerte".into()));
    assert_eq!(
        text("escaped"),
        Value::Text("L'alerte de \"Jo\"\nest partie".into())
    );
    assert_eq!(text("quoted"), Value::Text("  deux  espaces ".into()));
    assert_eq!(text("tagged"), Value::Text("Bonjour %1$s !".into()));
    assert_eq!(text("empty"), Value::Text(String::new()));
}

#[test]
fn reads_the_quantities_of_plurals() {
    let resources = parse(STRINGS);
    let alerts = resources.iter().find(|r| r.name == "alerts").unwrap();
    assert_eq!(alerts.kind(), Kind::Plurals);
    assert_eq!(
        alerts.value,
        Value::Plurals(vec![
            ("one".into(), "%d alerte".into()),
            ("other".into(), "%d alertes".into()),
        ])
    );
}

#[test]
fn locates_each_name_and_skips_other_resources() {
    let resources = parse(STRINGS);
    let names: Vec<&str> = resources.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        names,
        ["title", "escaped", "quoted", "tagged", "empty", "alerts"]
    );
    assert_eq!(
        resources[0],
        Resource {
            name: "title".into(),
            value: Value::Text("Alerte".into()),
            range: Range::new(Position::new(3, 18), Position::new(3, 23)),
        }
    );
}

#[test]
fn reads_nothing_from_broken_xml() {
    assert_eq!(parse("<resources><string name=\"a\">"), []);
}

#[test]
fn ends_a_trailing_backslash() {
    let resources = parse(r#"<resources><string name="a">a\</string></resources>"#);
    assert_eq!(resources[0].value, Value::Text("a".into()));
}

#[test]
fn finds_the_values_files_outside_build_outputs() {
    let root = env::temp_dir().join(format!("android-resources-lsp-scan-{}", process::id()));
    let files = [
        "app/src/main/res/values/strings.xml",
        "app/src/debug/res/values/strings.xml",
        "app/src/main/res/values-fr/strings.xml",
        "app/src/main/res/values/notes.txt",
        "app/src/main/values/strings.xml",
        "app/build/res/values/strings.xml",
        ".idea/res/values/strings.xml",
    ];
    for file in files {
        let path = root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "<resources/>").unwrap();
    }
    assert_eq!(
        scan(&root),
        [
            root.join("app/src/debug/res/values/strings.xml"),
            root.join("app/src/main/res/values/strings.xml"),
        ]
    );
    for file in files {
        assert_eq!(
            indexed(Path::new(file)),
            scan(&root).contains(&root.join(file))
        );
    }
    fs::remove_dir_all(root).unwrap();
}
