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
    let id = h.send_request("textDocument/semanticTokens/full", serde_json::json!({}));
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

#[test]
fn goto_definition_on_a_local_variable() {
    let mut h = Harness::start("goto-local", &mini_library());
    let uri = h.open("Test.scd", "{\n\tvar foo = 1;\n\n\tfoo.postln;\n}\n");
    let _ = h.await_diagnostics(&uri);

    // Line 3 is `\tfoo.postln;` — the cursor sits on the use of `foo`.
    let response: GotoDefinitionResponse = h.request_at("textDocument/definition", &uri, 3, 2);
    let GotoDefinitionResponse::Scalar(location) = response else {
        panic!("expected exactly one definition");
    };
    // Line 1 is `\tvar foo = 1;`, and `foo` starts at character 5.
    assert_eq!(location.uri, uri);
    assert_eq!(location.range.start, Position::new(1, 5));
    assert_eq!(location.range.end, Position::new(1, 8));
}

#[test]
fn goto_definition_on_an_argument_from_inside_the_body() {
    let mut h = Harness::start("goto-arg", &mini_library());
    let uri = h.open(
        "Mine.sc",
        "Mine : Object {\n\trun { |freq = 440|\n\t\t^freq\n\t}\n}\n",
    );
    let _ = h.await_diagnostics(&uri);

    let response: GotoDefinitionResponse = h.request_at("textDocument/definition", &uri, 2, 4);
    let GotoDefinitionResponse::Scalar(location) = response else {
        panic!("expected exactly one definition");
    };
    assert_eq!(location.range.start, Position::new(1, 8));
}

#[test]
fn an_inner_declaration_wins_over_an_outer_one() {
    let mut h = Harness::start("goto-shadow", &mini_library());
    // Two bindings called `v`; the use must resolve to the inner one.
    let uri = h.open("Test.scd", "{ |v|\n\t{ |v|\n\t\tv.postln;\n\t};\n}\n");
    let _ = h.await_diagnostics(&uri);

    let response: GotoDefinitionResponse = h.request_at("textDocument/definition", &uri, 2, 2);
    let GotoDefinitionResponse::Scalar(location) = response else {
        panic!("expected exactly one definition");
    };
    assert_eq!(
        location.range.start.line, 1,
        "should resolve to the inner `v`"
    );
}

#[test]
fn hover_on_a_local_describes_the_binding() {
    let mut h = Harness::start("hover-local", &mini_library());
    let uri = h.open("Test.scd", "{\n\tvar foo = 1 + 2;\n\n\tfoo.postln;\n}\n");
    let _ = h.await_diagnostics(&uri);

    let hover: Hover = h.request_at("textDocument/hover", &uri, 3, 2);
    let HoverContents::Markup(markup) = hover.contents else {
        panic!("expected markup");
    };
    assert!(markup.value.contains("var foo = 1 + 2"), "{}", markup.value);
    assert!(markup.value.contains("variable"), "{}", markup.value);
}

#[test]
fn completion_offers_parameter_names_inside_a_call() {
    let mut h = Harness::start("kwargs", &mini_library());
    let uri = h.open("Test.scd", "SinOsc.ar(");

    let response: CompletionResponse = h.request_at("textDocument/completion", &uri, 0, 10);
    let CompletionResponse::List(list) = response else {
        panic!("expected a list");
    };
    let labels: Vec<&str> = list.items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"freq:"), "{labels:?}");
    assert!(labels.contains(&"phase:"), "{labels:?}");
}

#[test]
fn inlay_hints_name_positional_arguments() {
    let mut h = Harness::start("inlay", &mini_library());
    let uri = h.open("Test.scd", "SinOsc.ar(440, 0)\n");
    let _ = h.await_diagnostics(&uri);

    let hints: Vec<InlayHint> = h.request(
        "textDocument/inlayHint",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 1, "character": 0 },
            },
        }),
    );
    let labels: Vec<String> = hints
        .iter()
        .map(|h| match &h.label {
            InlayHintLabel::String(s) => s.clone(),
            _ => panic!("expected a string label"),
        })
        .collect();
    assert_eq!(labels, vec!["freq:", "phase:"]);
    assert_eq!(hints[0].position, Position::new(0, 10));
}

