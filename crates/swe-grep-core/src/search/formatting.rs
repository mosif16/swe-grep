use std::fmt::Write as _;
use std::path::Path;

use serde::Serialize;

use crate::file_cache::FileCache;

pub(crate) const HINT_KEYWORD_LIMIT: usize = 32;

#[derive(Clone, Serialize)]
pub struct ContextHint {
    pub kind: String,
    pub label: String,
    pub line: usize,
}

pub(crate) fn gather_expanded_snippet(
    root: &Path,
    file_cache: &FileCache,
    path: &Path,
    line: usize,
    before: usize,
    after: usize,
) -> Option<(String, usize, usize)> {
    if line == 0 {
        return None;
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let contents = file_cache
        .read_file(&absolute)
        .map(|f| f.contents().to_string())?;
    let lines: Vec<&str> = contents.lines().collect();

    if lines.is_empty() || line > lines.len() {
        return None;
    }

    let before_cap = before.min(line - 1);
    let start = line - before_cap;
    let remaining_after = lines.len() - line;
    let after_cap = after.min(remaining_after);
    let end = line + after_cap;
    let width = end.to_string().len().max(1);

    let mut buffer = String::new();
    for idx in start..=end {
        let text = lines.get(idx - 1).copied().unwrap_or_default();
        if writeln!(&mut buffer, "{:0width$} {}", idx, text, width = width).is_err() {
            return None;
        }
    }

    Some((buffer, start, end))
}

pub(crate) fn rust_context_hints(contents: &str, line: usize) -> Vec<ContextHint> {
    let lines: Vec<&str> = contents.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let idx = line.saturating_sub(1).min(lines.len() - 1);

    let declaration = find_rust_declaration(&lines, idx);
    let anchor = declaration.as_ref().map(|(idx, _)| *idx).unwrap_or(idx);

    let mut hints: Vec<ContextHint> = Vec::new();
    collect_rust_attributes(&lines, anchor, &mut hints);

    if let Some((decl_idx, decl_label)) = declaration.clone() {
        push_hint(&mut hints, "declaration", decl_label, decl_idx + 1);
    }

    if let Some((impl_idx, impl_label)) =
        find_rust_impl(&lines, anchor, declaration.as_ref().map(|(idx, _)| *idx))
    {
        push_hint(&mut hints, "impl", impl_label, impl_idx + 1);
    }

    collect_rust_modules(&lines, anchor, &mut hints);

    finalize_hints(hints)
}

pub(crate) fn swift_context_hints(contents: &str, line: usize) -> Vec<ContextHint> {
    let lines: Vec<&str> = contents.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let idx = line.saturating_sub(1).min(lines.len() - 1);

    let signature = find_swift_signature(&lines, idx);
    let anchor = signature.as_ref().map(|(idx, _)| *idx).unwrap_or(idx);

    let mut hints: Vec<ContextHint> = Vec::new();
    collect_swift_attributes(&lines, anchor, &mut hints);

    if let Some((type_idx, type_label, kind)) = find_swift_type(&lines, anchor) {
        push_hint(&mut hints, kind, type_label, type_idx + 1);
    }

    if let Some((sig_idx, sig_label)) = signature {
        push_hint(&mut hints, "declaration", sig_label, sig_idx + 1);
    }

    finalize_hints(hints)
}

pub(crate) fn format_snippet(
    root: &Path,
    file_cache: &FileCache,
    path: &Path,
    line: usize,
    raw: &str,
) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    match ext.as_deref() {
        Some("swift") => format_swift_snippet(root, file_cache, path, line, raw),
        Some("ts") | Some("tsx") => format_typescript_snippet(raw),
        _ => format_default_snippet(raw),
    }
}

