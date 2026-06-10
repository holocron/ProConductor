//! Log syntax highlighting — span-based pipeline (inspired by tailspin) —
//! and the per-line LayoutJob builder for the log viewer.

use eframe::egui::{self, Color32};
use crate::process::{LogLine, Source};
use crate::theme::{AMBER, BLUE, RED, TEXT_DIM, TEXT_MUTED, TEXT_PRI};

// ══════════════════════════════════════════════════════════════════════════════
// Log highlighting — tailspin-inspired span pipeline
// Finders produce (start, end, Color32, priority) spans on byte offsets.
// Lower priority wins on overlap. Rendered via egui LayoutJob.
// ══════════════════════════════════════════════════════════════════════════════

#[derive(Clone, Copy)]
struct HSpan { start: usize, end: usize, color: Color32, bold: bool, priority: u8 }

// ── Highlight colours ─────────────────────────────────────────────────────────
const HL_NUMBER:  Color32 = Color32::from_rgb( 86, 210, 255);  // cyan
const HL_STRING:  Color32 = Color32::from_rgb(152, 220,  90);  // green
const HL_URL:     Color32 = Color32::from_rgb( 86, 180, 255);  // light blue
const _HL_KEY:    Color32 = Color32::from_rgb(140, 170, 220);  // steel blue
const HL_UUID:    Color32 = Color32::from_rgb(190, 140, 255);  // purple
const HL_IP:      Color32 = Color32::from_rgb(240, 175,  60);  // amber
const HL_PATH:    Color32 = Color32::from_rgb(170, 170, 170);  // grey
const HL_HTTP_OK: Color32 = Color32::from_rgb( 33, 212, 126);  // green
const HL_HTTP_RD: Color32 = Color32::from_rgb( 86, 210, 255);  // cyan
const HL_HTTP_CL: Color32 = Color32::from_rgb(240, 175,  60);  // amber
const HL_HTTP_ER: Color32 = Color32::from_rgb(240,  74,  94);  // red
const HL_METHOD:  Color32 = Color32::from_rgb( 68, 136, 255);  // blue
const HL_ERR_LVL: Color32 = Color32::from_rgb(240,  74,  94);  // red
const HL_WRN_LVL: Color32 = Color32::from_rgb(240, 175,  60);  // amber
const HL_INF_LVL: Color32 = Color32::from_rgb( 68, 136, 255);  // blue
const HL_DBG_LVL: Color32 = Color32::from_rgb(100, 130, 160);  // dim

fn is_word_boundary(b: &[u8], start: usize, end: usize) -> bool {
    let lb = if start == 0 { true } else { !b[start-1].is_ascii_alphanumeric() && b[start-1] != b'_' };
    let rb = end >= b.len() || (!b[end].is_ascii_alphanumeric() && b[end] != b'_');
    lb && rb
}

fn push(spans: &mut Vec<HSpan>, start: usize, end: usize, color: Color32, bold: bool, prio: u8) {
    if start < end { spans.push(HSpan { start, end, color, bold, priority: prio }); }
}

// ── Numbers ───────────────────────────────────────────────────────────────────
fn find_numbers(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            // Make sure left boundary is not alphanumeric/underscore
            let lb = i == 0 || (!b[i-1].is_ascii_alphanumeric() && b[i-1] != b'_');
            if lb {
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() { i += 1; }
                // Optional decimal
                if i + 1 < b.len() && b[i] == b'.' && b[i+1].is_ascii_digit() {
                    i += 1;
                    while i < b.len() && b[i].is_ascii_digit() { i += 1; }
                }
                // Right boundary
                let rb = i >= b.len() || (!b[i].is_ascii_alphanumeric() && b[i] != b'_');
                if rb { push(spans, start, i, HL_NUMBER, false, 50); }
                continue;
            }
        }
        i += 1;
    }
}

// ── Quoted strings ────────────────────────────────────────────────────────────
fn find_quoted(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    for quote in [b'"', b'\''] {
        let mut open: Option<usize> = None;
        for (i, &c) in b.iter().enumerate() {
            if c != quote { continue; }
            match open {
                // An opening quote must not follow an alphanumeric character —
                // this keeps apostrophes in contractions ("don't") from pairing.
                None => {
                    if i == 0 || !b[i - 1].is_ascii_alphanumeric() {
                        open = Some(i);
                    }
                }
                Some(s) => {
                    push(spans, s, i + 1, HL_STRING, false, 40);
                    open = None;
                }
            }
        }
    }
}

// ── Log levels ────────────────────────────────────────────────────────────────
fn find_log_levels(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let levels: &[(&[u8], Color32, bool)] = &[
        (b"CRITICAL", HL_ERR_LVL, true), (b"FATAL",    HL_ERR_LVL, true),
        (b"ERROR",    HL_ERR_LVL, true), (b"ERR",      HL_ERR_LVL, false),
        (b"WARNING",  HL_WRN_LVL, true), (b"WARN",     HL_WRN_LVL, false),
        (b"INFO",     HL_INF_LVL, false),
        (b"DEBUG",    HL_DBG_LVL, false), (b"DBG",     HL_DBG_LVL, false),
        (b"TRACE",    HL_DBG_LVL, false),
    ];
    for &(kw, color, bold) in levels {
        let mut i = 0;
        while i + kw.len() <= b.len() {
            if b[i..].starts_with(kw) && is_word_boundary(b, i, i + kw.len()) {
                push(spans, i, i + kw.len(), color, bold, 5);
            }
            i += 1;
        }
    }
}