#[test]
fn no_inlay_hints_when_the_receiver_is_unknown() {
    let mut h = Harness::start("inlay-unknown", &mini_library());
    let uri = h.open("Test.scd", "x.ar(440, 0)\n");
    let _ = h.await_diagnostics(&uri);

    let hints: Vec<InlayHint> = h.request(
        "textDocument/inlayHint",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "range": {
                "start": { "line": 0, "character": 0 },
                "end": { "line": 1, "character": 0 },
            },
        }),
    );
    assert!(
        hints.is_empty(),
        "labelling these would be a guess: {hints:?}"
    );
}

#[test]
fn malformed_params_are_answered_rather_than_ignored() {
    // A request must always get a response. Dropping one leaves the client
    // waiting on an id that never comes, which is a hang rather than a
    // failure and much harder to diagnose from the other end.
    let mut h = Harness::start("bad-params", &mini_library());
    let id = h.send_request("textDocument/rename", serde_json::json!({}));
    let response = h.await_response(id);
    assert!(response.error.is_some(), "{response:?}");
}

#[test]
fn references_to_a_class_span_the_workspace() {
    let mut h = Harness::start("refs-class", &mini_library());
    let uri = h.open("Test.scd", "SinOsc.ar(440)");

    let found: Vec<Location> = h.request(
        "textDocument/references",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 2 },
            "context": { "includeDeclaration": true },
        }),
    );
    // The definition in SinOsc.sc and the use in the open buffer.
    assert!(found.len() >= 2, "{found:?}");
    assert!(found.iter().any(|l| l.uri.path().ends_with("SinOsc.sc")));
    assert!(found.iter().any(|l| l.uri == uri));
}

#[test]
fn references_to_a_local_stay_in_its_scope() {
    let mut h = Harness::start("refs-local", &mini_library());
    let uri = h.open("Test.scd", "{\n\tvar foo = 1;\n\tfoo + foo;\n}\n");
    let _ = h.await_diagnostics(&uri);

    let found: Vec<Location> = h.request(
        "textDocument/references",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 2 },
            "context": { "includeDeclaration": true },
        }),
    );
    // The declaration and both uses, all in this buffer.
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(found.iter().all(|l| l.uri == uri));
}

#[test]
fn rename_rewrites_a_local_and_only_it() {
    let mut h = Harness::start("rename-local", &mini_library());
    let uri = h.open("Test.scd", "{\n\tvar foo = 1;\n\tfoo + foo;\n}\n");
    let _ = h.await_diagnostics(&uri);

    let edit: WorkspaceEdit = h.request(
        "textDocument/rename",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 2, "character": 2 },
            "newName": "bar",
        }),
    );
    let changes = edit.changes.expect("changes");
    let edits = changes.get(&uri).expect("edits for this file");
    assert_eq!(edits.len(), 3);
    assert!(edits.iter().all(|e| e.new_text == "bar"));
}

#[test]
fn renaming_a_method_is_refused_with_a_reason() {
    let mut h = Harness::start("rename-method", &mini_library());
    let uri = h.open("Test.scd", "SinOsc.ar(440)");

    let id = h.send_request(
        "textDocument/rename",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 8 },
            "newName": "audioRate",
        }),
    );
    let response = h.await_response(id);
    let error = response.error.expect("a refusal, not a silent null");
    assert!(
        error.message.contains("dispatches at run time"),
        "{}",
        error.message
    );
}

#[test]
fn prepare_rename_refuses_before_the_box_opens() {
    let mut h = Harness::start("prepare-rename", &mini_library());
    let uri = h.open("Test.scd", "SinOsc.ar(440)");

    // On the method: refused, with the reason.
    let id = h.send_request(
        "textDocument/prepareRename",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 8 },
        }),
    );
    assert!(h.await_response(id).error.is_some());

    // On the class: allowed, and it offers the current name.
    let response: PrepareRenameResponse = h.request(
        "textDocument/prepareRename",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": 0, "character": 2 },
        }),
    );
    match response {
        PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } => {
            assert_eq!(placeholder, "SinOsc");
        }
        other => panic!("expected a placeholder: {other:?}"),
    }
}