fn format_swift_snippet(
    root: &Path,
    file_cache: &FileCache,
    path: &Path,
    line: usize,
    raw: &str,
) -> Option<String> {
    let trimmed: Vec<String> = raw.lines().map(|entry| entry.trim().to_string()).collect();

    let mut attributes_rev: Vec<String> = Vec::new();

    let mut candidate_idx: Option<usize> = None;
    for (idx, entry) in trimmed.iter().enumerate() {
        if entry.is_empty() || entry.starts_with("//") {
            continue;
        }
        if entry.starts_with("func ")
            || entry.starts_with("protocol ")
            || entry.starts_with("extension ")
            || entry.starts_with("struct ")
            || entry.starts_with("class ")
            || entry.starts_with("actor ")
            || entry.starts_with("init(")
            || entry.starts_with("init ")
            || entry.starts_with("enum ")
        {
            candidate_idx = Some(idx);
            break;
        }
    }

    let (selected_idx, _selected) = if let Some(idx) = candidate_idx {
        (idx, trimmed[idx].clone())
    } else {
        trimmed.iter().enumerate().find_map(|(idx, entry)| {
            if entry.is_empty() {
                None
            } else {
                Some((idx, entry.clone()))
            }
        })?
    };

    let mut signature_segments = vec![trimmed[selected_idx].clone()];
    for entry in trimmed.iter().skip(selected_idx + 1) {
        let trimmed_entry = entry.trim();
        if trimmed_entry.is_empty() {
            break;
        }
        if trimmed_entry.starts_with('@') {
            let attr = collapse_whitespace(trimmed_entry);
            if !attributes_rev.iter().any(|existing| existing == &attr) {
                attributes_rev.push(attr);
            }
            continue;
        }
        if trimmed_entry.starts_with('}') {
            break;
        }
        if trimmed_entry.starts_with(")")
            || trimmed_entry.starts_with("async")
            || trimmed_entry.starts_with("throws")
            || trimmed_entry.starts_with("rethrows")
            || trimmed_entry.starts_with("->")
            || trimmed_entry.starts_with("where ")
            || trimmed_entry.starts_with("some ")
        {
            signature_segments.push(trimmed_entry.to_string());
            continue;
        }
        break;
    }

    let collapsed_signature = collapse_whitespace(&signature_segments.join(" "));
    let mut formatted = collapsed_signature.clone();
    let lowered_sig = collapsed_signature.to_ascii_lowercase();
    if collapsed_signature.contains("async") {
        formatted.push_str(" [async]");
    }
    if lowered_sig.contains("await ") {
        formatted.push_str(" [await]");
    }
    for access in ["public", "internal", "private", "fileprivate", "open"].iter() {
        if lowered_sig.starts_with(access)
            || lowered_sig.contains(&format!(" {access} "))
            || lowered_sig.contains(&format!(" {access}("))
        {
            formatted.push_str(" [");
            formatted.push_str(access);
            formatted.push(']');
            break;
        }
    }
    if collapsed_signature.contains('<') && collapsed_signature.contains('>') {
        formatted.push_str(" [generic]");
    }

    let mut context: Option<String> = None;

    if selected_idx > 0 {
        for entry in trimmed[..selected_idx].iter().rev() {
            if entry.is_empty() {
                continue;
            }
            if entry.starts_with('@') {
                let attr = collapse_whitespace(entry);
                if !attributes_rev.iter().any(|existing| existing == &attr) {
                    attributes_rev.push(attr);
                }
                continue;
            }
            if entry.starts_with("extension ")
                || entry.starts_with("struct ")
                || entry.starts_with("class ")
                || entry.starts_with("protocol ")
                || entry.starts_with("actor ")
                || entry.starts_with("enum ")
            {
                context = Some(collapse_whitespace(entry));
            }
            break;
        }
    }

    if context.is_none() {
        let full_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        if let Some(contents) = file_cache
            .read_file(&full_path)
            .map(|f| f.contents().to_string())
        {
            let lines: Vec<&str> = contents.lines().collect();
            if !lines.is_empty() {
                let mut idx = line.saturating_sub(1);
                while idx > 0 {
                    idx -= 1;
                    if let Some(candidate) = lines.get(idx) {
                        let trimmed_candidate = candidate.trim();
                        if trimmed_candidate.is_empty() {
                            continue;
                        }
                        if trimmed_candidate.starts_with('@') {
                            let attr = collapse_whitespace(trimmed_candidate);
                            if !attributes_rev.iter().any(|existing| existing == &attr) {
                                attributes_rev.push(attr);
                            }
                            continue;
                        }
                        if trimmed_candidate.starts_with("extension ")
                            || trimmed_candidate.starts_with("struct ")
                            || trimmed_candidate.starts_with("class ")
                            || trimmed_candidate.starts_with("protocol ")
                            || trimmed_candidate.starts_with("actor ")
                            || trimmed_candidate.starts_with("enum ")
                        {
                            context = Some(collapse_whitespace(trimmed_candidate));
                            break;
                        }
                        if trimmed_candidate.starts_with("func ")
                            || trimmed_candidate.starts_with("init")
                            || trimmed_candidate.starts_with("let ")
                            || trimmed_candidate.starts_with("var ")
                        {
                            break;
                        }
                    } else {
                        break;
                    }
                }
            }
        }
    }

    attributes_rev.reverse();
    if let Some(ctx) = context {
        formatted = format!("{ctx} :: {formatted}");
    }
    for attr in attributes_rev {
        formatted.push_str(" [");
        formatted.push_str(&attr);
        formatted.push(']');
    }

    Some(formatted)
}