// ── HTTP methods ──────────────────────────────────────────────────────────────
fn find_http_methods(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    for kw in [b"GET".as_ref(), b"POST", b"PUT", b"DELETE", b"PATCH", b"HEAD", b"OPTIONS"] {
        let mut i = 0;
        while i + kw.len() <= b.len() {
            if b[i..].starts_with(kw) && is_word_boundary(b, i, i + kw.len()) {
                push(spans, i, i + kw.len(), HL_METHOD, false, 10);
            }
            i += 1;
        }
    }
}

// ── HTTP status codes ─────────────────────────────────────────────────────────
fn find_http_status(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i + 3 <= b.len() {
        if b[i].is_ascii_digit() && b[i+1].is_ascii_digit() && b[i+2].is_ascii_digit() {
            let lb = i == 0 || b[i-1] == b' ' || b[i-1] == b'"';
            let rb = i+3 >= b.len() || b[i+3] == b' ' || b[i+3] == b'"' || b[i+3] == b'\r';
            if lb && rb {
                let code = (b[i]-b'0') as u16 * 100
                         + (b[i+1]-b'0') as u16 * 10
                         + (b[i+2]-b'0') as u16;
                let color = match code {
                    200..=299 => HL_HTTP_OK,
                    300..=399 => HL_HTTP_RD,
                    400..=499 => HL_HTTP_CL,
                    500..=599 => HL_HTTP_ER,
                    _ => { i += 1; continue; }
                };
                push(spans, i, i+3, color, false, 8);
            }
        }
        i += 1;
    }
}

// ── URLs ──────────────────────────────────────────────────────────────────────
fn find_urls(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i + 8 <= b.len() {
        let is_http  = b[i..].starts_with(b"http://");
        let is_https = b[i..].starts_with(b"https://");
        if is_http || is_https {
            let start = i;
            i += if is_https { 8 } else { 7 };
            while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'"' && b[i] != b'\'' { i += 1; }
            push(spans, start, i, HL_URL, false, 20);
            continue;
        }
        i += 1;
    }
}

// ── UUIDs ─────────────────────────────────────────────────────────────────────
fn find_uuids(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    fn is_hex(c: u8) -> bool { c.is_ascii_hexdigit() }
    if b.len() < 36 { return; }
    let mut i = 0;
    while i + 36 <= b.len() {
        // 8-4-4-4-12
        let ok = (0..8).all(|j| is_hex(b[i+j]))
            && b[i+8] == b'-'
            && (0..4).all(|j| is_hex(b[i+9+j]))
            && b[i+13] == b'-'
            && (0..4).all(|j| is_hex(b[i+14+j]))
            && b[i+18] == b'-'
            && (0..4).all(|j| is_hex(b[i+19+j]))
            && b[i+23] == b'-'
            && (0..12).all(|j| is_hex(b[i+24+j]));
        if ok && (i == 0 || !is_hex(b[i-1])) && (i+36 >= b.len() || !is_hex(b[i+36])) {
            push(spans, i, i+36, HL_UUID, false, 15);
            i += 36;
            continue;
        }
        i += 1;
    }
}

// ── IP addresses ──────────────────────────────────────────────────────────────
fn find_ips(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            let lb = i == 0 || !b[i-1].is_ascii_digit();
            if lb {
                // Try to match d{1,3}.d{1,3}.d{1,3}.d{1,3}
                let start = i;
                let mut j = i;
                let mut valid = true;
                for seg in 0..4 {
                    let seg_start = j;
                    while j < b.len() && b[j].is_ascii_digit() { j += 1; }
                    let seg_len = j - seg_start;
                    if seg_len == 0 || seg_len > 3 { valid = false; break; }
                    // Parse octet value
                    let val: u32 = text[seg_start..j].parse().unwrap_or(999);
                    if val > 255 { valid = false; break; }
                    if seg < 3 {
                        if j >= b.len() || b[j] != b'.' { valid = false; break; }
                        j += 1; // skip dot
                    }
                }
                if valid && (j >= b.len() || !b[j].is_ascii_digit()) {
                    push(spans, start, j, HL_IP, false, 12);
                    i = j;
                    continue;
                }
            }
        }
        i += 1;
    }
}

// ── Unix paths ────────────────────────────────────────────────────────────────
fn find_paths(text: &str, spans: &mut Vec<HSpan>) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && (i == 0 || b[i-1] == b' ' || b[i-1] == b'"' || b[i-1] == b'\'') {
            let start = i;
            while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'"' && b[i] != b'\'' { i += 1; }
            if i - start > 1 { push(spans, start, i, HL_PATH, false, 30); }
            continue;
        }
        i += 1;
    }
}

