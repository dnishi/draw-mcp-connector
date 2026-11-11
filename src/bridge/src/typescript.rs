//! Converts the JSON Schema generated from the types in protocol.rs into TypeScript type definitions.
//! Supports only the shapes protocol.rs uses (object / oneOf / const / enum / array / $ref and primitive types).

use std::collections::BTreeMap;
use std::fmt::Write;

use rmcp::schemars::Schema;
use serde_json::{Map, Value};

const HEADER: &str = "\
// This file is generated from src/bridge/src/protocol.rs. Do not edit it directly.
// To regenerate it, run `UPDATE_PROTOCOL_TS=1 cargo test protocol` in src/bridge.
";

/// Collects each root type and the types it references ($defs), and outputs them sorted by name
pub fn generate(roots: &[Schema]) -> String {
    let mut definitions = BTreeMap::new();
    for root in roots {
        let mut root = root.as_object().cloned().unwrap_or_default();
        if let Some(Value::Object(defs)) = root.remove("$defs") {
            definitions.extend(defs);
        }
        root.remove("$schema");
        let name = root
            .remove("title")
            .and_then(|title| title.as_str().map(str::to_owned))
            .expect("ルートの型には title(型の名前)がある");
        definitions.insert(name, Value::Object(root));
    }

    let mut output = String::from(HEADER);
    for (name, schema) in &definitions {
        output.push('\n');
        write_definition(&mut output, name, schema);
    }
    output
}

fn object(schema: &Value) -> &Map<String, Value> {
    schema.as_object().expect("スキーマは JSON オブジェクト")
}

fn description(schema: &Value) -> Option<&str> {
    schema.get("description").and_then(Value::as_str)
}

fn write_doc(output: &mut String, indent: &str, schema: &Value) {
    if let Some(text) = description(schema) {
        writeln!(output, "{indent}/** {text} */").unwrap();
    }
}

fn write_definition(output: &mut String, name: &str, schema: &Value) {
    write_doc(output, "", schema);
    if let Some(variants) = union_variants(schema) {
        writeln!(output, "export type {name} =").unwrap();
        for variant in variants {
            write_doc(output, "  ", variant);
            writeln!(output, "  | {}", type_expression(variant)).unwrap();
        }
        output.insert(output.len() - 1, ';');
    } else if is_object(schema) {
        writeln!(output, "export interface {name} {{").unwrap();
        for (property, property_schema, required) in properties(schema) {
            write_doc(output, "  ", property_schema);
            let optional = if required { "" } else { "?" };
            let ty = type_expression(property_schema);
            writeln!(output, "  {property}{optional}: {ty};").unwrap();
        }
        output.push_str("}\n");
    } else {
        writeln!(output, "export type {name} = {};", type_expression(schema)).unwrap();
    }
}

fn union_variants(schema: &Value) -> Option<&Vec<Value>> {
    let schema = object(schema);
    schema
        .get("oneOf")
        .or_else(|| schema.get("anyOf"))
        .and_then(Value::as_array)
}

fn is_object(schema: &Value) -> bool {
    schema.get("type").and_then(Value::as_str) == Some("object")
}

/// Orders properties as listed in required (= Rust definition order), then the rest by name
fn properties(schema: &Value) -> Vec<(&str, &Value, bool)> {
    let Some(all) = schema.get("properties").and_then(Value::as_object) else {
        return Vec::new();
    };
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();

    let required_first = required
        .iter()
        .filter_map(|name| all.get_key_value(*name))
        .map(|(name, schema)| (name.as_str(), schema, true));
    let others = all
        .iter()
        .filter(|(name, _)| !required.contains(&name.as_str()))
        .map(|(name, schema)| (name.as_str(), schema, false));
    required_first.chain(others).collect()
}

fn string_literal(value: &Value) -> String {
    format!(
        "'{}'",
        value.as_str().expect("const / enum は文字列だけに対応")
    )
}

fn type_expression(schema: &Value) -> String {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        return reference
            .strip_prefix("#/$defs/")
            .expect("参照は $defs の中だけ")
            .to_owned();
    }
    if let Some(variants) = union_variants(schema) {
        let types: Vec<_> = variants.iter().map(type_expression).collect();
        return types.join(" | ");
    }
    if let Some(value) = schema.get("const") {
        return string_literal(value);
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        let literals: Vec<_> = values.iter().map(string_literal).collect();
        return literals.join(" | ");
    }

    match schema.get("type") {
        Some(Value::Array(types)) => {
            let types: Vec<_> = types.iter().map(primitive_type).collect();
            types.join(" | ")
        }
        Some(Value::String(ty)) if ty == "array" => {
            let item = type_expression(&schema["items"]);
            if item.contains(' ') {
                format!("({item})[]")
            } else {
                format!("{item}[]")
            }
        }
        Some(Value::String(ty)) if ty == "object" => {
            let fields: Vec<_> = properties(schema)
                .into_iter()
                .map(|(name, property, required)| {
                    let optional = if required { "" } else { "?" };
                    format!("{name}{optional}: {}", type_expression(property))
                })
                .collect();
            format!("{{ {} }}", fields.join("; "))
        }
        Some(ty) => primitive_type(ty),
        None => "unknown".to_owned(),
    }
}

fn primitive_type(ty: &Value) -> String {
    match ty.as_str() {
        Some("string") => "string",
        Some("number" | "integer") => "number",
        Some("boolean") => "boolean",
        Some("null") => "null",
        _ => panic!("対応していない型: {ty}"),
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::schemars::json_schema;

    #[test]
    fn converts_tagged_union_and_interface() {
        let root = json_schema!({
            "title": "Message",
            "oneOf": [
                {
                    "description": "挨拶",
                    "type": "object",
                    "properties": {
                        "type": { "type": "string", "const": "hello" },
                        "names": { "type": "array", "items": { "$ref": "#/$defs/Name" } },
                        "note": { "type": ["string", "null"] }
                    },
                    "required": ["type", "names"]
                }
            ],
            "$defs": {
                "Name": {
                    "type": "object",
                    "properties": { "value": { "type": "string", "description": "名前" } },
                    "required": ["value"]
                }
            }
        });

        assert_eq!(
            generate(&[root]),
            format!(
                "{HEADER}
export type Message =
  /** 挨拶 */
  | {{ type: 'hello'; names: Name[]; note?: string | null }};

export interface Name {{
  /** 名前 */
  value: string;
}}
"
            )
        );
    }
}
