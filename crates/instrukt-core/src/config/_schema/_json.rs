//! The configuration schema rendered as JSON Schema: the project layer's
//! editor schema, each layer's schema, and the text `instrukt config schema`
//! prints, all from the same schema [`check`](super::check) validates with.

use super::*;

/// The JSON Schema of the project layer for the developer's editor,
/// generated from the same schema [`check`] validates a project layer
/// with: a key only the global layer may hold is absent, and every map
/// admits exactly the keys validation admits.
pub fn editor_schema() -> serde_json::Value {
    let mut schema = json_node(&schema(), LayerKind::Project).unwrap_or_else(|| json!({}));
    if let Some(object) = schema.as_object_mut() {
        object.insert("required".to_owned(), json!(["schema_version"]));
        let mut head = serde_json::Map::new();
        head.insert(
            "$schema".to_owned(),
            json!("https://json-schema.org/draft/2020-12/schema"),
        );
        head.insert("title".to_owned(), json!("instrukt.yaml (project layer)"));
        head.extend(std::mem::take(object));
        *object = head;
    }
    schema["properties"]["modules"]["properties"]["history"]["default"] = json!(true);
    schema
}

/// JSON Schema of `layer` (`Global`, `Project`, or `Company`), generated from
/// the same schema tree [`check`] validates with.
pub fn layer_json_schema(layer: LayerKind) -> serde_json::Value {
    let mut schema = json_node(&schema(), layer).unwrap_or_else(|| json!({}));
    if let Some(object) = schema.as_object_mut() {
        let title = match layer {
            LayerKind::Global => "instrukt.yaml (global layer)",
            LayerKind::Project => "instrukt.yaml (project layer)",
            LayerKind::Company => "company.yaml (company home)",
        };
        let required = match layer {
            LayerKind::Company => json!(["schema_version", "company"]),
            _ => json!(["schema_version"]),
        };
        object.insert("required".to_owned(), required);
        let mut head = serde_json::Map::new();
        head.insert(
            "$schema".to_owned(),
            json!("https://json-schema.org/draft/2020-12/schema"),
        );
        head.insert("title".to_owned(), json!(title));
        head.extend(std::mem::take(object));
        *object = head;
    }
    schema
}

/// Describe what `layer` can declare at optional dotted `path`, as JSON Schema
/// (`as_json = true`) or readable field documentation (`as_json = false`).
pub fn describe_schema(
    layer: LayerKind,
    path: Option<&str>,
    as_json: bool,
) -> Result<String, super::super::_error::ConfigError> {
    let root = layer_json_schema(layer);
    let mut current = &root;
    let mut walked = Vec::new();
    if let Some(path) = path {
        let trimmed = path.trim();
        if trimmed.is_empty() {
            return Err(super::super::_error::ConfigError::InvalidValue {
                key: "schema".into(),
                message: "path must be nonempty".into(),
            });
        }
        for segment in trimmed.split('.') {
            if let Some(next) = current.get("properties").and_then(|p| p.get(segment)) {
                current = next;
                walked.push(segment.to_owned());
            } else if let Some(next) = current
                .get("additionalProperties")
                .filter(|v| v.is_object())
            {
                current = next;
                walked.push(segment.to_owned());
            } else {
                let mut available: Vec<&str> = current
                    .get("properties")
                    .and_then(|p| p.as_object())
                    .map(|map| map.keys().map(String::as_str).collect())
                    .unwrap_or_default();
                available.sort_unstable();
                let scope = if walked.is_empty() {
                    layer.described().to_owned()
                } else {
                    format!("`{}` in {}", walked.join("."), layer.described())
                };
                let keys = if available.is_empty() {
                    "none".to_owned()
                } else {
                    available.join(", ")
                };
                return Err(super::super::_error::ConfigError::InvalidValue {
                    key: trimmed.to_owned(),
                    message: format!(
                        "`{segment}` cannot be declared under {scope}; declarable keys: {keys}"
                    ),
                });
            }
        }
    }
    if as_json {
        return serde_json::to_string_pretty(current).map_err(|error| {
            super::super::_error::ConfigError::InvalidValue {
                key: path.unwrap_or("").into(),
                message: error.to_string(),
            }
        });
    }
    let prefix = walked.join(".");
    let mut lines = Vec::new();
    render_schema_text(&prefix, current, false, &mut lines);
    Ok(lines.join("\n"))
}

fn render_schema_text(
    prefix: &str,
    schema: &serde_json::Value,
    required: bool,
    out: &mut Vec<String>,
) {
    let type_desc = schema_type_summary(schema);
    let req = if required { "required" } else { "optional" };
    let default = schema
        .get("default")
        .map(|d| format!(", default {d}"))
        .unwrap_or_default();
    // A description of several lines, such as a connector kind's setup with
    // its manifest, continues indented under the key it describes.
    let mut desc_lines = schema
        .get("description")
        .and_then(|d| d.as_str())
        .map(str::lines)
        .into_iter()
        .flatten();
    let desc = desc_lines
        .next()
        .map(|d| format!(" — {d}"))
        .unwrap_or_default();
    if !prefix.is_empty() {
        out.push(format!("{prefix} ({type_desc}, {req}{default}){desc}"));
        out.extend(desc_lines.map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("    {line}")
            }
        }));
    }
    let req_list: Vec<&str> = schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        let mut keys: Vec<&String> = props.keys().collect();
        keys.sort();
        for key in keys {
            let child_path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            render_schema_text(
                &child_path,
                &props[key],
                req_list.contains(&key.as_str()),
                out,
            );
        }
    }
    if let Some(extra) = schema.get("additionalProperties").filter(|v| v.is_object()) {
        let child_path = if prefix.is_empty() {
            "<name>".to_owned()
        } else {
            format!("{prefix}.<name>")
        };
        render_schema_text(&child_path, extra, false, out);
    }
}

