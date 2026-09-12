//! End-to-end tests over a real LSP transport.

mod harness;

use harness::{mini_library, Harness};
use lsp_types::*;

#[test]
fn initialize_advertises_what_it_implements() {
    let mut h = Harness::start("caps", &mini_library());
    // Re-asking is not possible after the handshake, so check the capabilities
    // through behaviour instead: every advertised request must answer.
    let uri = h.open("Test.scd", "SinOsc.ar(440)");

    let _: Option<Hover> = h.request_at("textDocument/hover", &uri, 0, 2);
    let _: Option<GotoDefinitionResponse> = h.request_at("textDocument/definition", &uri, 0, 2);
    let _: Option<CompletionResponse> = h.request_at("textDocument/completion", &uri, 0, 9);
    let _: Option<DocumentSymbolResponse> = h.request(
        "textDocument/documentSymbol",
        serde_json::json!({ "textDocument": { "uri": uri } }),
    );
    let _: Option<WorkspaceSymbolResponse> =
        h.request("workspace/symbol", serde_json::json!({ "query": "Sin" }));
}

#[test]
fn diagnostics_arrive_on_open_and_clear_on_fix() {
    let mut h = Harness::start("diag", &mini_library());
    let uri = h.open("Broken.scd", "SinOsc.");

    let first = h.await_diagnostics(&uri);
    assert_eq!(first.diagnostics.len(), 1);
    assert!(
        first.diagnostics[0].message.contains("method name"),
        "unexpected message: {}",
        first.diagnostics[0].message
    );

    // Complete the expression; the error must go away.
    h.change(
        &uri,
        2,
        Range::new(Position::new(0, 7), Position::new(0, 7)),
        "ar",
    );
    let second = h.await_diagnostics(&uri);
    assert!(second.diagnostics.is_empty(), "{:?}", second.diagnostics);
    assert_eq!(second.version, Some(2));
}

#[test]
fn completion_after_a_dot_on_a_class_is_class_side_and_inherited() {
    let mut h = Harness::start("complete", &mini_library());
    let uri = h.open("Test.scd", "SinOsc.");

    let response: CompletionResponse = h.request_at("textDocument/completion", &uri, 0, 7);
    let CompletionResponse::List(list) = response else {
        panic!("expected a completion list");
    };
    let labels: Vec<_> = list.items.iter().map(|i| i.label.as_str()).collect();

    assert!(labels.contains(&"ar"), "{labels:?}");
    assert!(labels.contains(&"kr"), "{labels:?}");
    // Inherited from UGen.
    assert!(labels.contains(&"multiNew"), "{labels:?}");
    // Instance methods of the *receiver* are not class-side sends.
    assert!(!labels.contains(&"postln"), "{labels:?}");

    let ar = list.items.iter().find(|i| i.label == "ar").unwrap();
    assert_eq!(
        ar.detail.as_deref(),
        Some("*ar(freq = 440, phase = 0, mul = 1, add = 0)")
    );
}

#[test]
fn completion_narrows_as_the_prefix_grows() {
    let mut h = Harness::start("prefix", &mini_library());
    let uri = h.open("Test.scd", "S");

    let response: CompletionResponse = h.request_at("textDocument/completion", &uri, 0, 1);
    let CompletionResponse::List(list) = response else {
        panic!("expected a list");
    };
    let labels: Vec<_> = list.items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(labels, vec!["Saw", "SinOsc"]);
}

#[test]
fn goto_definition_on_a_class_name() {
    let mut h = Harness::start("goto-class", &mini_library());
    let uri = h.open("Test.scd", "SinOsc.ar(440)");

    let response: GotoDefinitionResponse = h.request_at("textDocument/definition", &uri, 0, 2);
    let GotoDefinitionResponse::Scalar(location) = response else {
        panic!("expected exactly one definition");
    };
    assert!(location.uri.path().ends_with("SinOsc.sc"));
    // The name, not the doc comment above it.
    assert_eq!(location.range.start, Position::new(1, 0));
}