fn format_typescript_snippet(raw: &str) -> Option<String> {
    let mut candidate: Option<String> = None;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        if trimmed.starts_with("<") || trimmed.starts_with("</") {
            return Some(collapse_whitespace(trimmed));
        }
        if trimmed.contains('<') && trimmed.contains('>') {
            candidate = Some(trimmed.to_string());
            break;
        }
        if trimmed.starts_with("export")
            || trimmed.starts_with("type ")
            || trimmed.starts_with("interface ")
            || trimmed.contains("=>")
        {
            candidate = Some(trimmed.to_string());
        }
    }

    let selected = candidate.or_else(|| {
        raw.lines()
            .map(|line| line.trim().to_string())
            .find(|line| !line.is_empty())
    })?;

    let mut formatted = collapse_whitespace(&selected);
    if selected.contains("async") {
        formatted.push_str(" [async]");
    }
    let lowered = selected.trim_start();
    if lowered.starts_with("use") || lowered.contains(" = use") {
        formatted.push_str(" [hook]");
    }
    if lowered.contains("React.FC") || lowered.contains("React.FunctionComponent") {
        formatted.push_str(" [component]");
    }
    if lowered.contains("React.forwardRef") || lowered.contains("React.memo") {
        formatted.push_str(" [component]");
    }
    if lowered.contains("Promise<") {
        formatted.push_str(" [promise]");
    }
    if lowered.contains("=>") {
        formatted.push_str(" [arrow]");
    }
    if lowered.contains("await ") {
        formatted.push_str(" [await]");
    }
    if selected.contains('<') && selected.contains('>') {
        formatted.push_str(" [generic]");
    }
    if lowered.contains("satisfies ") {
        formatted.push_str(" [satisfies]");
    }
    Some(formatted)
}

fn format_default_snippet(raw: &str) -> Option<String> {
    raw.lines()
        .map(|line| line.trim())
        .find(|line| !line.is_empty())
        .map(collapse_whitespace)
}

fn push_hint(target: &mut Vec<ContextHint>, kind: &str, label: String, line: usize) {
    if label.is_empty() {
        return;
    }
    if target
        .iter()
        .any(|hint| hint.kind == kind && hint.label == label && hint.line == line)
    {
        return;
    }
    target.push(ContextHint {
        kind: kind.to_string(),
        label,
        line,
    });
}

fn finalize_hints(mut hints: Vec<ContextHint>) -> Vec<ContextHint> {
    hints.sort_by_key(|hint| hint.line);
    hints.dedup_by(|a, b| a.line == b.line && a.kind == b.kind && a.label == b.label);
    hints
}

fn keyword_near_start(text: &str, keyword: &str) -> bool {
    if keyword.is_empty() {
        return false;
    }

    let bytes = text.as_bytes();
    let needle = keyword.as_bytes();
    let first_requires_boundary = !keyword
        .chars()
        .next()
        .map(|ch| ch.is_whitespace())
        .unwrap_or(false);
    let last_requires_boundary = !keyword
        .chars()
        .last()
        .map(|ch| ch.is_whitespace())
        .unwrap_or(false);

    let limit = bytes.len().min(HINT_KEYWORD_LIMIT + needle.len());
    let mut idx = 0;
    while idx + needle.len() <= limit {
        if &bytes[idx..idx + needle.len()] == needle {
            let before_ok = if first_requires_boundary {
                idx == 0 || !bytes[idx - 1].is_ascii_alphanumeric() && bytes[idx - 1] != b'_'
            } else {
                true
            };
            let after_pos = idx + needle.len();
            let after_ok = if last_requires_boundary {
                after_pos >= bytes.len()
                    || !bytes
                        .get(after_pos)
                        .map(|ch| ch.is_ascii_alphanumeric() || *ch == b'_')
                        .unwrap_or(false)
            } else {
                true
            };
            if before_ok && after_ok {
                return true;
            }
        }
        idx += 1;
    }
    false
}

