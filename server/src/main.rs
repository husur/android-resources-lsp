// lsp_types::Uri holds a Cell (from fluent_uri) but hashes only its string, so it is a safe key.
#![allow(clippy::mutable_key_type)]

use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use android_resources_lsp::references::{self, Reference};
use android_resources_lsp::resources::{self, Kind, Resource, Value};
use android_resources_lsp::uri;
use lsp_server::{Connection, ErrorCode, Message, Notification, Request, RequestId, Response};
use lsp_types::{
    Diagnostic, DiagnosticSeverity, DidChangeTextDocumentParams, DidChangeWatchedFilesParams,
    DidChangeWatchedFilesRegistrationOptions, DidCloseTextDocumentParams,
    DidOpenTextDocumentParams, FileSystemWatcher, GlobPattern, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, InlayHint, InlayHintLabel, InlayHintParams, Location,
    MarkupContent, MarkupKind, OneOf, Position, PublishDiagnosticsParams, Registration,
    RegistrationParams, ServerCapabilities, ServerInfo, TextDocumentPositionParams,
    TextDocumentSyncKind, Uri,
};
use serde_json::{Value as Json, from_value, json};

const NAME: &str = env!("CARGO_PKG_NAME");
/// Longer texts are cut in the hints, like Android Studio folds them.
const HINT_LENGTH: usize = 40;

struct State {
    connection: Connection,
    roots: Vec<PathBuf>,
    /// The resources of each values file.
    files: HashMap<PathBuf, Vec<Resource>>,
    /// The open Kotlin sources.
    documents: HashMap<Uri, String>,
    refresh_support: bool,
    requests: i32,
}

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let (connection, io_threads) = Connection::stdio();
    let (initialize_id, initialize_params) = connection.initialize_start()?;
    #[allow(deprecated)]
    let InitializeParams {
        capabilities,
        workspace_folders,
        root_uri,
        ..
    } = from_value(initialize_params).unwrap_or_default();
    let roots = workspace_folders
        .map(|folders| folders.into_iter().map(|folder| folder.uri).collect())
        .or(root_uri.map(|root| vec![root]))
        .unwrap_or_default()
        .iter()
        .filter_map(|root| uri::to_path(root.as_str()))
        .collect();
    let workspace = capabilities.workspace.unwrap_or_default();
    let watch_support = workspace
        .did_change_watched_files
        .and_then(|watched| watched.dynamic_registration)
        .unwrap_or(false);
    let refresh_support = workspace
        .inlay_hint
        .and_then(|inlay_hint| inlay_hint.refresh_support)
        .unwrap_or(false);
    let initialize_result = InitializeResult {
        capabilities: ServerCapabilities {
            text_document_sync: Some(TextDocumentSyncKind::FULL.into()),
            hover_provider: Some(HoverProviderCapability::Simple(true)),
            definition_provider: Some(OneOf::Left(true)),
            inlay_hint_provider: Some(OneOf::Left(true)),
            ..Default::default()
        },
        server_info: Some(ServerInfo {
            name: NAME.to_string(),
            version: Some(env!("CARGO_PKG_VERSION").to_string()),
        }),
    };
    connection.initialize_finish(initialize_id, serde_json::to_value(initialize_result)?)?;

    let receiver = connection.receiver.clone();
    let mut state = State {
        connection,
        roots,
        files: HashMap::new(),
        documents: HashMap::new(),
        refresh_support,
        requests: 0,
    };
    let found: Vec<PathBuf> = state
        .roots
        .iter()
        .flat_map(|root| resources::scan(root))
        .collect();
    for path in found {
        state.load(path);
    }
    if watch_support {
        state.watch();
    }
    for message in &receiver {
        match message {
            Message::Request(request) => {
                if state.connection.handle_shutdown(&request)? {
                    break;
                }
                state.respond(request);
            }
            Message::Notification(notification) => state.notified(notification),
            Message::Response(_) => {}
        }
    }
    drop(state);
    io_threads.join()?;
    Ok(())
}

impl State {
    fn respond(&self, request: Request) {
        let Request { id, method, params } = request;
        let response = match method.as_str() {
            "textDocument/hover" => match from_value::<HoverParams>(params) {
                Ok(params) => {
                    Response::new_ok(id, self.hover(params.text_document_position_params))
                }
                Err(_) => invalid(id),
            },
            "textDocument/definition" => match from_value::<GotoDefinitionParams>(params) {
                Ok(params) => {
                    Response::new_ok(id, self.definition(params.text_document_position_params))
                }
                Err(_) => invalid(id),
            },
            "textDocument/inlayHint" => match from_value::<InlayHintParams>(params) {
                Ok(params) => Response::new_ok(id, self.inlay_hints(params)),
                Err(_) => invalid(id),
            },
            _ => Response::new_err(id, ErrorCode::MethodNotFound as i32, method),
        };
        self.send(response);
    }

