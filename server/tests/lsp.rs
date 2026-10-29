use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::process::{self, Child, ChildStdout, Command, ExitStatus, Stdio};
use std::time::Duration;
use std::{env, fs, thread};

use android_resources_lsp::uri;
use lsp_server::{Message, Notification, Request, RequestId, Response, ResponseError};
use serde_json::{Value, json};

const MAIN_STRINGS: &str = "app/src/main/res/values/strings.xml";
const DEBUG_STRINGS: &str = "app/src/debug/res/values/strings.xml";
const BUILD_STRINGS: &str = "app/build/res/values/strings.xml";
const SCREEN: &str = "app/src/main/java/com/example/Screen.kt";
const DEBUG_SCREEN: &str = "app/src/debug/java/com/example/Debug.kt";

const STRINGS: &str = r#"<resources>
    <string name="app_name">DatiPlus</string>
    <string name="title">Alerte</string>
    <string name="long_text">Une très longue phrase qui ne tient pas dans un indice</string>
    <plurals name="alerts">
        <item quantity="one">%d alerte</item>
        <item quantity="other">%d alertes</item>
    </plurals>
</resources>
"#;

const SOURCE: &str = r#"package com.example

fun Screen(count: Int) {
    Text(stringResource(R.string.title))
    Text(pluralStringResource(R.plurals.alerts, count, count))
    Text(stringResource(R.string.long_text))
    Text(stringResource(R.string.missing))
    Text(stringResource(com.example.lib.R.string.gone))
    Text(stringResource(android.R.string.ok))
    Text(stringResource(R.string.app_name))
}
"#;

/// A project of its own for each test, removed at the end.
struct Project {
    root: PathBuf,
}

impl Project {
    fn new(test: &str) -> Self {
        let root = env::temp_dir().join(format!("android-resources-lsp-{}-{test}", process::id()));
        let _ = fs::remove_dir_all(&root);
        let project = Self { root };
        project.write(MAIN_STRINGS, STRINGS);
        project.write(
            DEBUG_STRINGS,
            r#"<resources><string name="app_name">DatiPlus (debug)</string></resources>"#,
        );
        project
    }