/// The chain that expand-selection walks, and that anything evaluating a
/// block has to ask for. Counting parentheses is the alternative, and the
/// second case here is the one it gets wrong.
#[test]
fn selection_range_widens_to_the_enclosing_block() {
    let mut h = Harness::start("selrange", &mini_library());
    let source = "(\n\tSinOsc.ar(440);\n)\n";
    let uri = h.open("Block.scd", source);

    // On `440`, inside the argument list, inside the call, inside the block.
    let ranges: Vec<SelectionRange> = h.request(
        "textDocument/selectionRange",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "positions": [{ "line": 1, "character": 11 }],
        }),
    );

    let widths = chain(&ranges[0]);
    // Innermost is the literal itself.
    assert_eq!(
        widths[0],
        Range::new(Position::new(1, 11), Position::new(1, 14)),
        "innermost should be `440`: {widths:?}"
    );
    // Outermost is the whole file; the one below it is the `( ... )` block.
    let block = widths[widths.len() - 2];
    assert_eq!(
        block,
        Range::new(Position::new(0, 0), Position::new(2, 1)),
        "should widen to the parenthesised block: {widths:?}"
    );
}

/// A parenthesis inside a string is text, not structure. This is where brace
/// matching produces a block that stops in the wrong place.
#[test]
fn selection_range_ignores_parens_inside_strings_and_comments() {
    let mut h = Harness::start("selrange-str", &mini_library());
    let source = "(\n\t\"a ( b\".postln; // ) not structure\n\tSinOsc.ar(440);\n)\n";
    let uri = h.open("Tricky.scd", source);

    let ranges: Vec<SelectionRange> = h.request(
        "textDocument/selectionRange",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "positions": [{ "line": 2, "character": 11 }],
        }),
    );

    let widths = chain(&ranges[0]);
    let block = widths[widths.len() - 2];
    assert_eq!(
        block,
        Range::new(Position::new(0, 0), Position::new(3, 1)),
        "the block runs to the real `)`, not the one in the string: {widths:?}"
    );
}

/// Every step must select more than the one before it, or a keypress appears
/// to do nothing.
#[test]
fn selection_range_never_repeats_a_range() {
    let mut h = Harness::start("selrange-dedup", &mini_library());
    let uri = h.open("Nested.scd", "((((1))))\n");

    let ranges: Vec<SelectionRange> = h.request(
        "textDocument/selectionRange",
        serde_json::json!({
            "textDocument": { "uri": uri },
            "positions": [{ "line": 0, "character": 4 }],
        }),
    );

    let widths = chain(&ranges[0]);
    for pair in widths.windows(2) {
        assert_ne!(pair[0], pair[1], "duplicate step in {widths:?}");
    }
}

/// Innermost first, following the `parent` links outward.
fn chain(range: &SelectionRange) -> Vec<Range> {
    let mut out = vec![range.range];
    let mut node = range.parent.as_deref();
    while let Some(parent) = node {
        out.push(parent.range);
        node = parent.parent.as_deref();
    }
    out
}

/// A buffer that has never been saved has no path — the editor gives it a URI
/// like `untitled:Untitled-1`. Everything has to keep working on one, because
/// that is where SuperCollider tends to get written.
///
/// It is read as a script, not a class file. A class has to live in a `.sc`
/// file on disk before sclang will compile it at all, so a class definition in
/// an unsaved buffer could never be real.
#[test]
fn an_unsaved_buffer_is_a_document_like_any_other() {
    let mut h = Harness::start("untitled", &mini_library());

    let uri = Url::parse("untitled:Untitled-1").unwrap();
    h.send_notification(
        "textDocument/didOpen",
        serde_json::json!({
            "textDocument": {
                "uri": uri, "languageId": "supercollider", "version": 1,
                "text": "(\nvar freq = 440;\nSinOsc.ar(freq);\n)\n",
            }
        }),
    );

    // Diagnostics arrive for it, and a top-level block is valid script.
    let diagnostics = h.await_diagnostics(&uri);
    assert!(
        diagnostics.diagnostics.is_empty(),
        "unexpected errors: {:?}",
        diagnostics.diagnostics
    );

    // Completion works inside it.
    let response: Option<CompletionResponse> = h.request_at("textDocument/completion", &uri, 2, 9);
    let items = match response {
        Some(CompletionResponse::Array(items)) => items,
        Some(CompletionResponse::List(list)) => list.items,
        None => Vec::new(),
    };
    assert!(
        items.iter().any(|i| i.label == "ar"),
        "expected `ar` from an unsaved buffer"
    );

    // And a local declared in it resolves back to it, which is the part that
    // fails when a URI cannot be turned back from its synthetic path.
    let definition: Option<GotoDefinitionResponse> =
        h.request_at("textDocument/definition", &uri, 2, 12);
    match definition {
        Some(GotoDefinitionResponse::Scalar(location)) => assert_eq!(location.uri, uri),
        other => panic!("expected the unsaved buffer, got {other:?}"),
    }
}

