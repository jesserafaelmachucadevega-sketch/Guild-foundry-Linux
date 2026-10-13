// SPDX-License-Identifier: Apache-2.0
// document.rs — document text extraction, comparison, conversion, OCR, and
// upload intake for the Personal Agent's attach pipeline.
//
// Native (pure-Rust) extraction covers: text/code formats, HTML, PDF, DOCX,
// PPTX, XLSX/XLS/ODS, ODT/ODP, RTF. Conversions beyond text formats and OCR
// use system binaries when installed (pandoc / LibreOffice / Tesseract /
// Poppler) and say exactly what is missing when they are not — never a
// silent fake.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

use base64::Engine as _;
use serde_json::{json, Value};

/// Chars returned by the extract/ocr tools.
const EXTRACT_CAP: usize = 200_000;
/// Chars attached per uploaded file in the chat pipeline.
const UPLOAD_CAP: usize = 50_000;

pub(crate) fn ext_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_default()
}

fn cap_text(text: &str, cap: usize) -> (String, bool) {
    if text.len() <= cap {
        return (text.to_string(), false);
    }
    let mut end = cap;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}…", &text[..end]), true)
}

fn read_utf8(p: &Path) -> Result<String, String> {
    let bytes = fs::read(p).map_err(|e| format!("read failed: {}", e))?;
    let probe = &bytes[..bytes.len().min(8192)];
    if probe.contains(&0) {
        return Err(format!(
            "unsupported binary format '{}' — no text extractor for it",
            ext_of(&p.to_string_lossy())
        ));
    }
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn is_image_ext(ext: &str) -> bool {
    matches!(ext, "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif")
}

// ---------------------------------------------------------------------------
// Extraction entry points
// ---------------------------------------------------------------------------

pub fn extract_text(path: &str, ocr_flag: Option<bool>) -> Result<Value, String> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err(format!("file not found: {}", path));
    }
    let ext = ext_of(path);
    let text = if is_image_ext(&ext) && ocr_flag.unwrap_or(false) {
        return self::ocr(path, None);
    } else {
        extract_file(p, &ext)?
    };
    let (capped, truncated) = cap_text(&text, EXTRACT_CAP);
    Ok(json!({
        "file": p.file_name().and_then(|s| s.to_str()).unwrap_or(path),
        "format": if ext.is_empty() { "unknown" } else { ext.as_str() },
        "chars": text.len(),
        "truncated": truncated,
        "text": capped,
    }))
}

fn extract_file(p: &Path, ext: &str) -> Result<String, String> {
    match ext {
        "txt" | "md" | "markdown" | "csv" | "tsv" | "log" | "json" | "xml" | "yaml" | "yml"
        | "toml" | "ini" | "conf" | "tex" | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "rb"
        | "go" | "java" | "kt" | "swift" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "php"
        | "css" | "scss" | "sql" | "sh" | "bash" | "zsh" | "lua" | "pl" | "r" | "dart" | "vue"
        | "svelte" => read_utf8(p),
        "htm" | "html" | "xhtml" => Ok(crate::integrations::web::html_to_text(&read_utf8(p)?)),
        "pdf" => pdf_extract::extract_text(p).map_err(|e| format!("PDF extraction failed: {}", e)),
        "docx" | "dotx" | "docm" => word_extract(p),
        "pptx" | "potx" => pptx_extract(p),
        "xlsx" | "xlsm" | "xlsb" | "xls" | "ods" => sheet_extract(p),
        "odt" | "ott" | "odp" | "odg" => odf_extract(p),
        "rtf" => Ok(strip_rtf(&read_utf8(p)?)),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "tif" => Err(
            "image file — pass ocr=true (or use document.ocr) to read text from images; requires Tesseract."
                .to_string(),
        ),
        "heic" | "heif" => {
            Err("HEIC images are not supported — convert to PNG/JPEG first.".to_string())
        }
        _ => read_utf8(p),
    }
}

// ---------------------------------------------------------------------------
// Office / ODF / RTF extraction
// ---------------------------------------------------------------------------

fn read_zip_entry(p: &Path, name: &str) -> Result<String, String> {
    let f = fs::File::open(p).map_err(|e| e.to_string())?;
    let mut zip =
        zip::ZipArchive::new(f).map_err(|e| format!("not a valid Office/ODF file: {}", e))?;
    let mut entry = zip
        .by_name(name)
        .map_err(|_| format!("missing {} inside the file", name))?;
    let mut s = String::new();
    entry.read_to_string(&mut s).map_err(|e| e.to_string())?;
    Ok(s)
}

