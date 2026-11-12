//! Spring MVC REST 接口扫描器（rest-endpoints.js 的 Rust 移植）。
//! 基于正则 + 注释剥离（保留字符串与行号），非完整 Java 解析；
//! 组合注解/元注解、接口继承等复杂场景可能漏识别，属最小可用版取舍。

use regex::Regex;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 跳过的目录（与 zed-java-init.js 约定一致）
const SKIP: &[&str] = &[
    "target", "node_modules", ".git", ".idea", ".zed", ".settings", "dist", "build", "log",
    "logs", "doc", "docs", "targetImages", ".gradle", "out", "bin",
];

#[derive(Debug, Clone)]
pub struct Endpoint {
    pub method: String,
    pub path: String,
    pub file: String,
    pub rel: String,
    pub line: usize,      // 1-based，跳转目标（方法签名行，找不到则注解行）
    pub anno_line: usize, // 1-based，注解行
    pub class_name: String,
    pub method_name: String,
}

fn shortcut_method(name: &str) -> Option<&'static str> {
    match name {
        "GetMapping" => Some("GET"),
        "PostMapping" => Some("POST"),
        "PutMapping" => Some("PUT"),
        "DeleteMapping" => Some("DELETE"),
        "PatchMapping" => Some("PATCH"),
        _ => None,
    }
}

fn mapping_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"@\s*(RequestMapping|GetMapping|PostMapping|PutMapping|DeleteMapping|PatchMapping)\b")
            .unwrap()
    })
}

fn class_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\bclass\s+([A-Za-z_$][\w$]*)").unwrap())
}

fn method_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"([A-Za-z_$][\w$]*)\s*\(").unwrap())
}

/// 注释剥离：只清掉 // 与 /* */，保留字符串内容与换行（字节偏移/行号不变）。
/// 按字节处理：语法字符均为 ASCII，中文等多字节字符在字符串里原样保留、在注释里逐字节替换为空格。
pub fn strip_comments(s: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0usize;
    let n = s.len();
    // 0=code 1=line 2=block 3=str 4=chr
    let mut state = 0u8;
    while i < n {
        let c = s[i];
        let d = if i + 1 < n { s[i + 1] } else { 0 };
        match state {
            0 => {
                if c == b'/' && d == b'/' { state = 1; out.extend_from_slice(b"  "); i += 2; continue; }
                if c == b'/' && d == b'*' { state = 2; out.extend_from_slice(b"  "); i += 2; continue; }
                if c == b'"' { state = 3; out.push(c); i += 1; continue; }
                if c == b'\'' { state = 4; out.push(c); i += 1; continue; }
                out.push(c); i += 1; continue;
            }
            1 => {
                if c == b'\n' { state = 0; out.push(b'\n'); } else { out.push(b' '); }
                i += 1; continue;
            }
            2 => {
                if c == b'*' && d == b'/' { state = 0; out.extend_from_slice(b"  "); i += 2; continue; }
                out.push(if c == b'\n' { b'\n' } else { b' ' }); i += 1; continue;
            }
            3 => {
                if c == b'\\' { out.push(c); if i + 1 < n { out.push(s[i + 1]); } i += 2; continue; }
                if c == b'"' { state = 0; }
                out.push(c); i += 1; continue;
            }
            _ => {
                if c == b'\\' { out.push(c); if i + 1 < n { out.push(s[i + 1]); } i += 2; continue; }
                if c == b'\'' { state = 0; }
                out.push(c); i += 1; continue;
            }
        }
    }
    out
}

/// 读取平衡圆括号内容，返回 (内部内容, 结束偏移)。s[i] == '('
fn read_paren(s: &[u8], i: usize) -> (String, usize) {
    let mut depth = 0i32;
    let mut j = i;
    let mut in_str = false;
    while j < s.len() {
        let c = s[j];
        if in_str {
            if c == b'\\' { j += 1; } else if c == b'"' { in_str = false; }
            j += 1; continue;
        }
        if c == b'"' { in_str = true; }
        else if c == b'(' { depth += 1; }
        else if c == b')' { depth -= 1; if depth == 0 { j += 1; break; } }
        j += 1;
    }
    let inner = String::from_utf8_lossy(&s[i + 1..j.saturating_sub(1)]).into_owned();
    (inner, j)
}