fn schema_type_summary(schema: &serde_json::Value) -> String {
    if let Some(values) = schema.get("enum").and_then(|e| e.as_array()) {
        let opts: Vec<String> = values
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| v.to_string())
            })
            .collect();
        return format!("one of: {}", opts.join(" | "));
    }
    if let Some(pattern) = schema.get("pattern").and_then(|p| p.as_str())
        && pattern.contains("\\$\\{")
    {
        return "${NAME} secret reference".to_owned();
    }
    if let Some(kind) = schema.get("type").and_then(|t| t.as_str()) {
        if kind == "array" {
            let item = schema
                .get("items")
                .map(schema_type_summary)
                .unwrap_or_else(|| "any".into());
            return format!("list of {item}");
        }
        return kind.to_owned();
    }
    if schema.get("anyOf").is_some() {
        return "union".to_owned();
    }
    "value".to_owned()
}

/// `node` as JSON Schema for `layer`; `None` when the layer may hold none
/// of it.
fn json_node(node: &Node, layer: LayerKind) -> Option<serde_json::Value> {
    match node {
        Node::Leaf(Rule::ShippedAdapterInProject, _) if layer == LayerKind::Project => {
            Some(json!({ "enum": SHIPPED_BLUEPRINT_ENGINES }))
        }
        Node::Leaf(rule, shape) => rule.admits(layer).then(|| json_shape(*shape, layer)),
        Node::Map { children, open } => {
            let properties: serde_json::Map<String, serde_json::Value> = children
                .iter()
                .filter_map(|(name, child)| Some(((*name).to_owned(), json_node(child, layer)?)))
                .collect();
            let open = open.as_deref().and_then(|open| json_node(open, layer));
            if properties.is_empty() && open.is_none() {
                return None;
            }
            Some(json!({
                "type": "object",
                "properties": properties,
                "additionalProperties": open.unwrap_or(json!(false)),
            }))
        }
    }
}

/// `schema` with `contract`, the key's contract the specification owning
/// it states, as its description.
fn described(mut schema: serde_json::Value, contract: &str) -> serde_json::Value {
    schema["description"] = json!(contract);
    schema
}

/// The JSON Schema of a key holding a `${NAME}` reference into `layer`'s
/// secrets file: `contract`, then which file the reference resolves from
/// and the `instrukt secrets set` command that writes its value.
pub(in crate::config) fn secret_reference(contract: &str, layer: LayerKind) -> serde_json::Value {
    let (file, command) = match layer {
        LayerKind::Company => (
            "the company home's secrets file",
            "instrukt secrets set <NAME> --company <name>",
        ),
        LayerKind::Global | LayerKind::Project => (
            "the machine's secrets file `~/.instrukt/secrets.env`",
            "instrukt secrets set <NAME>",
        ),
    };
    json!({
        "type": "string",
        "pattern": "^\\$\\{[A-Za-z_][A-Za-z0-9_]*\\}$",
        "description": format!(
            "{contract}; an exact ${{NAME}} reference into {file}, its value written with `{command}`"
        ),
    })
}

/// A JSON Schema `pattern` admitting exactly the strings
/// [`Field::NonEmptyStr`] accepts: those holding a character outside Rust's
/// `char::is_whitespace`, the set `str::trim` removes. A regex class escape
/// such as `\S` would not do, because JSON Schema reads patterns as ECMA-262,
/// whose whitespace differs (it adds U+FEFF and omits U+0085). The pattern is
/// a complemented class of `\uXXXX` escapes, which ECMA-262 reads as exactly
/// those code points; a validator using another regex dialect may not accept
/// the `\uXXXX` escape.
pub(in crate::config) fn non_blank_pattern() -> String {
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for code in (0..=u32::from(char::MAX))
        .filter(|&code| char::from_u32(code).is_some_and(char::is_whitespace))
    {
        match runs.last_mut() {
            Some((_, end)) if *end + 1 == code => *end = code,
            _ => runs.push((code, code)),
        }
    }
    let class: String = runs
        .iter()
        .map(|&(start, end)| {
            if start == end {
                format!("\\u{start:04X}")
            } else {
                format!("\\u{start:04X}-\\u{end:04X}")
            }
        })
        .collect();
    format!("[^{class}]")
}