fn word_extract(p: &Path) -> Result<String, String> {
    let xml = read_zip_entry(p, "word/document.xml")?;
    text_events(
        &xml,
        &[b"w:p", b"w:br"],
        &[b"w:tab"],
        &[b"w:instrText", b"w:delText"],
    )
}

fn pptx_extract(p: &Path) -> Result<String, String> {
    let f = fs::File::open(p).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("not a valid Office file: {}", e))?;
    let mut names: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        if let Ok(entry) = zip.by_index(i) {
            names.push(entry.name().to_string());
        }
    }
    let mut slides: Vec<String> = names
        .into_iter()
        .filter(|n| n.starts_with("ppt/slides/slide") && n.ends_with(".xml"))
        .collect();
    slides.sort_by_key(|n| {
        n.trim_start_matches("ppt/slides/slide")
            .trim_end_matches(".xml")
            .parse::<u64>()
            .unwrap_or(0)
    });
    let mut out = String::new();
    for (i, name) in slides.iter().enumerate() {
        let mut s = String::new();
        zip.by_name(name)
            .map_err(|e| e.to_string())?
            .read_to_string(&mut s)
            .map_err(|e| e.to_string())?;
        out.push_str(&format!("--- Slide {} ---\n", i + 1));
        out.push_str(&text_events(
            &s,
            &[b"a:p", b"a:br"],
            &[b"a:tab"],
            &[],
        )?);
        out.push('\n');
    }
    if out.trim().is_empty() {
        return Err("no slide text found in the presentation".to_string());
    }
    Ok(out)
}

fn odf_extract(p: &Path) -> Result<String, String> {
    let xml = read_zip_entry(p, "content.xml")?;
    text_events(
        &xml,
        &[b"text:p", b"text:h", b"text:line-break"],
        &[b"text:tab", b"text:s"],
        &[b"office:annotation", b"text:note"],
    )
}

/// Collect text from Office/ODF XML. `break_on` tags insert a newline,
/// `tab_on` tags insert a tab, text inside `skip_inside` elements is ignored.
fn text_events(
    xml: &str,
    break_on: &[&[u8]],
    tab_on: &[&[u8]],
    skip_inside: &[&[u8]],
) -> Result<String, String> {
    use quick_xml::events::Event;
    use quick_xml::Reader;

    fn in_list(n: &[u8], list: &[&[u8]]) -> bool {
        list.iter().any(|s| *s == n)
    }

    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);
    let mut out = String::new();
    let mut skip_depth = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = e.name();
                let n = name.as_ref();
                if in_list(n, skip_inside) {
                    skip_depth += 1;
                } else if skip_depth == 0 {
                    if in_list(n, break_on) {
                        out.push('\n');
                    } else if in_list(n, tab_on) {
                        out.push('\t');
                    }
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let n = name.as_ref();
                if in_list(n, skip_inside) && skip_depth > 0 {
                    skip_depth -= 1;
                }
            }
            Ok(Event::Empty(e)) => {
                let name = e.name();
                let n = name.as_ref();
                if skip_depth == 0 {
                    if in_list(n, break_on) {
                        out.push('\n');
                    } else if in_list(n, tab_on) {
                        out.push('\t');
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if skip_depth == 0 {
                    let s = t.unescape().map_err(|e| format!("XML parse error: {}", e))?;
                    out.push_str(&s);
                }
            }
            Ok(_) => {}
            Err(e) => return Err(format!("XML parse error: {}", e)),
        }
    }
    Ok(out)
}

fn data_to_string(d: &calamine::Data) -> String {
    use calamine::Data;
    match d {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => format!("{}", f),
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => b.to_string(),
        Data::Error(_) => "#ERR".to_string(),
        other => other.to_string(),
    }
}