/// 读取一个属性值：字符串、{数组}、或裸 token（到顶层逗号/右括号为止）
fn read_value(s: &[u8], i: usize) -> String {
    let mut i = i;
    while i < s.len() && (s[i] as char).is_whitespace() { i += 1; }
    if i >= s.len() { return String::new(); }
    if s[i] == b'{' {
        let mut depth = 0i32; let mut j = i; let mut in_str = false;
        while j < s.len() {
            let c = s[j];
            if in_str { if c == b'\\' { j += 1; } else if c == b'"' { in_str = false; } j += 1; continue; }
            if c == b'"' { in_str = true; }
            else if c == b'{' { depth += 1; }
            else if c == b'}' { depth -= 1; if depth == 0 { j += 1; break; } }
            j += 1;
        }
        return String::from_utf8_lossy(&s[i..j]).into_owned();
    }
    if s[i] == b'"' {
        let mut j = i + 1;
        while j < s.len() { if s[j] == b'\\' { j += 1; } else if s[j] == b'"' { j += 1; break; } j += 1; }
        return String::from_utf8_lossy(&s[i..j]).into_owned();
    }
    let mut j = i; let mut depth = 0i32; let mut in_str = false;
    while j < s.len() {
        let c = s[j];
        if in_str { if c == b'\\' { j += 1; } else if c == b'"' { in_str = false; } j += 1; continue; }
        if c == b'"' { in_str = true; }
        else if c == b'(' || c == b'{' { depth += 1; }
        else if c == b')' || c == b'}' { if depth == 0 { break; } depth -= 1; }
        else if c == b',' && depth == 0 { break; }
        j += 1;
    }
    String::from_utf8_lossy(&s[i..j]).into_owned()
}

fn extract_strings(s: &str) -> Vec<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r#""((?:[^"\\]|\\.)*)""#).unwrap());
    re.captures_iter(s).map(|c| c[1].to_string()).collect()
}

/// 找到 value= / path= 的起始（返回等号后的偏移），用字节扫描避免正则跨字符串误匹配
fn find_attr(args: &str, attr: &str) -> Option<usize> {
    let b = args.as_bytes();
    let pat = format!("{}", attr);
    let pb = pat.as_bytes();
    let mut i = 0usize;
    while i + pb.len() <= b.len() {
        if &b[i..i + pb.len()] == pb {
            // 前一个字符不能是单词字符（避免匹配 xxxvalue）
            let prev_ok = i == 0 || !(b[i - 1] as char).is_alphanumeric() && b[i - 1] != b'_';
            let mut k = i + pb.len();
            while k < b.len() && (b[k] as char).is_whitespace() { k += 1; }
            if prev_ok && k < b.len() && b[k] == b'=' {
                k += 1;
                while k < b.len() && (b[k] as char).is_whitespace() { k += 1; }
                return Some(k);
            }
        }
        i += 1;
    }
    None
}

fn parse_args(name: &str, args: &str) -> (Vec<String>, Vec<String>) {
    let b = args.as_bytes();
    let mut paths: Vec<String> = vec![String::new()];
    let path_rhs: Option<String> = if let Some(k) = find_attr(args, "value").or_else(|| find_attr(args, "path")) {
        Some(read_value(b, k))
    } else {
        let t = args.trim_start();
        if t.starts_with('"') || t.starts_with('{') {
            let off = args.len() - t.len();
            Some(read_value(b, off))
        } else { None }
    };
    if let Some(rhs) = path_rhs {
        let ps = extract_strings(&rhs);
        if !ps.is_empty() { paths = ps; }
    }

    let mut methods: Vec<String> = Vec::new();
    if let Some(m) = shortcut_method(name) {
        methods.push(m.to_string());
    } else {
        if let Some(k) = find_attr(args, "method") {
            let rhs = read_value(b, k);
            static RE: OnceLock<Regex> = OnceLock::new();
            let re = RE.get_or_init(|| Regex::new(r"RequestMethod\s*\.\s*(\w+)").unwrap());
            let mut found: Vec<String> = re.captures_iter(&rhs).map(|c| c[1].to_uppercase()).collect();
            if found.is_empty() {
                static RE2: OnceLock<Regex> = OnceLock::new();
                let re2 = RE2.get_or_init(|| Regex::new(r"\b(GET|POST|PUT|DELETE|PATCH|HEAD|OPTIONS|TRACE)\b").unwrap());
                found = re2.captures_iter(&rhs).map(|c| c[1].to_string()).collect();
            }
            methods = found;
        }
        if methods.is_empty() { methods.push("ANY".to_string()); }
    }
    (paths, methods)
}

fn join_path(base: &str, sub: &str) -> String {
    let mut p = format!("/{}/{}", base, sub);
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"/{2,}").unwrap());
    p = re.replace_all(&p, "/").into_owned();
    if p.len() > 1 { p = p.trim_end_matches('/').to_string(); }
    if p.is_empty() { "/".to_string() } else { p }
}

fn line_of(src: &str, byte_idx: usize) -> usize {
    src[..byte_idx].bytes().filter(|&b| b == b'\n').count() + 1
}

