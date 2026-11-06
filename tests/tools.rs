//! The tools on a small package, and over the protocol.

use std::path::PathBuf;

use fhir_toolbox::Toolbox;
use fhir_toolbox::mcp::Server;
use octofhir_fhirschema::FhirVersion;
use rmcp::ServiceExt;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

fn package() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let folder = directory.path().join("package");
    std::fs::create_dir(&folder).unwrap();
    let write = |name: &str, value: Value| {
        std::fs::write(folder.join(name), value.to_string()).unwrap();
    };
    write(
        "package.json",
        json!({"name": "example.ig", "version": "1.0.0", "fhirVersions": ["4.0.1"]}),
    );
    write(
        "StructureDefinition-named-patient.json",
        json!({
            "resourceType": "StructureDefinition",
            "url": "http://example.org/StructureDefinition/named-patient",
            "name": "NamedPatient", "title": "Patient with a name", "status": "active",
            "fhirVersion": "4.0.1", "kind": "resource", "abstract": false, "type": "Patient",
            "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Patient",
            "derivation": "constraint",
            "differential": {"element": [
                {"id": "Patient", "path": "Patient"},
                {"id": "Patient.name", "path": "Patient.name", "min": 1, "mustSupport": true},
            ]},
        }),
    );
    write(
        "StructureDefinition-nickname.json",
        json!({
            "resourceType": "StructureDefinition",
            "url": "http://example.org/StructureDefinition/nickname",
            "name": "Nickname", "status": "active", "fhirVersion": "4.0.1",
            "kind": "complex-type", "abstract": false, "type": "Extension",
            "context": [{"type": "element", "expression": "Patient"}],
            "baseDefinition": "http://hl7.org/fhir/StructureDefinition/Extension",
            "derivation": "constraint",
            "differential": {"element": [
                {"id": "Extension", "path": "Extension"},
                {"id": "Extension.value[x]", "path": "Extension.value[x]",
                 "type": [{"code": "string"}]},
            ]},
        }),
    );
    (directory, folder)
}

async fn toolbox(folder: &std::path::Path) -> Toolbox {
    Toolbox::builder(FhirVersion::R4)
        .package(folder.to_str().unwrap())
        .build()
        .await
        .unwrap()
}