#[test]
fn goto_definition_on_a_class_side_selector_resolves_through_the_chain() {
    let mut h = Harness::start("goto-method", &mini_library());
    let uri = h.open("Test.scd", "SinOsc.multiNew(1)");

    // `multiNew` is defined on UGen, not SinOsc.
    let response: GotoDefinitionResponse = h.request_at("textDocument/definition", &uri, 0, 9);
    let GotoDefinitionResponse::Scalar(location) = response else {
        panic!("expected exactly one definition");
    };
    assert!(
        location.uri.path().ends_with("UGen.sc"),
        "{:?}",
        location.uri
    );
}

#[test]
fn goto_definition_on_an_unknown_receiver_offers_every_implementor() {
    let mut h = Harness::start("goto-many", &mini_library());
    let uri = h.open("Test.scd", "x.ar");

    // `ar` is on both SinOsc and Saw, and nothing says which `x` is.
    let response: GotoDefinitionResponse = h.request_at("textDocument/definition", &uri, 0, 2);
    let GotoDefinitionResponse::Array(locations) = response else {
        panic!("expected several definitions");
    };
    assert_eq!(locations.len(), 2);
}

#[test]
fn hover_shows_the_signature_and_the_comment() {
    let mut h = Harness::start("hover", &mini_library());
    let uri = h.open("Test.scd", "SinOsc.ar(440)");

    let hover: Hover = h.request_at("textDocument/hover", &uri, 0, 8);
    let HoverContents::Markup(markup) = hover.contents else {
        panic!("expected markup");
    };
    assert!(markup.value.contains("*ar(freq = 440"), "{}", markup.value);
    assert!(markup.value.contains("SinOsc"), "{}", markup.value);
    // The comment above the method, not the one above the class.
    assert!(markup.value.contains("Audio rate"), "{}", markup.value);
    assert!(
        !markup.value.contains("sine oscillator"),
        "{}",
        markup.value
    );
}

#[test]
fn hover_on_a_class_shows_its_superclass_chain() {
    let mut h = Harness::start("hover-class", &mini_library());
    let uri = h.open("Test.scd", "SinOsc.ar(440)");

    let hover: Hover = h.request_at("textDocument/hover", &uri, 0, 2);
    let HoverContents::Markup(markup) = hover.contents else {
        panic!("expected markup");
    };
    assert!(markup.value.contains("SinOsc : UGen"), "{}", markup.value);
    assert!(markup.value.contains("UGen → Object"), "{}", markup.value);
    assert!(
        markup.value.contains("A sine oscillator"),
        "{}",
        markup.value
    );
}

#[test]
fn document_symbols_nest_methods_under_their_class() {
    let mut h = Harness::start("docsym", &mini_library());
    let uri = h.open(
        "Mine.sc",
        "Mine : Object {\n\tvar <count;\n\t*new { ^super.new }\n\trun { ^1 }\n}\n",
    );

    let response: DocumentSymbolResponse = h.request(
        "textDocument/documentSymbol",
        serde_json::json!({ "textDocument": { "uri": uri } }),
    );
    let DocumentSymbolResponse::Nested(symbols) = response else {
        panic!("expected nested symbols");
    };
    assert_eq!(symbols.len(), 1);
    assert_eq!(symbols[0].name, "Mine");
    assert_eq!(symbols[0].kind, SymbolKind::CLASS);

    let children = symbols[0].children.as_ref().unwrap();
    let names: Vec<_> = children.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"new"), "{names:?}");
    assert!(names.contains(&"run"), "{names:?}");
    // `var <count` generates a getter, which is a real method.
    assert!(names.contains(&"count"), "{names:?}");
}

#[test]
fn workspace_symbols_find_classes_and_methods() {
    let mut h = Harness::start("worksym", &mini_library());

    let response: WorkspaceSymbolResponse =
        h.request("workspace/symbol", serde_json::json!({ "query": "SinOsc" }));
    let WorkspaceSymbolResponse::Flat(symbols) = response else {
        panic!("expected flat symbols");
    };
    assert!(symbols
        .iter()
        .any(|s| s.name == "SinOsc" && s.kind == SymbolKind::CLASS));

    let response: WorkspaceSymbolResponse = h.request(
        "workspace/symbol",
        serde_json::json!({ "query": "multiNew" }),
    );
    let WorkspaceSymbolResponse::Flat(symbols) = response else {
        panic!("expected flat symbols");
    };
    assert!(symbols
        .iter()
        .any(|s| s.name == "*multiNew" && s.container_name.as_deref() == Some("UGen")));
}

