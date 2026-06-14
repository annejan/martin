//! `MARTIN_MCP=1` (or the `--mcp` arg): a **stdio MCP server** (JSON-RPC 2.0, newline-delimited) that
//! proxies tool calls to a running `MARTIN_SERVE` bridge. No Bevy here — stdout stays clean JSON-RPC
//! (logs go to stderr). Register it in `.mcp.json` so an MCP client (e.g. Claude Code) drives the live
//! engine with native tools: `camera` / `seek` / `pause` / `play` / `step` / `dump_camera` / `state`,
//! and `screenshot` which returns the PNG **inline** as image content.
//!
//! It connects to an already-running `MARTIN_SERVE` instance (start that separately, with your show).
//! Port: `MARTIN_MCP_PORT`, else `MARTIN_SERVE` if numeric, else 7878.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

use base64::Engine;
use serde_json::{Value, json};

/// The MCP protocol version this server speaks.
const PROTOCOL: &str = "2025-06-18";

/// If MCP mode is requested, run the stdio server (blocking) and return `true` so `main` exits before
/// touching Bevy. Otherwise `false` (normal run).
pub(crate) fn maybe_run() -> bool {
    let on = std::env::var_os("MARTIN_MCP").is_some() || std::env::args().any(|a| a == "--mcp");
    if !on {
        return false;
    }
    run();
    true
}

fn bridge_port() -> u16 {
    std::env::var("MARTIN_MCP_PORT")
        .or_else(|_| std::env::var("MARTIN_SERVE"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(7878)
}

/// Send one command line to the bridge and read its one-line JSON reply.
fn bridge(cmd: &Value) -> Result<Value, String> {
    let mut stream =
        TcpStream::connect(("127.0.0.1", bridge_port())).map_err(|e| format!("connect: {e}"))?;
    writeln!(stream, "{cmd}").map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    serde_json::from_str(&line).map_err(|e| format!("bad reply: {e}"))
}

fn run() {
    eprintln!("mcp: martin MCP server on stdio → bridge 127.0.0.1:{}", bridge_port());
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines().map_while(Result::ok) {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "initialize" => json!({
                "protocolVersion": PROTOCOL,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "martin", "version": env!("CARGO_PKG_VERSION")},
            }),
            "tools/list" => json!({"tools": tool_defs()}),
            "tools/call" => call_tool(req.get("params")),
            "ping" => json!({}),
            _ => {
                // a notification (no id) needs no reply; an unknown request gets an empty result.
                if id.is_none() {
                    continue;
                }
                json!({})
            }
        };
        // notifications carry no id → never reply.
        let Some(id) = id else { continue };
        let msg = json!({"jsonrpc": "2.0", "id": id, "result": result});
        if writeln!(out, "{msg}").is_err() {
            break;
        }
        let _ = out.flush();
    }
}

/// A `tools/call` → bridge command + MCP content. `screenshot` returns the PNG inline as image content.
fn call_tool(params: Option<&Value>) -> Value {
    let name = params
        .and_then(|p| p.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let args = params
        .and_then(|p| p.get("arguments"))
        .cloned()
        .unwrap_or(json!({}));

    if name == "screenshot" {
        return match screenshot(&args) {
            Ok(content) => json!({"content": content}),
            Err(e) => error_content(&e),
        };
    }

    // every other tool maps 1:1 to a bridge command of the same name, forwarding its arguments.
    let mut cmd = args;
    cmd["cmd"] = json!(name);
    match bridge(&cmd) {
        Ok(reply) => json!({"content": [{"type": "text", "text": reply.to_string()}]}),
        Err(e) => error_content(&format!("bridge: {e} (is MARTIN_SERVE running?)")),
    }
}

fn screenshot(args: &Value) -> Result<Value, String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("/tmp/martin_mcp_shot.png")
        .to_string();
    bridge(&json!({"cmd": "screenshot", "path": path}))?;
    // the PNG lands a frame or two after the reply (GPU readback) — give it a moment.
    std::thread::sleep(Duration::from_millis(600));
    let bytes = std::fs::read(&path).map_err(|e| format!("read {path}: {e}"))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(json!([
        {"type": "image", "data": b64, "mimeType": "image/png"},
        {"type": "text", "text": format!("screenshot {} ({} KiB)", path, bytes.len() / 1024)},
    ]))
}

fn error_content(msg: &str) -> Value {
    json!({"content": [{"type": "text", "text": msg}], "isError": true})
}

fn tool_defs() -> Value {
    let num = || json!({"type": "number"});
    json!([
        {
            "name": "camera",
            "description": "Nudge the orbit camera live (any field optional): dist, yaw, pitch (radians), pos [x,y,z] look-at.",
            "inputSchema": {"type": "object", "properties": {
                "dist": num(), "yaw": num(), "pitch": num(),
                "pos": {"type": "array", "items": num(), "minItems": 3, "maxItems": 3},
            }},
        },
        {
            "name": "seek",
            "description": "Set the show clock to time t (seconds).",
            "inputSchema": {"type": "object", "properties": {"t": num()}, "required": ["t"]},
        },
        {"name": "pause", "description": "Freeze the show clock.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "play", "description": "Resume the show clock.", "inputSchema": {"type": "object", "properties": {}}},
        {
            "name": "step",
            "description": "Advance the (paused) clock by dt seconds (default 0.1).",
            "inputSchema": {"type": "object", "properties": {"dt": num()}},
        },
        {
            "name": "screenshot",
            "description": "Render the current frame and return it inline as a PNG image.",
            "inputSchema": {"type": "object", "properties": {"path": {"type": "string"}}},
        },
        {"name": "dump_camera", "description": "Return a paste-ready [camera] line for the current pose + time.", "inputSchema": {"type": "object", "properties": {}}},
        {"name": "state", "description": "Return the current time, paused flag, and camera pose.", "inputSchema": {"type": "object", "properties": {}}},
    ])
}