#[tokio::test]
async fn profiles_and_extensions_are_found_and_described() {
    let (_directory, folder) = package();
    let toolbox = toolbox(&folder).await;
    assert_eq!(toolbox.packages(), ["example.ig@1.0.0"]);

    let found = toolbox
        .call(
            "find_profiles",
            &json!({"query": "with name", "type": "Patient"}),
        )
        .await
        .unwrap();
    assert_eq!(found["total"], 1);
    assert_eq!(found["profiles"][0]["name"], "NamedPatient");
    assert_eq!(found["profiles"][0]["package"], "example.ig@1.0.0");
    let none = toolbox
        .call("find_profiles", &json!({"type": "Observation"}))
        .await
        .unwrap();
    assert_eq!(none["total"], 0);

    let extensions = toolbox
        .call("find_extensions", &json!({"context": "Patient"}))
        .await
        .unwrap();
    assert_eq!(extensions["extensions"][0]["name"], "Nickname");
    assert_eq!(extensions["extensions"][0]["context"], json!(["Patient"]));

    let described = toolbox
        .call("describe_profile", &json!({"profile": "NamedPatient"}))
        .await
        .unwrap();
    assert_eq!(described["elements"][1]["path"], "Patient.name");
    assert_eq!(described["elements"][1]["min"], 1);
    assert!(
        toolbox
            .call("describe_profile", &json!({"profile": "Unknown"}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_resource_is_validated_against_fhir_and_a_profile() {
    let (_directory, folder) = package();
    let toolbox = toolbox(&folder).await;
    let profile = "http://example.org/StructureDefinition/named-patient";

    let plain = json!({"resourceType": "Patient", "gender": "female"});
    let core = toolbox
        .call("validate_resource", &json!({"resource": plain}))
        .await
        .unwrap();
    assert_eq!(core["valid"], true, "{core}");

    let without_name = toolbox
        .call(
            "validate_resource",
            &json!({"resource": plain, "profile": profile}),
        )
        .await
        .unwrap();
    assert_eq!(without_name["valid"], false, "{without_name}");

    let named = json!({"resourceType": "Patient", "name": [{"family": "Doe"}]});
    let with_name = toolbox
        .call(
            "validate_resource",
            &json!({"resource": named, "profile": profile}),
        )
        .await
        .unwrap();
    assert_eq!(with_name["valid"], true, "{with_name}");

    let broken = json!({"resourceType": "Patient", "nonsense": true});
    let invalid = toolbox
        .call("validate_resource", &json!({"resource": broken}))
        .await
        .unwrap();
    assert_eq!(invalid["valid"], false, "{invalid}");
}

#[tokio::test]
async fn fhirpath_and_views_run_on_resources() {
    let (_directory, folder) = package();
    let toolbox = toolbox(&folder).await;
    let patient = json!({"resourceType": "Patient", "id": "p",
        "name": [{"family": "Doe", "given": ["Ann"]}]});

    let evaluated = toolbox
        .call(
            "evaluate_fhirpath",
            &json!({"expression": "name.given.first()", "resource": patient}),
        )
        .await
        .unwrap();
    assert_eq!(evaluated["result"], json!(["Ann"]));
    assert!(
        toolbox
            .call("evaluate_fhirpath", &json!({"expression": "1 +"}))
            .await
            .is_err()
    );

    let view = json!({"resourceType": "ViewDefinition", "status": "active", "resource": "Patient",
        "select": [{"column": [
            {"name": "id", "path": "id"},
            {"name": "family", "path": "name.family.first()"}]}]});
    let rows = toolbox
        .call("run_view", &json!({"view": view, "resources": [patient]}))
        .await
        .unwrap();
    assert_eq!(rows["rows"], json!([{"id": "p", "family": "Doe"}]));
}

#[tokio::test]
async fn the_protocol_lists_and_calls_tools() {
    let (_directory, folder) = package();
    let server = Server::new(toolbox(&folder).await);
    let (client, transport) = tokio::io::duplex(1 << 16);
    tokio::spawn(async move {
        let running = server.serve(transport).await.unwrap();
        running.waiting().await.unwrap();
    });
    let (read, mut write) = tokio::io::split(client);
    let mut lines = BufReader::new(read).lines();
    let mut ask = async |message: Value| {
        write
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
        if message.get("id").is_none() {
            return Value::Null;
        }
        let line = lines.next_line().await.unwrap().unwrap();
        serde_json::from_str::<Value>(&line).unwrap()
    };

    let hello = ask(
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
        "protocolVersion": "2025-06-18", "capabilities": {},
        "clientInfo": {"name": "test", "version": "1"}}}),
    )
    .await;
    assert_eq!(hello["result"]["serverInfo"]["name"], "fhir-toolbox");
    ask(json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;

    let listed = ask(json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).await;
    let tools = listed["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 6, "{listed}");
    assert_eq!(tools[0]["annotations"]["readOnlyHint"], true);

    let call = |id: i64, name: &str, arguments: Value| {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": name, "arguments": arguments}})
    };
    let done = ask(call(3, "evaluate_fhirpath", json!({"expression": "1 + 1"}))).await;
    assert_eq!(done["result"]["structuredContent"]["result"], json!([2]));
    let failed = ask(call(4, "evaluate_fhirpath", json!({"expression": "1 +"}))).await;
    assert_eq!(failed["result"]["isError"], true);
    let unknown = ask(call(5, "nothing", json!({}))).await;
    assert_eq!(unknown["error"]["code"], -32602);
}