fn sheet_extract(p: &Path) -> Result<String, String> {
    use calamine::{open_workbook_auto, Reader};
    let mut wb = open_workbook_auto(p).map_err(|e| format!("spreadsheet open failed: {}", e))?;
    let names = wb.sheet_names().to_owned();
    let mut out = String::new();
    for name in names {
        out.push_str(&format!("## Sheet: {}\n", name));
        match wb.worksheet_range(&name) {
            Ok(range) => {
                for row in range.rows() {
                    let cells: Vec<String> = row.iter().map(data_to_string).collect();
                    let line = cells.join("\t");
                    if !line.trim().is_empty() {
                        out.push_str(&line);
                        out.push('\n');
                    }
                }
            }
            Err(e) => out.push_str(&format!("(could not read sheet: {})\n", e)),
        }
        out.push('\n');
    }
    Ok(out)
}

/// Minimal RTF → text: drops control words/groups, keeps text runs, maps
/// \par and \line to newlines, decodes \'hh hex escapes.
fn strip_rtf(input: &str) -> String {
    let mut out = String::new();
    let mut chars = input.chars().peekable();
    let mut skip_group_depth: usize = 0; // inside {\*\...} ignorable groups
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(next) if next.is_ascii_alphabetic() => {
                    let mut word = String::new();
                    word.push(next);
                    while let Some(&ch) = chars.peek() {
                        if ch.is_ascii_alphabetic() {
                            word.push(ch);
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    while let Some(&ch) = chars.peek() {
                        if ch.is_ascii_digit() || ch == '-' {
                            chars.next();
                        } else {
                            break;
                        }
                    }
                    if let Some(&' ') = chars.peek() {
                        chars.next();
                    }
                    match word.as_str() {
                        "par" | "line" | "pard" => out.push('\n'),
                        "tab" => out.push('\t'),
                        _ => {}
                    }
                }
                Some('\'') => {
                    let h1 = chars.next();
                    let h2 = chars.next();
                    if let (Some(a), Some(b)) = (h1, h2) {
                        if let Ok(byte) = u8::from_str_radix(&format!("{}{}", a, b), 16) {
                            out.push(byte as char);
                        }
                    }
                }
                Some('*') => {
                    // Ignorable destination — skip its group body.
                    skip_group_depth = skip_group_depth.saturating_add(1);
                    let mut depth = 1i32;
                    let mut pending = 0i32;
                    for ch in chars.by_ref() {
                        if ch == '{' {
                            depth += 1;
                        } else if ch == '}' {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                        }
                        pending += 1;
                        let _ = pending;
                    }
                }
                Some('\\') => out.push('\\'),
                Some('{') => out.push('{'),
                Some('}') => out.push('}'),
                Some(other) => out.push(other),
                None => {}
            },
            '{' | '}' => {}
            _ => out.push(c),
        }
    }
    let _ = skip_group_depth;
    // Collapse runs of blank lines left by formatting groups.
    let lines: Vec<&str> = out.lines().map(|l| l.trim_end()).collect();
    let mut cleaned = String::new();
    let mut prev_blank = false;
    for l in lines {
        let blank = l.trim().is_empty();
        if blank && prev_blank {
            continue;
        }
        cleaned.push_str(l);
        cleaned.push('\n');
        prev_blank = blank;
    }
    cleaned.trim().to_string()
}

// ---------------------------------------------------------------------------
// compare
// ---------------------------------------------------------------------------

