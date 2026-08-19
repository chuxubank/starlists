use anyhow::Error;
use serde::Serialize;
use serde_json::{json, Value};

use crate::cli::Format;

#[derive(Debug)]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
    pub hint: Option<String>,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
        }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for AppError {}

pub fn success(format: Format, data: impl Serialize, table: impl FnOnce() -> String) {
    match format {
        Format::Table => print!("{}", table()),
        Format::Json => {
            let body = json!({ "ok": true, "data": data });
            println!("{}", serde_json::to_string_pretty(&body).unwrap());
        }
        Format::Sexp => {
            let value = serde_json::to_value(data).unwrap_or(Value::Null);
            println!("{}", value_to_sexp(&value));
        }
    }
}

pub fn fail(format: Format, err: &Error) -> i32 {
    let app = err.downcast_ref::<AppError>();
    let code = app.map(|e| e.code).unwrap_or("error");
    let message = app
        .map(|e| e.message.clone())
        .unwrap_or_else(|| format!("{err:#}"));
    let hint = app.and_then(|e| e.hint.clone());

    match format {
        Format::Json => {
            let mut error = json!({ "code": code, "message": message });
            if let Some(hint) = &hint {
                error["hint"] = json!(hint);
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({ "ok": false, "error": error })).unwrap()
            );
        }
        Format::Sexp => {
            let mut pairs = vec![
                sexp_pair("ok", &Value::Bool(false)),
                sexp_pair("code", &Value::String(code.to_string())),
                sexp_pair("message", &Value::String(message.clone())),
            ];
            if let Some(hint) = &hint {
                pairs.push(sexp_pair("hint", &Value::String(hint.clone())));
            }
            println!("({})", pairs.join(" "));
        }
        Format::Table => {
            eprintln!("error: {message}");
            if let Some(hint) = hint {
                eprintln!("hint: {hint}");
            }
        }
    }
    match code {
        "auth_missing" | "auth_scope" => 2,
        "not_found" => 3,
        "conflict" => 4,
        _ => 1,
    }
}

pub fn value_to_sexp(value: &Value) -> String {
    match value {
        Value::Null => "nil".to_string(),
        Value::Bool(true) => "t".to_string(),
        Value::Bool(false) => "nil".to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => format!("\"{}\"", escape_sexp(s)),
        Value::Array(items) => {
            let inner = items
                .iter()
                .map(value_to_sexp)
                .collect::<Vec<_>>()
                .join(" ");
            format!("({inner})")
        }
        Value::Object(map) => {
            let inner = map
                .iter()
                .map(|(k, v)| format!("({} . {})", k, value_to_sexp(v)))
                .collect::<Vec<_>>()
                .join(" ");
            format!("({inner})")
        }
    }
}

fn sexp_pair(key: &str, value: &Value) -> String {
    format!("({key} . {})", value_to_sexp(value))
}

fn escape_sexp(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

pub fn kv_table(rows: &[(&str, String)]) -> String {
    let width = rows.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    let mut out = String::new();
    for (k, v) in rows {
        out.push_str(&format!("{k:<width$}  {v}\n"));
    }
    out
}

pub fn columns(headers: &[&str], rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return format!("{}\n", headers.join("  "));
    }
    let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(cell.len());
        }
    }
    let mut out = String::new();
    out.push_str(
        &headers
            .iter()
            .enumerate()
            .map(|(i, h)| format!("{h:<w$}", w = widths[i]))
            .collect::<Vec<_>>()
            .join("  "),
    );
    out.push('\n');
    for row in rows {
        out.push_str(
            &row.iter()
                .enumerate()
                .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
                .collect::<Vec<_>>()
                .join("  "),
        );
        out.push('\n');
    }
    out
}