    fn notified(&mut self, notification: Notification) {
        let Notification { method, params } = notification;
        match method.as_str() {
            "textDocument/didOpen" => {
                if let Ok(DidOpenTextDocumentParams { text_document }) = from_value(params) {
                    self.documents
                        .insert(text_document.uri.clone(), text_document.text);
                    self.publish(&text_document.uri);
                }
            }
            "textDocument/didChange" => {
                if let Ok(mut change) = from_value::<DidChangeTextDocumentParams>(params)
                    && let Some(text) = change.content_changes.pop()
                    && let Some(document) = self.documents.get_mut(&change.text_document.uri)
                {
                    *document = text.text;
                    self.publish(&change.text_document.uri);
                }
            }
            "textDocument/didClose" => {
                if let Ok(DidCloseTextDocumentParams { text_document }) = from_value(params) {
                    self.documents.remove(&text_document.uri);
                    self.publish(&text_document.uri);
                }
            }
            "workspace/didChangeWatchedFiles" => {
                if let Ok(DidChangeWatchedFilesParams { changes }) = from_value(params) {
                    let paths: Vec<PathBuf> = changes
                        .iter()
                        .filter_map(|change| uri::to_path(change.uri.as_str()))
                        .filter(|path| self.indexed(path))
                        .collect();
                    for path in paths {
                        self.load(path);
                    }
                    self.resources_changed();
                }
            }
            _ => {}
        }
    }

    /// Reads a values file again, or forgets it once it is gone.
    fn load(&mut self, path: PathBuf) {
        match fs::read_to_string(&path) {
            Ok(xml) => self.files.insert(path, resources::parse(&xml)),
            Err(_) => self.files.remove(&path),
        };
    }

    fn indexed(&self, path: &Path) -> bool {
        self.roots
            .iter()
            .any(|root| path.strip_prefix(root).is_ok_and(resources::indexed))
    }

    /// Strings are added, edited and removed in files that Zed does not send to the server.
    fn watch(&mut self) {
        let options = DidChangeWatchedFilesRegistrationOptions {
            watchers: vec![FileSystemWatcher {
                glob_pattern: GlobPattern::String("**/res/values/*.xml".into()),
                kind: None,
            }],
        };
        let registration = Registration {
            id: "values".into(),
            method: "workspace/didChangeWatchedFiles".into(),
            register_options: serde_json::to_value(options).ok(),
        };
        let params = RegistrationParams {
            registrations: vec![registration],
        };
        self.request("client/registerCapability", json!(params));
    }

    fn resources_changed(&mut self) {
        for uri in self.documents.keys() {
            self.publish(uri);
        }
        if self.refresh_support {
            self.request("workspace/inlayHint/refresh", Json::Null);
        }
    }