pub fn compare(old_path: &str, new_path: &str, format: Option<&str>) -> Result<Value, String> {
    let a = extract_text(old_path, None)?;
    let b = extract_text(new_path, None)?;
    let a_text = a["text"].as_str().unwrap_or("").to_string();
    let b_text = b["text"].as_str().unwrap_or("").to_string();
    let al: Vec<String> = a_text.lines().map(|s| s.to_string()).collect();
    let bl: Vec<String> = b_text.lines().map(|s| s.to_string()).collect();

    use std::collections::HashMap;
    let mut counts_a: HashMap<&str, i64> = HashMap::new();
    let mut counts_b: HashMap<&str, i64> = HashMap::new();
    for l in &al {
        *counts_a.entry(l.as_str()).or_default() += 1;
    }
    for l in &bl {
        *counts_b.entry(l.as_str()).or_default() += 1;
    }

    // Removed: A-lines whose match budget in B is exhausted (order preserved).
    let mut removed: Vec<String> = Vec::new();
    let mut budget: HashMap<&str, i64> = HashMap::new();
    for (line, ca) in counts_a.iter() {
        let cb = counts_b.get(line).copied().unwrap_or(0);
        budget.insert(line, (*ca).min(cb));
    }
    let mut budget_r = budget.clone();
    for l in &al {
        let b = budget_r.get_mut(l.as_str()).unwrap();
        if *b > 0 {
            *b -= 1;
        } else {
            removed.push(l.clone());
        }
    }
    // Added: B-lines whose match budget in A is exhausted.
    let mut added: Vec<String> = Vec::new();
    let mut budget_a: HashMap<&str, i64> = HashMap::new();
    for (line, cb) in counts_b.iter() {
        let ca = counts_a.get(line).copied().unwrap_or(0);
        budget_a.insert(line, (*cb).min(ca));
    }
    for l in &bl {
        let a = budget_a.get_mut(l.as_str()).unwrap();
        if *a > 0 {
            *a -= 1;
        } else {
            added.push(l.clone());
        }
    }

    let common = al.len().saturating_sub(removed.len());
    let denom = al.len().max(bl.len()).max(1);
    let similarity = ((common as f64) / (denom as f64) * 1000.0).round() / 1000.0;

    let list_cap = 200usize;
    let out = json!({
        "old": Path::new(old_path).file_name().and_then(|s| s.to_str()).unwrap_or(old_path),
        "new": Path::new(new_path).file_name().and_then(|s| s.to_str()).unwrap_or(new_path),
        "old_lines": al.len(),
        "new_lines": bl.len(),
        "common_lines": common,
        "similarity": similarity,
        "removed_total": removed.len(),
        "added_total": added.len(),
        "removed": removed.iter().take(list_cap).collect::<Vec<_>>(),
        "added": added.iter().take(list_cap).collect::<Vec<_>>(),
        "listed_limit": list_cap,
    });
    let fmt = format.unwrap_or("unified");
    if fmt == "unified" {
        let mut diff = String::new();
        diff.push_str(&format!(
            "--- {}\n+++ {}\n",
            out["old"].as_str().unwrap_or("old"),
            out["new"].as_str().unwrap_or("new")
        ));
        for l in removed.iter().take(list_cap) {
            diff.push_str(&format!("- {}\n", l));
        }
        for l in added.iter().take(list_cap) {
            diff.push_str(&format!("+ {}\n", l));
        }
        let mut merged = out;
        merged["diff"] = json!(diff);
        return Ok(merged);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// convert
// ---------------------------------------------------------------------------

pub fn convert(
    input_path: &str,
    output_format: &str,
    output_path: Option<&str>,
) -> Result<Value, String> {
    let src = Path::new(input_path);
    if !src.is_file() {
        return Err(format!("file not found: {}", input_path));
    }
    let target = output_format
        .trim()
        .trim_start_matches('.')
        .to_lowercase();
    if !["pdf", "docx", "md", "html", "txt", "rtf", "odt"].contains(&target.as_str()) {
        return Err(format!(
            "unsupported output format '{}' (use pdf, docx, md, html, txt, rtf, or odt)",
            target
        ));
    }
    let dest: PathBuf = match output_path {
        Some(o) if !o.trim().is_empty() => PathBuf::from(o),
        _ => {
            let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
            src.with_file_name(format!("{}.{}", stem, target))
        }
    };
    if let Some(dir) = dest.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
    }

    let src_ext = ext_of(input_path);

    // Native text conversions first: anything extractable → txt/md/html.
    let native: Option<String> = match target.as_str() {
        "txt" => Some(extract_file(src, &src_ext)?),
        "md" => Some(if matches!(src_ext.as_str(), "html" | "htm" | "xhtml") {
            crate::integrations::web::html_to_markdown(&read_utf8(src)?)
        } else {
            extract_file(src, &src_ext)?
        }),
        "html" => Some(if matches!(src_ext.as_str(), "md" | "markdown") {
            markdown_to_html(&read_utf8(src)?)
        } else {
            text_to_html(&extract_file(src, &src_ext)?)
        }),
        _ => None,
    };
    if let Some(content) = native {
        fs::write(&dest, content).map_err(|e| e.to_string())?;
        return Ok(conversion_result(input_path, &dest, &target));
    }

    // Office/PDF/RTF outputs need a system converter.
    let soffice_first = matches!(target.as_str(), "pdf" | "rtf");
    let attempts: Vec<(&str, fn(&Path, &Path, &str) -> Result<(), String>)> = if soffice_first {
        vec![
            ("libreoffice", convert_with_soffice),
            ("pandoc", convert_with_pandoc),
        ]
    } else {
        vec![
            ("pandoc", convert_with_pandoc),
            ("libreoffice", convert_with_soffice),
        ]
    };
    let mut errors: Vec<String> = Vec::new();
    for (name, f) in attempts {
        let bin = if name == "libreoffice" { "soffice" } else { "pandoc" };
        if !binary_exists(bin) {
            errors.push(format!("{}: {} is not installed", name, bin));
            continue;
        }
        match f(src, &dest, &target) {
            Ok(()) => return Ok(conversion_result(input_path, &dest, &target)),
            Err(e) => errors.push(format!("{}: {}", name, e)),
        }
    }
    Err(format!(
        "Converting to '{}' needs pandoc or LibreOffice installed and able to read '{}'. Attempts — {}",
        target,
        src_ext,
        errors.join(" | ")
    ))
}

fn conversion_result(input: &str, dest: &Path, target: &str) -> Value {
    json!({
        "input": input,
        "output": dest.to_string_lossy(),
        "format": target,
        "bytes": fs::metadata(dest).map(|m| m.len()).unwrap_or(0),
    })
}

fn convert_with_pandoc(src: &Path, dest: &Path, target: &str) -> Result<(), String> {
    let fmt = match target {
        "md" => "markdown",
        "txt" => "plain",
        other => other,
    };
    let out = Command::new("pandoc")
        .arg(src)
        .arg("-t")
        .arg(fmt)
        .arg("-o")
        .arg(dest)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(stderr_snip(&out));
    }
    if !dest.exists() {
        return Err("pandoc reported success but produced no output file".to_string());
    }
    Ok(())
}