fn json_shape(shape: Shape, layer: LayerKind) -> serde_json::Value {
    let strings = |names: &[&str]| json!({ "type": "array", "items": { "enum": names } });
    let record = |required: &[&str], optional: &[(&str, serde_json::Value)]| {
        let mut properties: serde_json::Map<String, serde_json::Value> = required
            .iter()
            .map(|name| ((*name).to_owned(), json!({ "type": "string" })))
            .collect();
        properties.extend(
            optional
                .iter()
                .map(|(name, schema)| ((*name).to_owned(), schema.clone())),
        );
        json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        })
    };
    match shape {
        Shape::Runtimes => {
            // Each action is an optional, non-blank override, as validation
            // reads it: an omitted one keeps the shipped instruction.
            let non_blank = non_blank_pattern();
            let actions: Vec<_> = super::super::_runtime::ACTIONS
                .iter()
                .map(|(name, contract)| {
                    let action = json!({
                        "type": "string",
                        "pattern": non_blank,
                        "description": contract,
                    });
                    (*name, action)
                })
                .collect();
            let mut entry = record(&[], &actions);
            entry["description"] = json!(
                "This machine's agent-action instructions for the named runtime; \
                 an action left out keeps the runtime's shipped instruction."
            );
            json!({ "type": "object", "additionalProperties": entry })
        }
        Shape::Webhooks => super::super::_webhooks::editor_schema(layer),
        Shape::Members => super::super::_company::members_schema(),
        Shape::Mcp => super::super::_mcp::mcp_json_schema(layer),
        Shape::McpClients => super::super::_mcp::mcp_clients_json_schema(),
        Shape::Issuer => super::super::_mcp::issuer_json_schema(),
        Shape::IngressUrl => json!({ "type": "string", "format": "uri" }),
        Shape::Connectors => super::super::_company::connectors_schema(),
        Shape::MachineId => json!({"type": "string", "pattern": "^[0-9a-f]{16,64}$"}),
        Shape::Port => json!({"type": "integer", "minimum": 1, "maximum": 65535}),
        Shape::Credential | Shape::LocalRedisUrl => json!({ "type": "string" }),
        Shape::RedisTimeout => json!({ "type": "integer", "minimum": 11 }),
        Shape::Version => json!({ "type": "integer", "minimum": 1 }),
        Shape::SchemaVersion => described(
            json_shape(Shape::Version, layer),
            "The schema version this document is written in; `instrukt config init` writes \
             the current one, an older document is converted and a newer one refused.",
        ),
        Shape::CompanyName => described(
            json_shape(Shape::Slug, layer),
            "The company's identity on this computer: the name of its home folder \
             `~/.instrukt/companies/<name>/`, following the grammar of a project slug.",
        ),
        Shape::Profiles => described(
            json_shape(Shape::StrList, layer),
            "The ordered profile names, declared in the machine's `agents.yaml`, that every \
             session in the company's home starts on in place of the machine-wide order; \
             an empty list is a finding, and without it the machine-wide order applies.",
        ),
        Shape::Slug | Shape::Domain | Shape::Str | Shape::Identity => json!({ "type": "string" }),
        Shape::Bool => json!({ "type": "boolean" }),
        Shape::OwningOrganisation => described(
            json!({ "type": "string" }),
            "The login of the GitHub organisation that owns the company home repositories \
             this computer adds; adding a company is refused without one \
             (project/instrukt/spec/feature/configuration/company-homes, Generation).",
        ),
        Shape::CompanyRepository => described(
            json!({ "type": "boolean", "default": false }),
            "On, declares the repository a company's: its documentation is the company's \
             knowledge, read and rendered as a company's \
             (project/instrukt/spec/feature/configuration/company-homes, The home). Off, it \
             is a project like any other.",
        ),
        Shape::Apps => json!({"type":"object", "additionalProperties":{"type":"object"}}),
        Shape::Board => json!({
            "anyOf": [{ "const": false }, { "type": "string", "pattern": "^[A-Za-z0-9-]+/[0-9]*[1-9][0-9]*$" }],
        }),
        Shape::OnOff => json!({ "enum": ["on", "off"] }),
        Shape::Approver => json!({ "enum": ["human", "peer"] }),

        Shape::Autostart => json!({
            "anyOf": [
                { "type": "boolean" },
                { "type": "object", "additionalProperties": { "type": "boolean" } },
            ],
        }),
        Shape::BranchPrefixes => {
            json!({ "type": "array", "items": { "type": "string", "minLength": 1, "pattern": "\\S" } })
        }
        Shape::StrList => json!({ "type": "array", "items": { "type": "string" } }),
        Shape::ModuleList => {
            let names: Vec<&str> = Module::ALL.iter().map(|module| module.key()).collect();
            strings(&names)
        }

        Shape::WatchList => json!({ "type": "array", "items": record(&["remote", "ref"], &[]) }),
        Shape::Regenerable => {
            json!({ "type": "array", "items": { "type": "string", "minLength": 1 } })
        }
        Shape::RetentionWindow => described(
            json!({
                "type": "string",
                "pattern": super::super::_thread_retention::WINDOW_PATTERN,
                "default": super::super::_thread_retention::OFF,
            }),
            super::super::_thread_retention::WINDOW_CONTRACT,
        ),
    }
}