// ── Merge: lower priority wins on overlap ────────────────────────────────────
fn merge_spans_hl(text_len: usize, spans: Vec<HSpan>) -> Vec<HSpan> {
    if spans.is_empty() || text_len == 0 { return vec![]; }
    // Per-byte index into spans vec (using index+1 so 0 = unset)
    let mut map: Vec<Option<usize>> = vec![None; text_len];
    for (idx, s) in spans.iter().enumerate() {
        let end = s.end.min(text_len);
        for slot in &mut map[s.start..end] {
            match slot {
                None => *slot = Some(idx),
                Some(existing) if s.priority < spans[*existing].priority => *slot = Some(idx),
                _ => {}
            }
        }
    }
    // Run-length encode into output spans
    let mut out: Vec<HSpan> = Vec::new();
    let mut i = 0;
    while i < text_len {
        if let Some(idx) = map[i] {
            let s = &spans[idx];
            let start = i;
            while i < text_len && map[i] == Some(idx) { i += 1; }
            out.push(HSpan { start, end: i, color: s.color, bold: s.bold, priority: s.priority });
        } else { i += 1; }
    }
    out
}

// ── Build a LayoutJob for one log line ───────────────────────────────────────
fn highlight_log_line(
    text:     &str,
    base_color: Color32,
    font_id:  &egui::FontId,
) -> egui::text::LayoutJob {
    let mut spans: Vec<HSpan> = Vec::new();
    find_log_levels(text, &mut spans);
    find_http_status(text, &mut spans);
    find_http_methods(text, &mut spans);
    find_uuids(text, &mut spans);
    find_ips(text, &mut spans);
    find_urls(text, &mut spans);
    find_paths(text, &mut spans);
    find_quoted(text, &mut spans);
    find_numbers(text, &mut spans);

    let resolved = merge_spans_hl(text.len(), spans);

    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = f32::INFINITY; // no word wrap — horizontal scroll handles it

    let plain_fmt = |color: Color32, _bold: bool| egui::text::TextFormat {
        font_id: font_id.clone(),
        color,
        background: Color32::TRANSPARENT,
        italics: false,
        underline: egui::Stroke::NONE,
        strikethrough: egui::Stroke::NONE,
        valign: egui::Align::BOTTOM,
        ..Default::default()
    };

    let mut pos = 0usize;
    for s in &resolved {
        if s.start > pos {
            job.append(&text[pos..s.start], 0.0, plain_fmt(base_color, false));
        }
        job.append(&text[s.start..s.end], 0.0, plain_fmt(s.color, s.bold));
        pos = s.end;
    }
    if pos < text.len() {
        job.append(&text[pos..], 0.0, plain_fmt(base_color, false));
    }
    if job.sections.is_empty() {
        job.append(text, 0.0, plain_fmt(base_color, false));
    }
    job
}

/// Lines that actually carry error vocabulary — independent of which stream
/// they arrived on. Many healthy servers write access logs to stderr.
fn looks_like_error(text: &str) -> bool {
    let l = text.to_lowercase();
    l.contains("error") || l.contains("exception") || l.contains("traceback")
        || l.contains("fatal") || l.contains("panic") || l.contains("critical")
        || l.starts_with("  file \"")
}

/// Build the full row for one log line: timestamp + stream tag + highlighted
/// text, as ONE LayoutJob. Using ui.horizontal() would split the row into
/// multiple widgets which egui clips to available_width, breaking h-scroll.
///
/// Stream ≠ severity: plain stderr gets a muted lowercase "err" tag; the loud
/// red "ERR" is reserved for lines that actually look like errors.
pub(crate) fn log_line_job(line: &LogLine, font_id: &egui::FontId) -> egui::text::LayoutJob {
    let is_real_error = looks_like_error(&line.text);
    let base_color = if is_real_error { Color32::from_rgb(240, 120, 130) } else { TEXT_PRI };

    let (src_text, src_color) = match line.source {
        Source::Stdout => ("OUT", BLUE),
        Source::Stderr => if is_real_error { ("ERR", RED) } else { ("err", TEXT_MUTED) },
        Source::System => ("SYS", AMBER),
    };

    let dim_fmt = egui::text::TextFormat { font_id: font_id.clone(), color: TEXT_DIM, ..Default::default() };
    let src_fmt = egui::text::TextFormat { font_id: font_id.clone(), color: src_color, ..Default::default() };

    let mut job = highlight_log_line(&line.text, base_color, font_id);
    let text_part = std::mem::take(&mut job.text);
    let sections  = std::mem::take(&mut job.sections);
    let mut full_job = egui::text::LayoutJob::default();
    full_job.wrap.max_width = f32::INFINITY;
    full_job.append(&line.time, 0.0, dim_fmt.clone());
    full_job.append("  ", 0.0, dim_fmt.clone());
    full_job.append(src_text, 0.0, src_fmt);
    full_job.append(" ", 0.0, dim_fmt);
    // Re-add the highlighted text sections at the shifted offset
    let offset = full_job.text.len();
    full_job.text.push_str(&text_part);
    for mut s in sections {
        s.byte_range.start += offset;
        s.byte_range.end   += offset;
        full_job.sections.push(s);
    }
    full_job
}