    fn hover(&self, params: TextDocumentPositionParams) -> Option<Hover> {
        let (reference, document) = self.reference_at(params)?;
        let value = match self.definitions(&reference, &document).as_slice() {
            [] => return None,
            [(_, resource)] => describe(&resource.value),
            several => several
                .iter()
                .map(|(path, resource)| {
                    format!("{}\n{}", self.relative(path), describe(&resource.value))
                })
                .collect::<Vec<_>>()
                .join("\n\n"),
        };
        Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::PlainText,
                value,
            }),
            range: Some(reference.range),
        })
    }

    fn definition(&self, params: TextDocumentPositionParams) -> Option<GotoDefinitionResponse> {
        let (reference, document) = self.reference_at(params)?;
        let locations: Vec<Location> = self
            .definitions(&reference, &document)
            .into_iter()
            .filter_map(|(path, resource)| {
                Some(Location::new(
                    uri::from_path(path).parse().ok()?,
                    resource.range,
                ))
            })
            .collect();
        (!locations.is_empty()).then_some(GotoDefinitionResponse::Array(locations))
    }

    fn inlay_hints(&self, params: InlayHintParams) -> Vec<InlayHint> {
        let uri = params.text_document.uri;
        let (Some(text), Some(document)) = (self.documents.get(&uri), uri::to_path(uri.as_str()))
        else {
            return vec![];
        };
        let shown = params.range.start..=params.range.end;
        references::find(text)
            .iter()
            .filter(|reference| shown.contains(&reference.range.end))
            .filter_map(|reference| {
                let (_, resource) = self.definitions(reference, &document).into_iter().next()?;
                Some(InlayHint {
                    position: reference.range.end,
                    label: InlayHintLabel::String(hint(&resource.value)),
                    kind: None,
                    text_edits: None,
                    tooltip: None,
                    padding_left: Some(true),
                    padding_right: None,
                    data: None,
                })
            })
            .collect()
    }

    /// Only warns: a string can also come from a library or from `resValue` in a Gradle script.
    /// A qualified `R` is not checked at all, being more likely a library's.
    fn publish(&self, uri: &Uri) {
        let diagnostics = match self.documents.get(uri) {
            Some(text) => references::find(text)
                .into_iter()
                .filter(|reference| !reference.qualified && self.missing(reference))
                .map(|reference| Diagnostic {
                    range: reference.range,
                    severity: Some(DiagnosticSeverity::WARNING),
                    source: Some(NAME.into()),
                    message: format!(
                        "No {} named {} in res/values",
                        match reference.kind {
                            Kind::String => "string",
                            Kind::Plurals => "plurals",
                        },
                        reference.name
                    ),
                    ..Default::default()
                })
                .collect(),
            None => vec![],
        };
        let params = PublishDiagnosticsParams::new(uri.clone(), diagnostics, None);
        self.send(Notification::new(
            "textDocument/publishDiagnostics".into(),
            params,
        ));
    }

    fn reference_at(&self, params: TextDocumentPositionParams) -> Option<(Reference, PathBuf)> {
        let TextDocumentPositionParams {
            text_document,
            position,
        } = params;
        let text = self.documents.get(&text_document.uri)?;
        let reference = references::find(text)
            .into_iter()
            .find(|reference| covers(reference, position))?;
        Some((reference, uri::to_path(text_document.uri.as_str())?))
    }

    fn resources<'a>(
        &'a self,
        reference: &Reference,
    ) -> impl Iterator<Item = (&'a Path, &'a Resource)> {
        self.files.iter().flat_map(move |(path, resources)| {
            resources
                .iter()
                .filter(|resource| {
                    resource.kind() == reference.kind && resource.name == reference.name
                })
                .map(move |resource| (path.as_path(), resource))
        })
    }

    fn missing(&self, reference: &Reference) -> bool {
        self.resources(reference).next().is_none()
    }

    /// The closest to `document` first: its own source set before another, and its module before
    /// another.
    fn definitions(&self, reference: &Reference, document: &Path) -> Vec<(&Path, &Resource)> {
        let mut definitions: Vec<_> = self.resources(reference).collect();
        definitions.sort_by(|(a, _), (b, _)| {
            shared(b, document)
                .cmp(&shared(a, document))
                .then_with(|| a.cmp(b))
        });
        definitions
    }

    fn relative(&self, path: &Path) -> String {
        let relative = self
            .roots
            .iter()
            .find_map(|root| path.strip_prefix(root).ok())
            .unwrap_or(path);
        relative.display().to_string()
    }

    fn request(&mut self, method: &str, params: Json) {
        self.requests += 1;
        self.send(Request::new(self.requests.into(), method.into(), params));
    }

    fn send(&self, message: impl Into<Message>) {
        let _ = self.connection.sender.send(message.into());
    }
}

fn invalid(id: RequestId) -> Response {
    Response::new_err(id, ErrorCode::InvalidParams as i32, "invalid params".into())
}

fn covers(reference: &Reference, position: Position) -> bool {
    reference.range.start <= position && position <= reference.range.end
}

fn shared(a: &Path, b: &Path) -> usize {
    a.components()
        .zip(b.components())
        .take_while(|(a, b)| a == b)
        .count()
}

fn describe(value: &Value) -> String {
    match value {
        Value::Text(text) => text.clone(),
        Value::Plurals(items) => items
            .iter()
            .map(|(quantity, text)| format!("{quantity}: {text}"))
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// The `other` quantity stands for plurals, as it covers most counts.
fn hint(value: &Value) -> String {
    let text = match value {
        Value::Text(text) => text.as_str(),
        Value::Plurals(items) => items
            .iter()
            .find(|(quantity, _)| quantity == "other")
            .or(items.first())
            .map_or("", |(_, text)| text),
    };
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= HINT_LENGTH {
        return format!("\"{text}\"");
    }
    let cut: String = text.chars().take(HINT_LENGTH - 1).collect();
    format!("\"{cut}…\"")
}
