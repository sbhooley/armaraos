//! Structured extraction from spreadsheets, PDFs, and DOCX for agent tools.

use std::io::Read;
use std::path::Path;

use calamine::{open_workbook_auto, Data, Reader};
use regex_lite::Regex;
use rust_xlsxwriter::{Formula, Workbook};
use serde_json::Value;

const MAX_DOC_BYTES: usize = 20 * 1024 * 1024;
const MAX_OUTPUT_CHARS: usize = 450_000;

const DEFAULT_MAX_SHEETS: usize = 8;
const DEFAULT_MAX_ROWS: usize = 400;
const DEFAULT_MAX_COLS: usize = 40;
const MAX_SHEETS_CAP: usize = 20;
const MAX_ROWS_CAP: usize = 2000;
const MAX_COLS_CAP: usize = 100;

fn cell_display(dt: &Data) -> String {
    match dt {
        Data::Int(i) => i.to_string(),
        Data::Float(f) => f.to_string(),
        Data::String(s) => s.clone(),
        Data::Bool(b) => b.to_string(),
        Data::Error(e) => format!("#ERR:{e:?}"),
        Data::Empty => String::new(),
        Data::DateTime(dt) => format!("{dt:?}"),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
    }
}

fn extract_spreadsheet_text(
    path: &Path,
    max_sheets: usize,
    max_rows: usize,
    max_cols: usize,
) -> Result<String, String> {
    let mut workbook = open_workbook_auto(path).map_err(|e| format!("spreadsheet: {e}"))?;
    let names = workbook.sheet_names().to_vec();
    let mut out = String::new();
    out.push_str(&format!(
        "Format: spreadsheet ({} sheet(s) total; showing up to {}).\n",
        names.len(),
        max_sheets
    ));
    out.push_str(
        "Note: extracted values are usually cached results; original Excel formulas may not appear.\n\n",
    );

    for name in names.iter().take(max_sheets) {
        let range = workbook
            .worksheet_range(name)
            .map_err(|e| format!("sheet {name}: {e}"))?;
        out.push_str(&format!("## Sheet: {name}\n"));
        let (height, width) = range.get_size();
        let row_lim = height.min(max_rows);
        let col_lim = width.min(max_cols);
        for r in 0..row_lim {
            let mut row_cells = Vec::new();
            for c in 0..col_lim {
                let cell = range.get((r, c)).unwrap_or(&Data::Empty);
                let s = cell_display(cell);
                row_cells.push(if s.contains('\t') || s.contains('\n') {
                    format!("\"{}\"", s.replace('\"', "\"\""))
                } else {
                    s
                });
            }
            out.push_str(&row_cells.join("\t"));
            out.push('\n');
        }
        if height > max_rows || width > max_cols {
            out.push_str(&format!(
                "(truncated: sheet size {}×{}, showing {}×{})\n",
                height, width, row_lim, col_lim
            ));
        }
        out.push('\n');
    }
    Ok(out)
}

fn extract_pdf_text(bytes: &[u8]) -> Result<String, String> {
    pdf_extract::extract_text_from_mem(bytes).map_err(|e| format!("pdf: {e}"))
}

fn extract_docx_text(bytes: &[u8]) -> Result<String, String> {
    let cursor = std::io::Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(cursor).map_err(|e| format!("docx zip: {e}"))?;
    let mut xml = String::new();
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("docx zip entry: {e}"))?;
        if file.name() == "word/document.xml" {
            file.read_to_string(&mut xml)
                .map_err(|e| format!("docx read document.xml: {e}"))?;
            break;
        }
    }
    if xml.is_empty() {
        return Err("docx: missing word/document.xml".to_string());
    }
    let re = Regex::new(r"<w:t[^>]*>([^<]*)</w:t>").map_err(|e| format!("regex: {e}"))?;
    let mut parts: Vec<&str> = Vec::new();
    for cap in re.captures_iter(&xml) {
        if let Some(m) = cap.get(1) {
            parts.push(m.as_str());
        }
    }
    Ok(parts.join(" "))
}

fn truncate_output(mut s: String) -> String {
    if s.len() <= MAX_OUTPUT_CHARS {
        return s;
    }
    s.truncate(MAX_OUTPUT_CHARS);
    s.push_str("\n\n[Output truncated to ");
    s.push_str(&MAX_OUTPUT_CHARS.to_string());
    s.push_str(" characters.]");
    s
}