/// 从注解结束位置向后找方法签名，返回 (方法名, 1-based 行号)
fn find_method(src: &str, from: usize) -> Option<(String, usize)> {
    let kw = ["if", "for", "while", "switch", "catch", "synchronized", "return", "new", "else", "do", "try"];
    let rest = &src[from..];
    let mut offset = from;
    for ln in rest.split('\n') {
        let trimmed = ln.trim();
        if trimmed.is_empty() || trimmed.starts_with('@') || trimmed.starts_with("//") {
            offset += ln.len() + 1; continue;
        }
        if let Some(c) = method_re().captures(ln) {
            let name = c[1].to_string();
            if !kw.contains(&name.as_str()) {
                return Some((name, line_of(src, offset)));
            }
        }
        offset += ln.len() + 1;
        if offset - from > 4000 { break; }
    }
    None
}

fn parse_file(file: &Path, root: &Path) -> Vec<Endpoint> {
    let raw = match fs::read(file) { Ok(b) => b, Err(_) => return Vec::new() };
    // 快速预筛
    let raw_str = String::from_utf8_lossy(&raw);
    if !mapping_re().is_match(&raw_str) { return Vec::new(); }
    let stripped = strip_comments(&raw);
    let src = String::from_utf8_lossy(&stripped).into_owned();

    let (class_name, class_pos) = match class_re().captures(&src) {
        Some(c) => (c[1].to_string(), c.get(0).unwrap().start()),
        None => (file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), 0),
    };

    let mut base_paths: Vec<String> = vec![String::new()];
    let mut method_annos: Vec<(String, String, usize, usize)> = Vec::new(); // name, args, start, end
    for m in mapping_re().captures_iter(&src) {
        let name = m[1].to_string();
        let start = m.get(0).unwrap().start();
        let after_name = m.get(0).unwrap().end();
        let b = src.as_bytes();
        let mut k = after_name;
        while k < b.len() && (b[k] as char).is_whitespace() { k += 1; }
        let (args, end) = if k < b.len() && b[k] == b'(' {
            let (inner, e) = read_paren(b, k);
            (inner, e)
        } else { (String::new(), after_name) };

        if start < class_pos && name == "RequestMapping" {
            let (ps, _) = parse_args(&name, &args);
            base_paths = if ps.is_empty() { vec![String::new()] } else { ps };
        } else if start > class_pos {
            method_annos.push((name, args, start, end));
        }
    }

    let rel = file.strip_prefix(root).unwrap_or(file).to_string_lossy().replace('\\', "/");
    let file_str = file.to_string_lossy().into_owned();
    let mut out = Vec::new();
    for (name, args, start, end) in method_annos {
        let (paths, methods) = parse_args(&name, &args);
        let meth = find_method(&src, end);
        let anno_line = line_of(&src, start);
        let (line, method_name) = match &meth {
            Some((n, l)) => (*l, n.clone()),
            None => (anno_line, String::new()),
        };
        for base in &base_paths {
            for sub in &paths {
                let full = join_path(base, sub);
                for method in &methods {
                    out.push(Endpoint {
                        method: method.clone(),
                        path: full.clone(),
                        file: file_str.clone(),
                        rel: rel.clone(),
                        line,
                        anno_line,
                        class_name: class_name.clone(),
                        method_name: method_name.clone(),
                    });
                }
            }
        }
    }
    out
}

fn walk(dir: &Path, depth: usize, files: &mut Vec<PathBuf>) {
    if depth > 12 { return; }
    let entries = match fs::read_dir(dir) { Ok(e) => e, Err(_) => return };
    for e in entries.flatten() {
        let path = e.path();
        let ft = match e.file_type() { Ok(t) => t, Err(_) => continue };
        if ft.is_dir() {
            let name = e.file_name().to_string_lossy().into_owned();
            if SKIP.contains(&name.as_str()) { continue; }
            walk(&path, depth + 1, files);
        } else if ft.is_file() && path.extension().map(|x| x == "java").unwrap_or(false) {
            files.push(path);
        }
    }
}

pub fn scan_project(root: &Path) -> (usize, Vec<Endpoint>) {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut files = Vec::new();
    walk(&root, 0, &mut files);
    let count = files.len();
    let mut endpoints = Vec::new();
    for f in &files {
        endpoints.extend(parse_file(f, &root));
    }
    endpoints.sort_by(|a, b| {
        if a.path == b.path { a.method.cmp(&b.method) } else { a.path.cmp(&b.path) }
    });
    (count, endpoints)
}

fn is_subsequence(needle: &str, hay: &str) -> bool {
    let mut it = hay.chars();
    for nc in needle.chars() {
        loop {
            match it.next() { Some(hc) if hc == nc => break, Some(_) => continue, None => return false }
        }
    }
    true
}

pub fn filter_endpoints<'a>(endpoints: &'a [Endpoint], query: &str) -> Vec<&'a Endpoint> {
    let q = query.trim().to_lowercase();
    if q.is_empty() { return endpoints.iter().collect(); }
    endpoints.iter().filter(|e| {
        let text = format!("{} {} {}.{}", e.method, e.path, e.class_name, e.method_name).to_lowercase();
        text.contains(&q) || is_subsequence(&q, &text)
    }).collect()
}