    fn write(&self, file: &str, text: &str) {
        let path = self.root.join(file);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn uri(&self, file: &str) -> String {
        uri::from_path(&self.root.join(file))
    }

    fn workspace(&self) -> Value {
        json!({ "workspaceFolders": [{ "uri": self.uri(""), "name": "project" }] })
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// The language server binary, driven over stdio like Zed does.
struct Server {
    process: Child,
    output: BufReader<ChildStdout>,
    next_id: i32,
    /// The requests the server sent, which are answered like Zed does.
    requests: Vec<Request>,
    notifications: Vec<Notification>,
}

impl Server {
    fn spawn() -> Self {
        let mut process = Command::new(env!("CARGO_BIN_EXE_android-resources-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let output = BufReader::new(process.stdout.take().unwrap());
        Self {
            process,
            output,
            next_id: 0,
            requests: vec![],
            notifications: vec![],
        }
    }

    /// Starts in `project`, with its Kotlin source open.
    fn start(project: &Project, capabilities: Value) -> Self {
        let mut params = project.workspace();
        params["capabilities"] = capabilities;
        let mut server = Self::spawn();
        server.initialize(params);
        server.open(&project.uri(SCREEN), SOURCE);
        server
    }

    fn initialize(&mut self, params: Value) -> Value {
        let result = self.request("initialize", params).unwrap();
        self.notify("initialized", json!({}));
        result
    }

    fn request(&mut self, method: &str, params: Value) -> Result<Value, ResponseError> {
        self.next_id += 1;
        let id = RequestId::from(self.next_id);
        self.send(Request::new(id.clone(), method.to_string(), params).into());
        loop {
            match Message::read(&mut self.output).unwrap() {
                Some(Message::Response(response)) if response.id == id => {
                    return response.response_result;
                }
                Some(Message::Request(request)) => {
                    self.send(Response::new_ok(request.id.clone(), ()).into());
                    self.requests.push(request);
                }
                Some(Message::Notification(notification)) => self.notifications.push(notification),
                Some(_) => {}
                None => panic!("the server stopped before answering {method}"),
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(Notification::new(method.to_string(), params).into());
    }

    fn send(&mut self, message: Message) {
        message.write(self.process.stdin.as_mut().unwrap()).unwrap();
    }

    fn open(&mut self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({ "textDocument": { "uri": uri, "languageId": "kotlin", "version": 1, "text": text } }),
        );
    }

    fn change(&mut self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": 2 },
                "contentChanges": [{ "text": text }],
            }),
        );
    }

    /// The server handles messages in order: once it answers a request, it has published the
    /// diagnostics of everything sent before.
    fn diagnostics(&mut self, uri: &str) -> Vec<Value> {
        self.request("textDocument/hover", position("file:///none.kt", 0, 0))
            .unwrap();
        let published = self.notifications.iter().rev().find(|notification| {
            notification.method == "textDocument/publishDiagnostics"
                && notification.params["uri"] == uri
        });
        let params = &published.expect("no diagnostics published").params;
        params["diagnostics"].as_array().unwrap().clone()
    }

    fn hints(&mut self, uri: &str, lines: (u32, u32)) -> Value {
        let params =
            json!({ "textDocument": { "uri": uri }, "range": range(lines.0, 0, lines.1, 0) });
        self.request("textDocument/inlayHint", params).unwrap()
    }

    /// The label of each hint, after its line and character.
    fn labels(&mut self, uri: &str) -> Vec<(u64, u64, String)> {
        let hints = self.hints(uri, (0, 99));
        hints
            .as_array()
            .unwrap()
            .iter()
            .map(|hint| {
                (
                    hint["position"]["line"].as_u64().unwrap(),
                    hint["position"]["character"].as_u64().unwrap(),
                    hint["label"].as_str().unwrap().to_string(),
                )
            })
            .collect()
    }

    fn hover(&mut self, uri: &str, line: u32, character: u32) -> Value {
        self.request("textDocument/hover", position(uri, line, character))
            .unwrap()
    }

    fn definition(&mut self, uri: &str, line: u32, character: u32) -> Value {
        self.request("textDocument/definition", position(uri, line, character))
            .unwrap()
    }

    /// Gives up after a few seconds, so that a server that hangs fails the test instead of blocking it.
    fn exited(&mut self) -> Option<ExitStatus> {
        for _ in 0..500 {
            if let Some(status) = self.process.try_wait().unwrap() {
                return Some(status);
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = self.process.kill();
        let _ = self.process.wait();
        None
    }

    /// Closing stdin ends the server normally, which is when it writes its coverage data.
    fn stop(&mut self) -> String {
        drop(self.process.stdin.take());
        self.exited();
        let mut errors = String::new();
        if let Some(mut stderr) = self.process.stderr.take() {
            let _ = stderr.read_to_string(&mut errors);
        }
        errors
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        eprint!("{}", self.stop());
    }
}

fn position(uri: &str, line: u32, character: u32) -> Value {
    json!({ "textDocument": { "uri": uri }, "position": { "line": line, "character": character } })
}

fn range(start_line: u32, start_character: u32, end_line: u32, end_character: u32) -> Value {
    json!({
        "start": { "line": start_line, "character": start_character },
        "end": { "line": end_line, "character": end_character },
    })
}

fn warning(line: u32, start: u32, end: u32, message: &str) -> Value {
    json!({
        "range": range(line, start, line, end),
        "severity": 2,
        "source": "android-resources-lsp",
        "message": message,
    })
}

#[test]
fn introduces_itself_and_its_capabilities() {
    let mut server = Server::spawn();
    let result = server.initialize(json!({ "capabilities": {} }));
    assert_eq!(result["serverInfo"]["name"], "android-resources-lsp");
    assert_eq!(result["capabilities"]["textDocumentSync"], 1);
    assert_eq!(result["capabilities"]["hoverProvider"], true);
    assert_eq!(result["capabilities"]["definitionProvider"], true);
    assert_eq!(result["capabilities"]["inlayHintProvider"], true);
}

#[test]
fn shows_each_text_after_its_reference() {
    let project = Project::new("hints");
    let mut server = Server::start(&project, json!({}));
    assert_eq!(
        server.labels(&project.uri(SCREEN)),
        [
            (3, 38, r#""Alerte""#.to_string()),
            (4, 46, r#""%d alertes""#.to_string()),
            (
                5,
                42,
                r#""Une très longue phrase qui ne tient pas…""#.to_string()
            ),
            (9, 41, r#""DatiPlus""#.to_string()),
        ]
    );
    let shown = server.hints(&project.uri(SCREEN), (4, 5));
    assert_eq!(shown.as_array().unwrap().len(), 1);
    assert_eq!(shown[0]["paddingLeft"], true);
    assert_eq!(server.labels("file:///closed.kt"), []);
}

#[test]
fn prefers_the_source_set_of_the_source() {
    let project = Project::new("source-sets");
    let mut server = Server::start(&project, json!({}));
    let debug = project.uri(DEBUG_SCREEN);
    server.open(&debug, "Text(stringResource(R.string.app_name))");
    assert_eq!(
        server.labels(&debug),
        [(0, 37, r#""DatiPlus (debug)""#.to_string())]
    );
}

#[test]
fn shows_the_text_on_hover() {
    let project = Project::new("hover");
    let mut server = Server::start(&project, json!({}));
    let screen = project.uri(SCREEN);
    assert_eq!(
        server.hover(&screen, 3, 35),
        json!({
            "contents": { "kind": "plaintext", "value": "Alerte" },
            "range": range(3, 33, 3, 38),
        })
    );
    assert_eq!(
        server.hover(&screen, 4, 46)["contents"]["value"],
        "one: %d alerte\nother: %d alertes"
    );
    assert_eq!(
        server.hover(&screen, 9, 33)["contents"]["value"],
        "app/src/main/res/values/strings.xml\nDatiPlus\n\napp/src/debug/res/values/strings.xml\nDatiPlus (debug)"
    );
    assert_eq!(server.hover(&screen, 0, 0), Value::Null);
    assert_eq!(server.hover(&screen, 6, 35), Value::Null);
    assert_eq!(server.hover("file:///closed.kt", 3, 35), Value::Null);
}

#[test]
fn goes_to_every_definition_closest_first() {
    let project = Project::new("definition");
    let mut server = Server::start(&project, json!({}));
    let screen = project.uri(SCREEN);
    assert_eq!(
        server.definition(&screen, 9, 35),
        json!([
            { "uri": project.uri(MAIN_STRINGS), "range": range(1, 18, 1, 26) },
            { "uri": project.uri(DEBUG_STRINGS), "range": range(0, 25, 0, 33) },
        ])
    );
    assert_eq!(server.definition(&screen, 6, 35), Value::Null);
}

#[test]
fn warns_about_missing_strings() {
    let project = Project::new("diagnostics");
    let mut server = Server::start(&project, json!({}));
    let screen = project.uri(SCREEN);
    assert_eq!(
        server.diagnostics(&screen),
        [warning(6, 33, 40, "No string named missing in res/values")]
    );
    server.change(&screen, "R.string.title\nR.plurals.gone");
    assert_eq!(
        server.diagnostics(&screen),
        [warning(1, 10, 14, "No plurals named gone in res/values")]
    );
    server.notify(
        "textDocument/didClose",
        json!({ "textDocument": { "uri": screen } }),
    );
    assert!(server.diagnostics(&screen).is_empty());
}

#[test]
fn follows_the_values_files_on_disk() {
    let project = Project::new("watch");
    let capabilities = json!({
        "workspace": {
            "didChangeWatchedFiles": { "dynamicRegistration": true },
            "inlayHint": { "refreshSupport": true },
        },
    });
    let mut server = Server::start(&project, capabilities);
    let screen = project.uri(SCREEN);
    assert_eq!(server.diagnostics(&screen).len(), 1);
    let registration = &server.requests[0];
    assert_eq!(registration.method, "client/registerCapability");
    assert_eq!(
        registration.params["registrations"][0]["registerOptions"]["watchers"][0]["globPattern"],
        "**/res/values/*.xml"
    );

    let fixed = STRINGS.replace(
        "</resources>",
        "<string name=\"missing\">Là</string></resources>",
    );
    project.write(MAIN_STRINGS, &fixed);
    project.write(BUILD_STRINGS, &fixed);
    let changed = |file: &str, kind: u8| json!({ "uri": project.uri(file), "type": kind });
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({ "changes": [changed(MAIN_STRINGS, 2), changed(BUILD_STRINGS, 1)] }),
    );
    assert!(server.diagnostics(&screen).is_empty());
    assert_eq!(server.requests[1].method, "workspace/inlayHint/refresh");

    fs::remove_file(project.root.join(MAIN_STRINGS)).unwrap();
    server.notify(
        "workspace/didChangeWatchedFiles",
        json!({ "changes": [changed(MAIN_STRINGS, 3)] }),
    );
    assert_eq!(server.diagnostics(&screen).len(), 4);
    assert_eq!(
        server.labels(&screen),
        [(9, 41, r#""DatiPlus (debug)""#.to_string())]
    );
}

#[test]
fn takes_the_root_uri_of_older_clients() {
    let project = Project::new("root-uri");
    let mut server = Server::spawn();
    server.initialize(json!({ "rootUri": project.uri(""), "capabilities": {} }));
    server.open(&project.uri(SCREEN), SOURCE);
    assert_eq!(server.labels(&project.uri(SCREEN)).len(), 4);
}

#[test]
fn rejects_invalid_and_unknown_requests() {
    let mut server = Server::spawn();
    server.initialize(json!({ "capabilities": {} }));
    for method in [
        "textDocument/hover",
        "textDocument/definition",
        "textDocument/inlayHint",
    ] {
        let error = server.request(method, json!({})).unwrap_err();
        assert_eq!(error.code, -32602, "{method}");
    }
    let error = server
        .request("textDocument/completion", json!({}))
        .unwrap_err();
    assert_eq!(error.code, -32601);
}

#[test]
fn stops_on_shutdown_then_exit() {
    let mut server = Server::spawn();
    server.initialize(json!({ "capabilities": {} }));
    assert_eq!(
        server.request("shutdown", Value::Null).unwrap(),
        Value::Null
    );
    server.notify("exit", Value::Null);
    assert!(server.exited().unwrap().success());
}
