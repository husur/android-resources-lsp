use std::path::{Path, PathBuf};

use android_resources_lsp::uri::{from_path, to_path};

#[test]
fn round_trips_a_path_with_spaces_and_accents() {
    let path = Path::new("/Users/me/Mes projets/Café/strings.xml");
    let uri = from_path(path);
    assert_eq!(uri, "file:///Users/me/Mes%20projets/Caf%C3%A9/strings.xml");
    assert_eq!(to_path(&uri).as_deref(), Some(path));
}

#[test]
fn handles_windows_drives() {
    assert_eq!(
        to_path("file:///C:/app/strings.xml"),
        Some(PathBuf::from("C:/app/strings.xml"))
    );
    assert_eq!(
        to_path("file:///c%3A/app/strings.xml"),
        Some(PathBuf::from("c:/app/strings.xml"))
    );
    assert_eq!(
        from_path(Path::new(r"C:\app\strings.xml")),
        "file:///C:/app/strings.xml"
    );
}

#[test]
fn rejects_what_is_not_a_file_path() {
    assert_eq!(to_path("https://example.com/strings.xml"), None);
    assert_eq!(to_path("file:///broken%2"), None);
    assert_eq!(to_path("file:///broken%zz"), None);
    assert_eq!(to_path("file:///broken%FF"), None);
}
