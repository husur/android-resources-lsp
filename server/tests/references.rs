use android_resources_lsp::references::{Reference, find};
use android_resources_lsp::resources::Kind;
use lsp_types::{Position, Range};

fn reference(kind: Kind, name: &str, line: u32, start: u32, qualified: bool) -> Reference {
    Reference {
        kind,
        name: name.to_string(),
        range: Range::new(
            Position::new(line, start),
            Position::new(line, start + name.len() as u32),
        ),
        qualified,
    }
}

#[test]
fn finds_strings_and_plurals_in_order() {
    let source = r#"Text(stringResource(R.plurals.alerts, count))
Text(stringResource(R.string.title) + "🚀" + stringResource(R.string.subtitle_2))"#;
    assert_eq!(
        find(source),
        [
            reference(Kind::Plurals, "alerts", 0, 30, false),
            reference(Kind::String, "title", 1, 29, false),
            reference(Kind::String, "subtitle_2", 1, 69, false),
        ]
    );
}

#[test]
fn tells_a_qualified_r_apart() {
    assert_eq!(
        find("getString(com.example.R.string.title)"),
        [reference(Kind::String, "title", 0, 31, true)]
    );
}

#[test]
fn leaves_out_the_platform_and_other_classes() {
    let source = "android.R.string.ok BR.string.title R.string. R.drawable.icon";
    assert_eq!(find(source), []);
}

#[test]
fn keeps_an_app_whose_package_ends_with_android() {
    assert_eq!(
        find("com.example.android.R.string.title"),
        [reference(Kind::String, "title", 0, 29, true)]
    );
}

#[test]
fn reads_a_name_up_to_the_end_of_the_text() {
    assert_eq!(
        find("R.string.title"),
        [reference(Kind::String, "title", 0, 9, false)]
    );
}

#[test]
fn skips_commented_out_code() {
    let source = r#"// R.string.line
/* R.string.block /* R.string.nested */ R.string.still */ R.string.after
/** [R.string.kdoc] */"#;
    assert_eq!(
        find(source),
        [reference(Kind::String, "after", 1, 67, false)]
    );
}

#[test]
fn reads_past_strings_and_characters() {
    let source = r#"val a = "https://x.com" + R.string.url
val b = '"' + R.string.quote + "\"//" + R.string.escaped
val c = """// raw""" + R.string.raw
val d = "unterminated
val e = R.string.next /* open"#;
    let names: Vec<String> = find(source).into_iter().map(|r| r.name).collect();
    assert_eq!(names, ["url", "quote", "escaped", "raw", "next"]);
}
