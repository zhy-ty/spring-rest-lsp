//! rest-nav-lsp — 极简 LSP：把 Spring REST 接口作为 workspace 符号暴露给 Zed。
//! 只声明 workspaceSymbolProvider，不碰补全/定义/诊断（那些仍走主 LSP，如 jdtls）。
//! Zed 的项目符号搜索(Ctrl-T)会向所有声明该能力的 LSP 发 workspace/symbol 并合并结果。
//!
//! 运行: 无参数 = LSP stdio 服务；--scan <root> [--json] = 命令行_dump 接口（调试用）。

mod scanner;

use scanner::{filter_endpoints, scan_project, Endpoint};
use serde_json::{json, Value};
use std::io::{self, BufRead, Read, Write};
use std::path::PathBuf;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use url::Url;

const RESCAN_INTERVAL_MS: u128 = 4000;
const MAX_RESULTS: usize = 2000;

struct State {
    root: PathBuf,
    endpoints: Vec<Endpoint>,
    last_scan: Option<Instant>,
}

fn log(msg: &str) {
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let path = std::env::temp_dir().join("rest-nav-lsp.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "[{}] {}", ts, msg);
    }
}

fn uri_to_path(uri: &str) -> Option<PathBuf> {
    Url::parse(uri).ok()?.to_file_path().ok()
}

fn path_to_uri(p: &str) -> String {
    Url::from_file_path(p).map(|u| u.to_string()).unwrap_or_else(|_| format!("file://{}", p))
}

fn rescan(state: &mut State, reason: &str) {
    let t0 = Instant::now();
    let (files, eps) = scan_project(&state.root);
    log(&format!("scan({}) root={} files={} endpoints={} in {}ms",
        reason, state.root.display(), files, eps.len(), t0.elapsed().as_millis()));
    state.endpoints = eps;
    state.last_scan = Some(Instant::now());
}

fn ensure_fresh(state: &mut State) {
    let stale = match state.last_scan { None => true, Some(t) => t.elapsed().as_millis() > RESCAN_INTERVAL_MS };
    if stale { rescan(state, "lazy"); }
}

fn send(out: &mut impl Write, v: &Value) {
    let s = serde_json::to_string(v).unwrap();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{}", s.len(), s);
    let _ = out.flush();
}

fn symbol_json(e: &Endpoint) -> Value {
    let line = (e.line.saturating_sub(1)) as u64;
    let container = if e.method_name.is_empty() { e.class_name.clone() } else { format!("{}.{}", e.class_name, e.method_name) };
    json!({
        "name": format!("{} {}", e.method, e.path),
        "kind": 6,
        "location": {
            "uri": path_to_uri(&e.file),
            "range": { "start": { "line": line, "character": 0 }, "end": { "line": line, "character": 0 } }
        },
        "containerName": container
    })
}

fn handle(msg: &Value, state: &mut State, out: &mut impl Write) {
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let id = msg.get("id").cloned();
    let params = msg.get("params").cloned().unwrap_or(Value::Null);

    // 通知（无 id）
    let id = match id { Some(v) if !v.is_null() => v, _ => {
        match method {
            "initialized" => log("initialized"),
            "exit" => { log("exit"); std::process::exit(0); }
            "workspace/didChangeWatchedFiles" => rescan(state, "watched"),
            _ => {}
        }
        return;
    }};

    log(&format!("request: {}{}", method,
        if method == "workspace/symbol" { format!(" query={}", params.get("query").unwrap_or(&Value::Null)) } else { String::new() }));

    let result: Value = match method {
        "initialize" => {
            let root = params.get("workspaceFolders").and_then(|f| f.get(0)).and_then(|f| f.get("uri")).and_then(|u| u.as_str())
                .and_then(uri_to_path)
                .or_else(|| params.get("rootUri").and_then(|u| u.as_str()).and_then(uri_to_path))
                .or_else(|| params.get("rootPath").and_then(|u| u.as_str()).map(PathBuf::from))
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            state.root = root;
            log(&format!("initialize root={}", state.root.display()));
            rescan(state, "initialize");
            json!({
                "capabilities": { "positionEncoding": "utf-16", "workspaceSymbolProvider": true },
                "serverInfo": { "name": "rest-nav", "version": env!("CARGO_PKG_VERSION") }
            })
        }
        "workspace/symbol" => {
            ensure_fresh(state);
            let query = params.get("query").and_then(|q| q.as_str()).unwrap_or("");
            let matched = filter_endpoints(&state.endpoints, query);
            let symbols: Vec<Value> = matched.iter().take(MAX_RESULTS).map(|e| symbol_json(e)).collect();
            log(&format!("workspace/symbol query={:?} -> {}", query, symbols.len()));
            Value::Array(symbols)
        }
        "shutdown" => { log("shutdown"); Value::Null }
        _ => Value::Null,
    };
    send(out, &json!({ "jsonrpc": "2.0", "id": id, "result": result }));
}

fn run_lsp() -> io::Result<()> {
    log("=== rest-nav-lsp started ===");
    let mut state = State { root: std::env::current_dir().unwrap_or_default(), endpoints: Vec::new(), last_scan: None };
    let stdin = io::stdin();
    let mut lock = stdin.lock();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    loop {
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            if lock.read_line(&mut line)? == 0 { return Ok(()); }
            let t = line.trim_end();
            if t.is_empty() { break; }
            if let Some(v) = t.strip_prefix("Content-Length:") {
                content_length = v.trim().parse().unwrap_or(0);
            }
        }
        if content_length == 0 { continue; }
        let mut body = vec![0u8; content_length];
        lock.read_exact(&mut body)?;
        match serde_json::from_slice::<Value>(&body) {
            Ok(msg) => handle(&msg, &mut state, &mut out),
            Err(e) => log(&format!("bad json: {}", e)),
        }
    }
}

fn run_scan(root: &str, as_json: bool) {
    let (files, eps) = scan_project(&PathBuf::from(root));
    if as_json {
        let arr: Vec<Value> = eps.iter().map(|e| json!({
            "method": e.method, "path": e.path, "file": e.file, "rel": e.rel,
            "line": e.line, "annoLine": e.anno_line, "className": e.class_name, "methodName": e.method_name
        })).collect();
        println!("{}", serde_json::to_string_pretty(&arr).unwrap());
    } else {
        println!("扫描 {} 个 .java 文件，识别 {} 个接口：", files, eps.len());
        for e in &eps {
            let handler = if e.method_name.is_empty() { e.class_name.clone() } else { format!("{}.{}", e.class_name, e.method_name) };
            println!("  {:<6} {:<40} -> {}:{}  ({})", e.method, e.path, e.rel, e.line, handler);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--scan") {
        let root = args.get(pos + 1).cloned().unwrap_or_else(|| ".".to_string());
        run_scan(&root, args.iter().any(|a| a == "--json"));
        return;
    }
    if let Err(e) = run_lsp() {
        log(&format!("lsp io error: {}", e));
    }
}