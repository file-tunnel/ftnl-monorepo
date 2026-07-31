use std::io::{self, BufRead, Write};

use serde_json::{Map, Value, json};

const SERVER_NAME: &str = "ftnl-mcp-server";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const LEGACY_PROTOCOL_VERSION: &str = "2025-11-25";
const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";
const MAX_INPUT_LINE_BYTES: usize = 1024 * 1024;
const MAX_OBJECT_KEY_BYTES: usize = 1024;
const MAX_TRANSFER_BYTES: u64 = 5 * 1024 * 1024 * 1024 * 1024;
const MIN_CHUNK_BYTES: u64 = 64 * 1024;
const DEFAULT_CHUNK_BYTES: u64 = 8 * 1024 * 1024;
const MAX_CHUNK_BYTES: u64 = 64 * 1024 * 1024;

fn main() {
    if let Err(error) = run() {
        eprintln!("{SERVER_NAME}: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = stdin.lock();
    let mut writer = stdout.lock();

    while let Some(line) = read_bounded_line(&mut reader)? {
        if line.is_empty() {
            continue;
        }

        let response = match std::str::from_utf8(&line) {
            Ok(input) => handle_message(input),
            Err(_) => Some(error_response(Value::Null, -32700, "Parse error")),
        };

        if let Some(response) = response {
            serde_json::to_writer(&mut writer, &response)
                .map_err(|error| io::Error::other(error.to_string()))?;
            writer.write_all(b"\n")?;
            writer.flush()?;
        }
    }

    Ok(())
}

fn read_bounded_line<R: BufRead>(reader: &mut R) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(available.len(), |position| position + 1);
        if line.len().saturating_add(take) > MAX_INPUT_LINE_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("input line exceeds {MAX_INPUT_LINE_BYTES} bytes"),
            ));
        }

        line.extend_from_slice(&available[..take]);
        reader.consume(take);

        if newline.is_some() {
            while line
                .last()
                .is_some_and(|byte| matches!(*byte, b'\n' | b'\r'))
            {
                line.pop();
            }
            return Ok(Some(line));
        }
    }
}

fn handle_message(input: &str) -> Option<Value> {
    let request = match serde_json::from_str::<Value>(input) {
        Ok(value) => value,
        Err(_) => return Some(error_response(Value::Null, -32700, "Parse error")),
    };

    handle_request(request)
}

fn handle_request(request: Value) -> Option<Value> {
    let Some(object) = request.as_object() else {
        return Some(error_response(Value::Null, -32600, "Invalid Request"));
    };

    let has_id = object.contains_key("id");
    let id = object.get("id").cloned().unwrap_or(Value::Null);
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Some(error_response(id, -32600, "Invalid Request"));
    }

    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return Some(error_response(id, -32600, "Invalid Request"));
    };
    let params = object
        .get("params")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    if !has_id {
        return None;
    }

    let modern = request_protocol_version(&params) == Some(MODERN_PROTOCOL_VERSION);
    if let Some(version) = request_protocol_version(&params) {
        if version != MODERN_PROTOCOL_VERSION && version != LEGACY_PROTOCOL_VERSION {
            return Some(error_response(id, -32022, "Unsupported protocol version"));
        }
    }

    let result = match method {
        "server/discover" => discover_result(),
        "initialize" => initialize_result(&params),
        "ping" if !modern => json!({}),
        "tools/list" => list_tools_result(modern),
        "tools/call" => match call_tool(&params, modern) {
            Ok(value) => value,
            Err(error) => return Some(error_response(id, -32602, error)),
        },
        _ => return Some(error_response(id, -32601, "Method not found")),
    };

    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn request_protocol_version(params: &Map<String, Value>) -> Option<&str> {
    params
        .get("_meta")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("io.modelcontextprotocol/protocolVersion"))
        .and_then(Value::as_str)
}

fn discover_result() -> Value {
    json!({
        "resultType": "complete",
        "supportedVersions": [MODERN_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION],
        "capabilities": {"tools": {}},
        "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
        "instructions": "Read-only File Tunnel capability inspection, bounded transfer planning, and object-key validation.",
        "ttlMs": 0,
        "cacheScope": "private"
    })
}