fn collect_rust_attributes(lines: &[&str], anchor: usize, hints: &mut Vec<ContextHint>) {
    if lines.is_empty() || anchor == 0 {
        return;
    }

    let mut idx = anchor;
    while idx > 0 {
        idx -= 1;
        let trimmed = lines[idx].trim();
        if trimmed.starts_with("#[") {
            push_hint(hints, "attribute", collapse_whitespace(trimmed), idx + 1);
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        break;
    }
}

fn find_rust_declaration(lines: &[&str], start_idx: usize) -> Option<(usize, String)> {
    if lines.is_empty() {
        return None;
    }

    let mut idx = start_idx.min(lines.len() - 1);
    loop {
        let trimmed = lines[idx].trim();
        if trimmed.is_empty()
            || trimmed.starts_with("//")
            || trimmed.starts_with("#[")
            || trimmed.starts_with("/*")
            || trimmed.starts_with("*/")
        {
            if idx == 0 {
                return None;
            }
            idx -= 1;
            continue;
        }
        if trimmed.starts_with('}') {
            if idx == 0 {
                return None;
            }
            idx -= 1;
            continue;
        }
        if looks_like_rust_declaration(trimmed) {
            return Some((idx, collapse_whitespace(trimmed)));
        }
        if idx == 0 {
            break;
        }
        idx -= 1;
    }
    None
}

fn looks_like_rust_declaration(trimmed: &str) -> bool {
    if trimmed.starts_with("//") || trimmed.starts_with("#[") {
        return false;
    }
    let collapsed = collapse_whitespace(trimmed);
    let lower = collapsed.to_ascii_lowercase();

    lower.starts_with("fn ")
        || lower.starts_with("fn(")
        || lower.contains(" fn ")
        || lower.contains(" fn(")
        || lower.starts_with("struct ")
        || lower.contains(" struct ")
        || lower.starts_with("enum ")
        || lower.contains(" enum ")
        || lower.starts_with("trait ")
        || lower.contains(" trait ")
        || lower.starts_with("type ")
        || lower.contains(" type ")
        || lower.starts_with("const ")
        || lower.contains(" const ")
        || lower.starts_with("static ")
        || lower.contains(" static ")
        || lower.starts_with("macro_rules!")
        || lower.contains(" macro_rules!")
}

fn find_rust_impl(
    lines: &[&str],
    start_idx: usize,
    skip: Option<usize>,
) -> Option<(usize, String)> {
    if lines.is_empty() || start_idx == 0 {
        return None;
    }

    let mut idx = start_idx;
    while idx > 0 {
        idx -= 1;
        if Some(idx) == skip {
            continue;
        }
        let trimmed = lines[idx].trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with("#[") {
            continue;
        }
        if trimmed.starts_with('}') {
            continue;
        }
        if looks_like_rust_impl(trimmed) {
            return Some((idx, collapse_whitespace(trimmed)));
        }
    }
    None
}

fn looks_like_rust_impl(trimmed: &str) -> bool {
    let collapsed = collapse_whitespace(trimmed);
    let lower = collapsed.to_ascii_lowercase();
    lower.starts_with("impl<")
        || lower.starts_with("impl(")
        || keyword_near_start(&lower, "impl ")
        || keyword_near_start(&lower, "impl<")
        || keyword_near_start(&lower, "impl(")
}

fn collect_rust_modules(lines: &[&str], start_idx: usize, hints: &mut Vec<ContextHint>) {
    if lines.is_empty() {
        return;
    }

    let mut modules: Vec<ContextHint> = Vec::new();
    let mut idx = start_idx.min(lines.len() - 1);
    loop {
        let trimmed = lines[idx].trim();
        if looks_like_rust_module(trimmed) && indentation(lines[idx]) == 0 {
            push_hint(
                &mut modules,
                "module",
                collapse_whitespace(trimmed),
                idx + 1,
            );
            if modules.len() >= 3 {
                break;
            }
        }
        if idx == 0 {
            break;
        }
        idx -= 1;
    }
    modules.reverse();
    hints.extend(modules.into_iter());
}

fn looks_like_rust_module(trimmed: &str) -> bool {
    let collapsed = collapse_whitespace(trimmed);
    let lower = collapsed.to_ascii_lowercase();
    keyword_near_start(&lower, "mod ")
}

fn indentation(line: &str) -> usize {
    line.chars().take_while(|ch| ch.is_whitespace()).count()
}

fn collect_swift_attributes(lines: &[&str], anchor: usize, hints: &mut Vec<ContextHint>) {
    if lines.is_empty() || anchor == 0 {
        return;
    }

    let mut idx = anchor.min(lines.len() - 1);
    while idx > 0 {
        idx -= 1;
        let trimmed = lines[idx].trim();
        if trimmed.starts_with('@') {
            push_hint(hints, "attribute", collapse_whitespace(trimmed), idx + 1);
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        break;
    }
}

fn find_swift_signature(lines: &[&str], start_idx: usize) -> Option<(usize, String)> {
    if lines.is_empty() {
        return None;
    }

    let mut idx = start_idx.min(lines.len() - 1);
    loop {
        let trimmed = lines[idx].trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('@') {
            if idx == 0 {
                return None;
            }
            idx -= 1;
            continue;
        }
        if trimmed.starts_with('}') {
            if idx == 0 {
                return None;
            }
            idx -= 1;
            continue;
        }
        if looks_like_swift_signature(trimmed) {
            return Some((idx, collapse_whitespace(trimmed)));
        }
        if idx == 0 {
            break;
        }
        idx -= 1;
    }
    None
}

fn looks_like_swift_signature(trimmed: &str) -> bool {
    let collapsed = collapse_whitespace(trimmed);
    let lower = collapsed.to_ascii_lowercase();

    lower.starts_with("func ")
        || lower.starts_with("func(")
        || lower.contains(" func ")
        || lower.contains(" func(")
        || keyword_near_start(&lower, "init")
        || keyword_near_start(&lower, "var ")
        || keyword_near_start(&lower, "let ")
        || keyword_near_start(&lower, "case ")
        || lower.starts_with("subscript")
        || lower.contains(" subscript")
        || lower.starts_with("deinit")
        || lower.contains(" deinit")
}

fn find_swift_type(lines: &[&str], start_idx: usize) -> Option<(usize, String, &'static str)> {
    if lines.is_empty() {
        return None;
    }

    let mut idx = start_idx.min(lines.len() - 1);
    loop {
        let trimmed = lines[idx].trim();
        if trimmed.is_empty() || trimmed.starts_with("//") || trimmed.starts_with('@') {
            if idx == 0 {
                return None;
            }
            idx -= 1;
            continue;
        }
        if let Some(kind) = swift_type_kind(trimmed) {
            return Some((idx, collapse_whitespace(trimmed), kind));
        }
        if trimmed.starts_with('}') {
            if idx == 0 {
                return None;
            }
            idx -= 1;
            continue;
        }
        if idx == 0 {
            break;
        }
        idx -= 1;
    }
    None
}

fn swift_type_kind(trimmed: &str) -> Option<&'static str> {
    let collapsed = collapse_whitespace(trimmed);
    let lower = collapsed.to_ascii_lowercase();

    if keyword_near_start(&lower, "extension ") {
        Some("extension")
    } else if keyword_near_start(&lower, "struct ")
        || keyword_near_start(&lower, "class ")
        || keyword_near_start(&lower, "enum ")
        || keyword_near_start(&lower, "protocol ")
        || keyword_near_start(&lower, "actor ")
    {
        Some("type")
    } else {
        None
    }
}

pub(crate) fn collapse_whitespace(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut last_was_space = false;
    for ch in input.chars() {
        if ch.is_whitespace() {
            if !last_was_space {
                result.push(' ');
                last_was_space = true;
            }
        } else {
            result.push(ch);
            last_was_space = false;
        }
    }
    result.trim().to_string()
}