/// An edit to a `.sc` file that has not been saved must still be visible to
/// the rest of the workspace, or a class being written is invisible until it
/// reaches disk.
#[test]
fn an_unsaved_edit_to_a_class_file_contributes_to_completion_elsewhere() {
    let mut h = Harness::start("untitled-index", &mini_library());

    let uri = h.open("Wobbler.sc", "Wobbler : Object {\n\t*wobble { ^1 }\n}\n");
    h.await_diagnostics(&uri);

    let other = h.open("Other.scd", "Wobb");
    let response: Option<CompletionResponse> =
        h.request_at("textDocument/completion", &other, 0, 4);
    let items = match response {
        Some(CompletionResponse::Array(items)) => items,
        Some(CompletionResponse::List(list)) => list.items,
        None => Vec::new(),
    };

    assert!(
        items.iter().any(|i| i.label == "Wobbler"),
        "a class in an unsaved edit should still be offered"
    );
}

/// `cmdlinecode` lets a script declare variables in a top-level `( … )` block,
/// which is how most `.scd` files are written. They have to parse, and they
/// have to be visible to completion in the block they belong to.
#[test]
fn variables_declared_in_a_top_level_block() {
    let mut h = Harness::start("cmdline-vars", &mini_library());
    let uri = h.open(
        "Block.scd",
        "(\nvar freq = 440;\nvar amp = 0.1;\nSinOsc.ar(fr);\n)\n",
    );

    // No complaint about the declarations themselves.
    let diagnostics = h.await_diagnostics(&uri);
    assert!(
        diagnostics.diagnostics.is_empty(),
        "`var` in a top-level block is valid SuperCollider: {:?}",
        diagnostics.diagnostics
    );

    // And they are in scope inside it.
    let response: Option<CompletionResponse> = h.request_at("textDocument/completion", &uri, 3, 12);
    let items = match response {
        Some(CompletionResponse::Array(items)) => items,
        Some(CompletionResponse::List(list)) => list.items,
        None => Vec::new(),
    };
    assert!(
        items.iter().any(|i| i.label == "freq"),
        "a local declared in the block should be offered: {:?}",
        items.iter().map(|i| &i.label).take(10).collect::<Vec<_>>()
    );
}

/// The same declarations with no block around them at all, which
/// `cmdlinecode` also allows.
#[test]
fn variables_declared_bare_at_the_top_of_a_script() {
    let mut h = Harness::start("cmdline-bare-vars", &mini_library());
    let uri = h.open("Bare.scd", "var freq = 440;\nSinOsc.ar(fr);\n");

    let diagnostics = h.await_diagnostics(&uri);
    assert!(
        diagnostics.diagnostics.is_empty(),
        "a bare `var` at the top of a script is valid: {:?}",
        diagnostics.diagnostics
    );

    let response: Option<CompletionResponse> = h.request_at("textDocument/completion", &uri, 1, 12);
    let items = match response {
        Some(CompletionResponse::Array(items)) => items,
        Some(CompletionResponse::List(list)) => list.items,
        None => Vec::new(),
    };
    assert!(items.iter().any(|i| i.label == "freq"), "got {items:?}");
}

/// `Routine { … }` is a call in a script and a class definition in a `.sc`
/// file. Nothing in the text says which, so the extension does — the same
/// split sclang makes between its two start symbols.
#[test]
fn a_block_after_a_class_name_depends_on_the_file() {
    let mut h = Harness::start("file-mode", &mini_library());

    // In a script it is a call, and parses.
    let script = h.open("Play.scd", "Routine { 1.rand }.play;\n");
    let in_script = h.await_diagnostics(&script);
    assert!(
        in_script.diagnostics.is_empty(),
        "`Routine {{ }}` is a call in a script: {:?}",
        in_script.diagnostics
    );

    // In a class file the same text opens a class body, and `1.rand` is not a
    // member — which is the correct complaint there.
    let class = h.open("Routine.sc", "Routine { 1.rand }\n");
    let in_class = h.await_diagnostics(&class);
    assert!(
        !in_class.diagnostics.is_empty(),
        "in a .sc file that is a class definition with a bad body"
    );
}