#[test]
fn an_unsaved_edit_is_visible_to_the_index_immediately() {
    let mut h = Harness::start("unsaved", &mini_library());
    // A class that exists only in the buffer — never written to disk.
    let uri = h.open("Fresh.sc", "Fresh : Object { boom { ^1 } }");
    let _ = h.await_diagnostics(&uri);

    let response: WorkspaceSymbolResponse =
        h.request("workspace/symbol", serde_json::json!({ "query": "Fresh" }));
    let WorkspaceSymbolResponse::Flat(symbols) = response else {
        panic!("expected flat symbols");
    };
    assert!(
        symbols.iter().any(|s| s.name == "Fresh"),
        "buffer-only class missing from the index: {symbols:?}"
    );
}

#[test]
fn closing_a_buffer_falls_back_to_what_is_on_disk() {
    let mut h = Harness::start("close", &mini_library());
    // Edit SinOsc.sc in the buffer to remove `kr`, then close without saving.
    let uri = h.open("SinOsc.sc", "SinOsc : UGen { *ar { ^1 } }");
    let _ = h.await_diagnostics(&uri);
    h.close(&uri);
    // didClose clears diagnostics, which is also the signal that it landed.
    let cleared = h.await_diagnostics(&uri);
    assert!(cleared.diagnostics.is_empty());

    let doc = h.open("Test.scd", "SinOsc.");
    let _ = h.await_diagnostics(&doc);
    let response: CompletionResponse = h.request_at("textDocument/completion", &doc, 0, 7);
    let CompletionResponse::List(list) = response else {
        panic!("expected a list");
    };
    let labels: Vec<_> = list.items.iter().map(|i| i.label.as_str()).collect();
    // `kr` is back, because the on-disk file has it.
    assert!(labels.contains(&"kr"), "{labels:?}");
}

#[test]
fn an_unknown_request_is_an_error_not_a_crash() {
    let mut h = Harness::start("unknown", &mini_library());
    let id = h.send_request("textDocument/rename", serde_json::json!({}));
    let response = h.await_response(id);
    assert!(response.error.is_some());

    // The server is still alive and answering.
    let uri = h.open("Test.scd", "SinOsc.ar");
    let hover: Option<Hover> = h.request_at("textDocument/hover", &uri, 0, 2);
    assert!(hover.is_some());
}

#[test]
fn signature_help_describes_the_call_being_typed() {
    let mut h = Harness::start("sighelp", &mini_library());
    let uri = h.open("Test.scd", "SinOsc.ar(440, ");

    // Character 15 is the end of the line, in the space after the comma.
    let help: SignatureHelp = h.request_at("textDocument/signatureHelp", &uri, 0, 15);
    assert_eq!(help.signatures.len(), 1);
    assert_eq!(
        help.signatures[0].label,
        "SinOsc.*ar(freq = 440, phase = 0, mul = 1, add = 0)"
    );
    // One comma behind the cursor, so the second parameter is active.
    assert_eq!(help.active_parameter, Some(1));
}

#[test]
fn completion_offers_names_declared_in_the_enclosing_method() {
    let mut h = Harness::start("locals", &mini_library());
    let uri = h.open(
        "Mine.sc",
        "Mine : Object {\n\tvar <count;\n\trun { |freq = 440|\n\t\tvar scaled = 1;\n\t\t^f\n\t}\n}\n",
    );
    let _ = h.await_diagnostics(&uri);

    // Line 4 is `\t\t^f`; the cursor sits after the `f`.
    let response: CompletionResponse = h.request_at("textDocument/completion", &uri, 4, 4);
    let CompletionResponse::List(list) = response else {
        panic!("expected a list");
    };
    let labels: Vec<&str> = list.items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels.contains(&"freq"),
        "the argument in scope: {labels:?}"
    );

    // And it outranks anything global that merely shares the prefix.
    let mut sorted = list.items.clone();
    sorted.sort_by_key(|i| i.sort_text.clone().unwrap_or_else(|| i.label.clone()));
    assert_eq!(sorted[0].label, "freq");

    let freq = list.items.iter().find(|i| i.label == "freq").unwrap();
    assert_eq!(freq.detail.as_deref(), Some("argument = 440"));
}