fn initialize_result(params: &Map<String, Value>) -> Value {
    let requested = params
        .get("protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or(LEGACY_PROTOCOL_VERSION);
    let negotiated = if requested == LEGACY_PROTOCOL_VERSION {
        requested
    } else {
        LEGACY_PROTOCOL_VERSION
    };

    json!({
        "protocolVersion": negotiated,
        "capabilities": {"tools": {}},
        "serverInfo": {"name": SERVER_NAME, "version": SERVER_VERSION},
        "instructions": "Read-only File Tunnel capability inspection, bounded transfer planning, and object-key validation."
    })
}

fn list_tools_result(modern: bool) -> Value {
    let mut result = json!({
        "tools": [
            {
                "name": "file_tunnel_capabilities",
                "description": "Report the server's read-only capability and safety boundary.",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }
            },
            {
                "name": "file_tunnel_plan_transfer",
                "description": "Compute a bounded chunk plan without reading files, opening sockets, or starting a transfer.",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {
                        "size_bytes": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": MAX_TRANSFER_BYTES
                        },
                        "chunk_size_bytes": {
                            "type": "integer",
                            "minimum": MIN_CHUNK_BYTES,
                            "maximum": MAX_CHUNK_BYTES,
                            "default": DEFAULT_CHUNK_BYTES
                        }
                    },
                    "required": ["size_bytes"],
                    "additionalProperties": false
                }
            },
            {
                "name": "file_tunnel_validate_object_key",
                "description": "Validate a relative File Tunnel object key and reject traversal or ambiguous path syntax.",
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "properties": {
                        "key": {
                            "type": "string",
                            "minLength": 1,
                            "maxLength": MAX_OBJECT_KEY_BYTES
                        }
                    },
                    "required": ["key"],
                    "additionalProperties": false
                }
            }
        ]
    });

    if modern {
        let object = result
            .as_object_mut()
            .expect("list result is constructed as an object");
        object.insert("resultType".into(), json!("complete"));
        object.insert("ttlMs".into(), json!(0));
        object.insert("cacheScope".into(), json!("private"));
    }

    result
}

fn call_tool(params: &Map<String, Value>, modern: bool) -> Result<Value, String> {
    ensure_only_keys(params, &["name", "arguments", "_meta"])?;
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| "name must be a string".to_owned())?;
    let arguments = params
        .get("arguments")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let value = match name {
        "file_tunnel_capabilities" => {
            ensure_only_keys(&arguments, &[])?;
            json!({
                "server": SERVER_NAME,
                "version": SERVER_VERSION,
                "protocol_versions": [MODERN_PROTOCOL_VERSION, LEGACY_PROTOCOL_VERSION],
                "read_only": true,
                "network_access": false,
                "filesystem_access": false,
                "credential_inputs": false,
                "mutation_tools": false,
                "max_input_line_bytes": MAX_INPUT_LINE_BYTES,
                "max_transfer_bytes": MAX_TRANSFER_BYTES,
                "tools": [
                    "file_tunnel_capabilities",
                    "file_tunnel_plan_transfer",
                    "file_tunnel_validate_object_key"
                ]
            })
        }
        "file_tunnel_plan_transfer" => plan_transfer(&arguments)?,
        "file_tunnel_validate_object_key" => validate_object_key_tool(&arguments)?,
        _ => return Err(format!("unknown tool: {name}")),
    };

    Ok(tool_result(value, modern))
}

fn plan_transfer(arguments: &Map<String, Value>) -> Result<Value, String> {
    ensure_only_keys(arguments, &["size_bytes", "chunk_size_bytes"])?;
    let size = required_u64(arguments, "size_bytes")?;
    if size == 0 || size > MAX_TRANSFER_BYTES {
        return Err(format!(
            "size_bytes must be between 1 and {MAX_TRANSFER_BYTES}"
        ));
    }

    let chunk_size = optional_u64(arguments, "chunk_size_bytes")?.unwrap_or(DEFAULT_CHUNK_BYTES);
    if !(MIN_CHUNK_BYTES..=MAX_CHUNK_BYTES).contains(&chunk_size) {
        return Err(format!(
            "chunk_size_bytes must be between {MIN_CHUNK_BYTES} and {MAX_CHUNK_BYTES}"
        ));
    }

    let chunk_count = size.div_ceil(chunk_size);
    let last_chunk_bytes = size - chunk_size.saturating_mul(chunk_count.saturating_sub(1));

    Ok(json!({
        "size_bytes": size,
        "chunk_size_bytes": chunk_size,
        "chunk_count": chunk_count,
        "last_chunk_bytes": last_chunk_bytes,
        "mutates_state": false,
        "starts_transfer": false
    }))
}

fn validate_object_key_tool(arguments: &Map<String, Value>) -> Result<Value, String> {
    ensure_only_keys(arguments, &["key"])?;
    let key = arguments
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| "key must be a string".to_owned())?;
    validate_object_key(key)?;

    Ok(json!({
        "key": key,
        "valid": true,
        "segments": key.split('/').count()
    }))
}