fn convert_with_soffice(src: &Path, dest: &Path, target: &str) -> Result<(), String> {
    let outdir = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let out = Command::new("soffice")
        .arg("--headless")
        .arg("--convert-to")
        .arg(target)
        .arg("--outdir")
        .arg(&outdir)
        .arg(src)
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(stderr_snip(&out));
    }
    let stem = src.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let produced = outdir.join(format!("{}.{}", stem, target));
    if produced != dest {
        if produced.exists() {
            fs::rename(&produced, dest).map_err(|e| e.to_string())?;
        } else if !dest.exists() {
            return Err(format!(
                "LibreOffice did not produce the expected file ({})",
                produced.display()
            ));
        }
    }
    Ok(())
}

fn markdown_to_html(md: &str) -> String {
    let mut out = String::new();
    for para in md.split("\n\n") {
        let p = para.trim();
        if p.is_empty() {
            continue;
        }
        let (tag, rest) = if let Some(r) = p.strip_prefix("### ") {
            ("h3", r)
        } else if let Some(r) = p.strip_prefix("## ") {
            ("h2", r)
        } else if let Some(r) = p.strip_prefix("# ") {
            ("h1", r)
        } else {
            ("", p)
        };
        if !tag.is_empty() {
            out.push_str(&format!("<{tag}>{}</{tag}>\n", inline_md(&escape_html(rest))));
            continue;
        }
        let lines: Vec<&str> = p.lines().collect();
        if !lines.is_empty()
            && lines
                .iter()
                .all(|l| l.trim_start().starts_with("- ") || l.trim_start().starts_with("* "))
        {
            out.push_str("<ul>\n");
            for l in lines {
                let item = l
                    .trim_start()
                    .strip_prefix("- ")
                    .or_else(|| l.trim_start().strip_prefix("* "))
                    .unwrap_or("");
                out.push_str(&format!("<li>{}</li>\n", inline_md(&escape_html(item))));
            }
            out.push_str("</ul>\n");
        } else {
            let body = escape_html(p).replace('\n', "<br/>\n");
            out.push_str(&format!("<p>{}</p>\n", inline_md(&body)));
        }
    }
    out
}

fn inline_md(s: &str) -> String {
    let bold = regex::Regex::new(r"\*\*(.+?)\*\*").unwrap();
    let s = bold.replace_all(s, "<strong>$1</strong>").to_string();
    let em = regex::Regex::new(r"\*(.+?)\*").unwrap();
    let s = em.replace_all(&s, "<em>$1</em>").to_string();
    let link = regex::Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").unwrap();
    link.replace_all(&s, "<a href=\"$2\">$1</a>").to_string()
}

