//! OpenAPI importer: the constructs a real spec exercises — path-segment tree, request naming,
//! `{{base_url}}` + `{var}`→`:var` URLs, parameter example synthesis, JSON body synthesis from a
//! `$ref`'d schema, servers→environment, security→auth, responses→examples.

use serde_json::Value;

const SPEC: &str = r##"{
  "openapi": "3.0.0",
  "info": { "title": "Petstore", "description": "A sample API" },
  "servers": [{ "url": "https://api.example.com/v1/" }],
  "components": {
    "securitySchemes": { "bearerAuth": { "type": "http", "scheme": "bearer" } },
    "schemas": { "Pet": { "type": "object", "properties": { "id": {"type":"integer"}, "name": {"type":"string"} } } }
  },
  "security": [{ "bearerAuth": [] }],
  "paths": {
    "/pets/{petId}": {
      "get": {
        "summary": "Get a pet",
        "operationId": "getPet",
        "parameters": [
          { "name": "petId", "in": "path", "required": true, "schema": {"type":"integer"} },
          { "name": "verbose", "in": "query", "schema": {"type":"boolean"} }
        ],
        "responses": { "200": { "description": "ok", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Pet" } } } } }
      },
      "post": {
        "operationId": "createPet",
        "requestBody": { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Pet" } } } },
        "responses": { "201": { "description": "created" } }
      }
    }
  }
}"##;

fn parse() -> Value {
    let m = cross_q::parse_to_mapped_items("openapi", SPEC, "spec.json");
    assert_eq!(m["ok"], Value::Bool(true), "parse should succeed: {m:?}");
    m["mapped"].clone()
}

fn by_name<'a>(arr: &'a Value, name: &str) -> &'a Value {
    arr.as_array()
        .unwrap()
        .iter()
        .find(|x| x["name"] == Value::String(name.to_string()))
        .unwrap_or_else(|| panic!("no record named {name}"))
}

#[test]
fn builds_the_path_segment_tree_under_a_titled_root() {
    let m = parse();
    let cols = &m["collections"];
    let root = by_name(cols, "Petstore");
    assert_eq!(
        root["parentId"],
        Value::Null,
        "root collection is top-level"
    );
    assert_eq!(
        root["data"]["variables"]["base_url"]["syncValue"],
        "https://api.example.com/v1"
    );
    // pets → {petId}, nested by URL segment.
    let pets = by_name(cols, "pets");
    assert_eq!(pets["parentId"], root["tempId"]);
    let pet_id = by_name(cols, "{petId}");
    assert_eq!(pet_id["parentId"], pets["tempId"]);
}

#[test]
fn requests_get_names_urls_methods_and_synthesized_params() {
    let m = parse();
    let get = by_name(&m["requests"], "Get a pet"); // summary wins over operationId
    let r = &get["data"]["request"];
    assert_eq!(r["method"], "GET");
    assert_eq!(r["url"], "{{base_url}}/pets/:petId"); // base_url prefix + {petId}→:petId
    assert_eq!(r["pathVariables"][0]["key"], "petId");
    assert_eq!(r["pathVariables"][0]["value"], "0"); // integer → 0
    assert_eq!(r["queryParams"][0]["key"], "verbose");
    assert_eq!(r["queryParams"][0]["value"], "false"); // boolean → false
    assert_eq!(get["data"]["auth"]["type"], "bearer_token"); // spec-level security applied
}

#[test]
fn json_body_is_synthesized_from_a_reffed_schema() {
    let m = parse();
    let post = by_name(&m["requests"], "createPet"); // operationId (no summary)
    let body = &post["data"]["request"]["body"];
    assert_eq!(body["contentType"], "json");
    // integer → 0, string → "string", pretty-printed.
    assert_eq!(body["raw"], "{\n  \"id\": 0,\n  \"name\": \"string\"\n}");
}

#[test]
fn servers_become_an_environment_and_security_becomes_auth() {
    let m = parse();
    let env = by_name(&m["environments"], "Petstore");
    assert_eq!(
        env["variables"]["base_url"]["syncValue"],
        "https://api.example.com/v1"
    ); // trailing / stripped
    let root = by_name(&m["collections"], "Petstore");
    assert_eq!(root["data"]["auth"]["type"], "bearer_token");
}

#[test]
fn numeric_responses_become_examples() {
    let m = parse();
    let names: Vec<&str> = m["examples"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"ok"),
        "200 description → example name: {names:?}"
    );
    assert!(names.contains(&"created"));
}

#[test]
fn accepts_yaml_input() {
    let yaml = "openapi: 3.0.0\ninfo:\n  title: Y\nservers:\n  - url: https://y.example.com\npaths:\n  /ping:\n    get:\n      responses:\n        '200':\n          description: pong\n";
    let m = cross_q::parse_to_mapped_items("openapi", yaml, "spec.yaml");
    assert_eq!(m["ok"], Value::Bool(true), "YAML should parse: {m:?}");
    let get = by_name(&m["mapped"]["requests"], "GET /ping"); // no summary/operationId → METHOD path
    assert_eq!(get["data"]["request"]["url"], "{{base_url}}/ping");
}