fn validate_object_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("key must not be empty".into());
    }
    if key.len() > MAX_OBJECT_KEY_BYTES {
        return Err(format!("key exceeds {MAX_OBJECT_KEY_BYTES} bytes"));
    }
    if key.trim() != key {
        return Err("key must not have leading or trailing whitespace".into());
    }
    if key.starts_with('/') || key.contains('\\') {
        return Err("key must be relative and use forward slashes".into());
    }
    if key.chars().any(char::is_control) {
        return Err("key must not contain control characters".into());
    }

    for (index, segment) in key.split('/').enumerate() {
        if segment.is_empty() {
            return Err("key must not contain empty path segments".into());
        }
        if matches!(segment, "." | "..") {
            return Err("key must not contain traversal segments".into());
        }
        if index == 0 && segment.ends_with(':') {
            return Err("key must not contain a drive-prefix segment".into());
        }
    }

    Ok(())
}

fn ensure_only_keys(arguments: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    if let Some(unexpected) = arguments
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
    {
        return Err(format!("unexpected argument: {unexpected}"));
    }
    Ok(())
}

fn required_u64(arguments: &Map<String, Value>, key: &str) -> Result<u64, String> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{key} must be a non-negative integer"))
}

fn optional_u64(arguments: &Map<String, Value>, key: &str) -> Result<Option<u64>, String> {
    arguments
        .get(key)
        .map(|value| {
            value
                .as_u64()
                .ok_or_else(|| format!("{key} must be a non-negative integer"))
        })
        .transpose()
}

fn tool_result(value: Value, modern: bool) -> Value {
    let text = serde_json::to_string_pretty(&value)
        .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".into());
    let mut result = json!({
        "content": [{"type": "text", "text": text}],
        "structuredContent": value,
        "isError": false
    });
    if modern {
        result
            .as_object_mut()
            .expect("tool result is constructed as an object")
            .insert("resultType".into(), json!("complete"));
    }
    result
}

fn error_response(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {"code": code, "message": message.into()}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, params: Value) -> Value {
        handle_request(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        }))
        .expect("request with an id must return a response")
    }

    #[test]
    fn validates_safe_relative_object_keys() {
        assert!(validate_object_key("accounts/alice/report.bin").is_ok());
        for invalid in [
            "",
            "/absolute",
            "../escape",
            "a/../escape",
            "a//b",
            "a\\b",
            " C:/drive",
            "C:/drive",
            "trailing ",
        ] {
            assert!(
                validate_object_key(invalid).is_err(),
                "expected {invalid:?} to be rejected"
            );
        }
    }

    #[test]
    fn computes_bounded_chunk_plan() {
        let response = request(
            "tools/call",
            json!({
                "name": "file_tunnel_plan_transfer",
                "arguments": {"size_bytes": 10_000_000, "chunk_size_bytes": 1_000_000}
            }),
        );
        assert_eq!(response["result"]["structuredContent"]["chunk_count"], 10);
        assert_eq!(
            response["result"]["structuredContent"]["last_chunk_bytes"],
            1_000_000
        );
        assert_eq!(
            response["result"]["structuredContent"]["starts_transfer"],
            false
        );
    }

    #[test]
    fn rejects_unknown_arguments() {
        let response = request(
            "tools/call",
            json!({
                "name": "file_tunnel_plan_transfer",
                "arguments": {"size_bytes": 1, "secret": "do-not-accept"}
            }),
        );
        assert_eq!(response["error"]["code"], -32602);
    }

    #[test]
    fn advertises_legacy_and_modern_protocol_eras() {
        let response = request(
            "server/discover",
            json!({
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION,
                    "io.modelcontextprotocol/clientCapabilities": {}
                }
            }),
        );
        assert_eq!(response["result"]["resultType"], "complete");
        assert_eq!(
            response["result"]["supportedVersions"][0],
            MODERN_PROTOCOL_VERSION
        );
        assert_eq!(response["result"]["ttlMs"], 0);
    }

    #[test]
    fn modern_tool_list_is_private_and_immediately_stale() {
        let response = request(
            "tools/list",
            json!({
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION
                }
            }),
        );
        assert_eq!(response["result"]["cacheScope"], "private");
        assert_eq!(response["result"]["ttlMs"], 0);
        assert_eq!(
            response["result"]["tools"].as_array().map(Vec::len),
            Some(3)
        );
    }

    #[test]
    fn notifications_do_not_emit_responses() {
        assert!(
            handle_request(json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }))
            .is_none()
        );
    }

    #[test]
    fn bounded_reader_rejects_oversized_lines() {
        let input = vec![b'x'; MAX_INPUT_LINE_BYTES + 1];
        let error = read_bounded_line(&mut io::Cursor::new(input))
            .expect_err("oversized line must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