fn text_to_html(text: &str) -> String {
    text.split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .map(|p| format!("<p>{}</p>", escape_html(p).replace('\n', "<br/>\n")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ---------------------------------------------------------------------------
// OCR
// ---------------------------------------------------------------------------

pub fn ocr(path: &str, language: Option<&str>) -> Result<Value, String> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err(format!("file not found: {}", path));
    }
    if !binary_exists("tesseract") {
        return Err(
            "OCR needs the Tesseract engine. Install it (e.g. `sudo apt install tesseract-ocr`) and retry."
                .to_string(),
        );
    }
    let lang = language.unwrap_or("eng");
    let name = p.file_name().and_then(|s| s.to_str()).unwrap_or(path);

    if ext_of(path) == "pdf" {
        if !binary_exists("pdftoppm") {
            return Err(
                "OCR on PDFs needs the Poppler utilities to rasterize pages. Install them (e.g. `sudo apt install poppler-utils`) and retry."
                    .to_string(),
            );
        }
        let tmp = std::env::temp_dir().join(format!("gfa-ocr-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let page_limit: u32 = 10;
        let status = Command::new("pdftoppm")
            .arg("-png")
            .arg("-r")
            .arg("200")
            .arg("-l")
            .arg(page_limit.to_string())
            .arg(p)
            .arg(tmp.join("page"))
            .status()
            .map_err(|e| e.to_string())?;
        if !status.success() {
            let _ = fs::remove_dir_all(&tmp);
            return Err("pdftoppm failed to rasterize the PDF.".to_string());
        }
        let mut pages: Vec<PathBuf> = fs::read_dir(&tmp)
            .map_err(|e| e.to_string())?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|q| ext_of(&q.to_string_lossy()) == "png")
            .collect();
        pages.sort();
        let mut out = String::new();
        let mut done = 0usize;
        for (i, page) in pages.iter().enumerate() {
            let o = Command::new("tesseract")
                .arg(page)
                .arg("-")
                .arg("-l")
                .arg(lang)
                .output()
                .map_err(|e| e.to_string())?;
            if o.status.success() {
                out.push_str(&format!("--- Page {} ---\n", i + 1));
                out.push_str(&String::from_utf8_lossy(&o.stdout));
                out.push('\n');
                done += 1;
            }
        }
        let _ = fs::remove_dir_all(&tmp);
        let (capped, truncated) = cap_text(&out, EXTRACT_CAP);
        return Ok(json!({
            "file": name,
            "engine": "tesseract",
            "pages": done,
            "page_limit": page_limit,
            "truncated": truncated,
            "text": capped,
        }));
    }

    let o = Command::new("tesseract")
        .arg(p)
        .arg("-")
        .arg("-l")
        .arg(lang)
        .output()
        .map_err(|e| e.to_string())?;
    if !o.status.success() {
        return Err(format!("tesseract failed: {}", stderr_snip(&o)));
    }
    let text = String::from_utf8_lossy(&o.stdout).to_string();
    let (capped, truncated) = cap_text(&text, EXTRACT_CAP);
    Ok(json!({
        "file": name,
        "engine": "tesseract",
        "truncated": truncated,
        "text": capped,
    }))
}

// ---------------------------------------------------------------------------
// Tooling helpers
// ---------------------------------------------------------------------------

fn binary_exists(bin: &str) -> bool {
    for flag in ["--version", "-v"] {
        let ok = Command::new(bin)
            .arg(flag)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            return true;
        }
    }
    false
}

fn stderr_snip(out: &Output) -> String {
    let e = String::from_utf8_lossy(&out.stderr);
    let e = e.trim();
    if e.is_empty() {
        format!("exit code {:?}", out.status.code())
    } else {
        e.chars().take(300).collect()
    }
}

// ---------------------------------------------------------------------------
// Upload intake commands (chat attach pipeline)
// ---------------------------------------------------------------------------

fn extract_bytes_for_upload(name: &str, bytes: &[u8]) -> Result<Value, String> {
    let ext = ext_of(name);
    let safe_ext = if ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        ext.clone()
    } else {
        "bin".to_string()
    };
    let tmp = std::env::temp_dir().join(format!(
        "gfa-upload-{}.{}",
        uuid::Uuid::new_v4(),
        safe_ext
    ));
    fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    let result = if is_image_ext(&ext) {
        Ok(String::new())
    } else {
        extract_file(&tmp, &ext)
    };
    let _ = fs::remove_file(&tmp);
    let text = result?;
    let (capped, truncated) = cap_text(&text, UPLOAD_CAP);
    Ok(json!({
        "name": name,
        "format": if ext.is_empty() { "unknown".to_string() } else { ext },
        "bytes": bytes.len(),
        "chars": text.len(),
        "truncated": truncated,
        "text": capped,
    }))
}