pub async fn tool_document_extract(
    input: &Value,
    workspace_root: Option<&Path>,
    ainl_library_root: Option<&Path>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let max_sheets = input["max_sheets"]
        .as_u64()
        .unwrap_or(DEFAULT_MAX_SHEETS as u64) as usize;
    let max_rows = input["max_rows_per_sheet"]
        .as_u64()
        .unwrap_or(DEFAULT_MAX_ROWS as u64) as usize;
    let max_cols = input["max_cols"]
        .as_u64()
        .unwrap_or(DEFAULT_MAX_COLS as u64) as usize;

    let max_sheets = max_sheets.clamp(1, MAX_SHEETS_CAP);
    let max_rows = max_rows.clamp(1, MAX_ROWS_CAP);
    let max_cols = max_cols.clamp(1, MAX_COLS_CAP);

    let resolved =
        crate::tool_runner::resolve_file_path_read(raw_path, workspace_root, ainl_library_root)?;

    if tokio::fs::metadata(&resolved)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false)
    {
        return Err("Path is a directory. Pass a file path such as uploads/your.xlsx.".to_string());
    }

    let bytes = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read file: {e}"))?;
    if bytes.len() > MAX_DOC_BYTES {
        return Err(format!(
            "File too large ({} bytes; max {} MB).",
            bytes.len(),
            MAX_DOC_BYTES / (1024 * 1024)
        ));
    }

    let lower = resolved.to_string_lossy().to_lowercase();
    let ext = Path::new(&lower)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let inner = match ext {
        "xlsx" | "xls" | "xlsb" | "ods" => {
            let path = resolved.clone();
            tokio::task::spawn_blocking(move || {
                extract_spreadsheet_text(&path, max_sheets, max_rows, max_cols)
            })
            .await
            .map_err(|e| format!("join: {e}"))?
        }
        "pdf" => {
            let b = bytes.clone();
            tokio::task::spawn_blocking(move || extract_pdf_text(&b))
                .await
                .map_err(|e| format!("join: {e}"))?
        }
        "docx" => {
            let b = bytes.clone();
            tokio::task::spawn_blocking(move || extract_docx_text(&b))
                .await
                .map_err(|e| format!("join: {e}"))?
        }
        _ => {
            return Err(format!(
                "Unsupported extension '{ext}'. Use document_extract with .xlsx, .xls, .xlsb, .ods, .pdf, or .docx."
            ));
        }
    }?;

    let header = format!("document_extract: {}\n---\n", resolved.display());
    Ok(truncate_output(format!("{header}{inner}")))
}

pub async fn tool_spreadsheet_build(
    input: &Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let sheets = input
        .get("sheets")
        .and_then(|v| v.as_array())
        .ok_or("Missing 'sheets' array")?;

    if sheets.is_empty() {
        return Err("sheets must contain at least one sheet".to_string());
    }
    if sheets.len() > 32 {
        return Err("Too many sheets (max 32)".to_string());
    }

    let resolved = crate::tool_runner::resolve_file_path(raw_path, workspace_root)?;
    if !resolved
        .to_string_lossy()
        .to_ascii_lowercase()
        .ends_with(".xlsx")
    {
        return Err("path must end with .xlsx".to_string());
    }

    let path = resolved.clone();
    let sheets_clone = sheets.clone();

    tokio::task::spawn_blocking(move || write_xlsx_from_json(&path, sheets_clone))
        .await
        .map_err(|e| format!("join: {e}"))?
}

fn write_xlsx_from_json(path: &Path, sheets: Vec<Value>) -> Result<String, String> {
    let mut workbook = Workbook::new();

    for (idx, sheet_val) in sheets.iter().enumerate() {
        let obj = sheet_val
            .as_object()
            .ok_or("Each sheet must be an object")?;
        let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or("Sheet1");
        let name = if name.is_empty() {
            format!("Sheet{}", idx + 1)
        } else {
            name.to_string()
        };
        let rows = obj
            .get("rows")
            .and_then(|v| v.as_array())
            .ok_or("Each sheet needs a 'rows' array")?;

        let worksheet = workbook
            .add_worksheet()
            .set_name(&name)
            .map_err(|e| format!("xlsx sheet name: {e}"))?;

        if rows.len() > 10_000 {
            return Err("Too many rows (max 10000 per sheet)".to_string());
        }

        for (r, row) in rows.iter().enumerate() {
            let row_arr = row
                .as_array()
                .ok_or("Each row must be an array of cell values")?;
            if row_arr.len() > 512 {
                return Err("Too many columns (max 512)".to_string());
            }
            for (c, cell) in row_arr.iter().enumerate() {
                if cell.is_null() {
                    continue;
                }
                match cell {
                    Value::Number(n) => {
                        if let Some(i) = n.as_i64() {
                            worksheet
                                .write_number(r as u32, c as u16, i as f64)
                                .map_err(|e| format!("xlsx write: {e}"))?;
                        } else if let Some(f) = n.as_f64() {
                            worksheet
                                .write_number(r as u32, c as u16, f)
                                .map_err(|e| format!("xlsx write: {e}"))?;
                        }
                    }
                    Value::Bool(b) => {
                        worksheet
                            .write_boolean(r as u32, c as u16, *b)
                            .map_err(|e| format!("xlsx write: {e}"))?;
                    }
                    Value::String(s) => {
                        let t = s.trim();
                        if t.starts_with('=') {
                            let formula = Formula::new(t);
                            worksheet
                                .write_formula(r as u32, c as u16, &formula)
                                .map_err(|e| format!("xlsx formula: {e}"))?;
                        } else {
                            worksheet
                                .write_string(r as u32, c as u16, t)
                                .map_err(|e| format!("xlsx write: {e}"))?;
                        }
                    }
                    _ => {
                        let s = cell.to_string();
                        worksheet
                            .write_string(r as u32, c as u16, &s)
                            .map_err(|e| format!("xlsx write: {e}"))?;
                    }
                }
            }
        }
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("mkdir: {e}"))?;
    }

    workbook.save(path).map_err(|e| format!("xlsx save: {e}"))?;
    Ok(format!("Wrote spreadsheet to {}", path.display()))
}