/// Extract readable text from an uploaded file (base64 payload) so the model
/// can actually read it. Images return an empty text plus a note to use OCR.
#[tauri::command]
pub async fn document_extract_upload(name: String, data_b64: String) -> Result<Value, String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64)
        .map_err(|e| format!("bad base64 payload: {}", e))?;
    tauri::async_runtime::spawn_blocking(move || extract_bytes_for_upload(&name, &bytes))
        .await
        .map_err(|e| e.to_string())?
}

/// Persist an uploaded file (images/documents the user attached) under
/// app-data/media/uploads so it survives restarts and can be rendered.
#[tauri::command]
pub fn upload_save_b64(
    app: tauri::AppHandle,
    name: String,
    data_b64: String,
) -> Result<Value, String> {
    use tauri::Manager;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_b64)
        .map_err(|e| format!("bad base64 payload: {}", e))?;
    if bytes.len() > 25 * 1024 * 1024 {
        return Err("upload too large (max 25 MB per file)".to_string());
    }
    let ext = ext_of(&name);
    let safe_ext = if ext.len() <= 8 && ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        ext
    } else {
        "bin".to_string()
    };
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("media")
        .join("uploads");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let fname = format!(
        "{}-{}.{}",
        chrono::Utc::now().format("%Y%m%d%H%M%S"),
        &uuid::Uuid::new_v4().to_string()[..8],
        safe_ext
    );
    let path = dir.join(&fname);
    fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    Ok(json!({
        "path": path.to_string_lossy(),
        "name": name,
        "bytes": bytes.len(),
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtf_extraction_keeps_text_and_drops_control_words() {
        let rtf = r"{\rtf1\ansi Hello \b bold\b0  world\par second line\'41}";
        let text = strip_rtf(rtf);
        assert!(text.contains("Hello"));
        assert!(text.contains("bold"));
        assert!(text.contains("second lineA"));
        assert!(!text.contains("rtf1"));
    }

    #[test]
    fn text_events_extracts_docx_paragraphs() {
        let xml = r#"<w:document xmlns:w="x"><w:body><w:p><w:r><w:t>First</w:t></w:r></w:p><w:p><w:r><w:t xml:space="preserve"> Second</w:t></w:r></w:p></w:body></w:document>"#;
        let text = text_events(xml, &[b"w:p"], &[b"w:tab"], &[b"w:instrText"]).unwrap();
        assert!(text.contains("First"));
        assert!(text.contains("Second"));
        assert!(text.find("First").unwrap() < text.find("Second").unwrap());
    }

    #[test]
    fn cap_text_respects_utf8_boundaries() {
        let s = "é".repeat(10);
        let (capped, truncated) = cap_text(&s, 5);
        assert!(truncated);
        assert!(capped.starts_with('é'));
        assert!(capped.len() <= 5 + 4); // 5 bytes max + ellipsis
    }

    #[test]
    fn markdown_roundtrip_basics() {
        let html = markdown_to_html("# Title\n\nSome **bold** text with a [link](https://x.dev).\n\n- one\n- two");
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<a href=\"https://x.dev\">link</a>"));
        assert!(html.contains("<li>one</li>"));
    }

    #[test]
    fn compare_classifies_added_and_removed() {
        let dir = std::env::temp_dir().join(format!("gfa-cmp-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        fs::write(&a, "alpha\nbeta\ngamma\n").unwrap();
        fs::write(&b, "alpha\ndelta\ngamma\n").unwrap();
        let out = compare(a.to_str().unwrap(), b.to_str().unwrap(), Some("unified")).unwrap();
        assert_eq!(out["removed_total"], 1);
        assert_eq!(out["added_total"], 1);
        assert!(out["diff"].as_str().unwrap().contains("- beta"));
        assert!(out["diff"].as_str().unwrap().contains("+ delta"));
        let _ = fs::remove_dir_all(&dir);
    }
}
