//! Cleans a pandoc JSON AST before it is written as GitHub-Flavored Markdown.
//!
//! The Markdown is written with `-t gfm` (raw HTML allowed) so that tables too
//! complex for pipe syntax survive as HTML `<table>`s instead of `[TABLE]`.
//! [`normalize`] removes everything else the gfm writer would emit as HTML,
//! reproducing what `gfm-raw_html` used to do for those nodes.

use serde_json::{Value, json};

/// Rewrites `ast` in place so the only HTML pandoc's gfm writer can produce is
/// the fallback for complex tables:
/// - `RawBlock` / `RawInline` are dropped;
/// - Image and Link attributes are cleared (no `<img …>` / `<a …>`);
/// - `Figure`s become their content followed by the caption as a paragraph;
/// - `Span` / `Div` are replaced by their content;
/// - `Underline` becomes emphasis, `SmallCaps` upper-case text, and
///   `Superscript` / `Subscript` Unicode characters or `^(…)` / `_(…)`.
pub fn normalize(ast: &mut Value) {
    if let Some(blocks) = ast.get_mut("blocks") {
        normalize_value(blocks);
    }
}

fn normalize_value(v: &mut Value) {
    match v {
        Value::Array(items) => {
            let old = std::mem::take(items);
            *items = old.into_iter().flat_map(expand).collect();
        }
        Value::Object(o) => o.values_mut().for_each(normalize_value),
        _ => {}
    }
}

/// The normalised replacement for one element of a list (usually one node).
fn expand(mut v: Value) -> Vec<Value> {
    let Some(tag) = v.get("t").and_then(Value::as_str).map(str::to_owned) else {
        normalize_value(&mut v);
        return vec![v];
    };
    let c = v.get_mut("c").map(Value::take).unwrap_or(Value::Null);
    match tag.as_str() {
        "RawBlock" | "RawInline" => Vec::new(),
        // [attr, content]
        "Span" | "Div" => contents(c.get(1).cloned()),
        // [attr, [short caption, caption blocks], body blocks]
        "Figure" => {
            let mut out: Vec<Value> = contents(c.get(2).cloned())
                .into_iter()
                .map(plain_to_para)
                .collect();
            let caption = contents(c.get(1).and_then(|cap| cap.get(1)).cloned());
            out.extend(caption.into_iter().map(plain_to_para));
            out
        }
        "SmallCaps" => {
            let mut inlines = contents(Some(c));
            inlines.iter_mut().for_each(uppercase);
            inlines
        }
        "Superscript" | "Subscript" => {
            let sup = tag == "Superscript";
            let inlines = contents(Some(c));
            match inlines
                .iter()
                .map(|i| to_script(i, sup))
                .collect::<Option<Vec<_>>>()
            {
                Some(converted) => converted,
                None => {
                    let mut out = vec![str_node(if sup { "^(" } else { "_(" })];
                    out.extend(inlines);
                    out.push(str_node(")"));
                    out
                }
            }
        }
        _ => {
            let mut c = c;
            match tag.as_str() {
                "Underline" => v["t"] = json!("Emph"),
                // [attr, inlines, target]
                "Image" | "Link" => {
                    if let Some(attr) = c.get_mut(0) {
                        *attr = json!(["", [], []]);
                    }
                }
                _ => {}
            }
            normalize_value(&mut c);
            if !c.is_null() {
                v["c"] = c;
            }
            vec![v]
        }
    }
}

/// The normalised items of a block or inline list (empty if absent).
fn contents(list: Option<Value>) -> Vec<Value> {
    match list {
        Some(Value::Array(items)) => items.into_iter().flat_map(expand).collect(),
        _ => Vec::new(),
    }
}

/// `Plain` → `Para`, so a figure's picture and caption are separate paragraphs.
fn plain_to_para(mut block: Value) -> Value {
    if block.get("t").and_then(Value::as_str) == Some("Plain") {
        block["t"] = json!("Para");
    }
    block
}

fn str_node(s: &str) -> Value {
    json!({"t": "Str", "c": s})
}

/// Upper-cases every `Str` inside `v`.
fn uppercase(v: &mut Value) {
    match v {
        Value::Object(o) if o.get("t").and_then(Value::as_str) == Some("Str") => {
            if let Some(Value::String(s)) = o.get_mut("c") {
                *s = s.to_uppercase();
            }
        }
        Value::Object(o) => o.values_mut().for_each(uppercase),
        Value::Array(a) => a.iter_mut().for_each(uppercase),
        _ => {}
    }
}

/// `inline` in Unicode super/subscript characters, as pandoc's writer does, or
/// `None` if it contains anything that has no such character.
fn to_script(inline: &Value, sup: bool) -> Option<Value> {
    match inline.get("t").and_then(Value::as_str)? {
        "Space" | "SoftBreak" | "LineBreak" => Some(inline.clone()),
        "Str" => {
            let s = inline.get("c")?.as_str()?;
            let converted: Option<String> = s.chars().map(|ch| script_char(ch, sup)).collect();
            Some(str_node(&converted?))
        }
        _ => None,
    }
}

fn script_char(ch: char, sup: bool) -> Option<char> {
    if ch.is_whitespace() {
        return Some(ch);
    }
    // (superscript, subscript) code points.
    let (up, down) = match ch {
        '1' => (0x00B9, 0x2081),
        '2' => (0x00B2, 0x2082),
        '3' => (0x00B3, 0x2083),
        '0'..='9' => {
            let d = ch as u32 - '0' as u32;
            (0x2070 + d, 0x2080 + d)
        }
        '+' => (0x207A, 0x208A),
        '-' => (0x207B, 0x208B),
        '=' => (0x207C, 0x208C),
        '(' => (0x207D, 0x208D),
        ')' => (0x207E, 0x208E),
        _ => return None,
    };
    char::from_u32(if sup { up } else { down })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(text: &str) -> Value {
        str_node(text)
    }

    fn doc(blocks: Value) -> Value {
        json!({"pandoc-api-version": [1, 23, 1, 2], "meta": {}, "blocks": blocks})
    }

    fn para(inlines: Value) -> Value {
        json!({"t": "Para", "c": inlines})
    }

    fn normalized(blocks: Value) -> Value {
        let mut ast = doc(blocks);
        normalize(&mut ast);
        ast["blocks"].take()
    }

    #[test]
    fn raw_nodes_dropped() {
        let got = normalized(json!([
            {"t": "RawBlock", "c": ["html", "<div>"]},
            para(json!([s("a"), {"t": "RawInline", "c": ["html", "<br>"]}, s("b")])),
        ]));
        assert_eq!(got, json!([para(json!([s("a"), s("b")]))]));
    }

    #[test]
    fn image_and_link_attributes_cleared() {
        let got = normalized(json!([para(json!([
            {"t": "Image", "c": [["id1", ["c"], [["width", "4pt"]]], [s("alt")], ["a.png", ""]]},
            {"t": "Link", "c": [["l", ["c"], []], [s("x")], ["http://a", ""]]},
        ]))]));
        assert_eq!(
            got,
            json!([para(json!([
                {"t": "Image", "c": [["", [], []], [s("alt")], ["a.png", ""]]},
                {"t": "Link", "c": [["", [], []], [s("x")], ["http://a", ""]]},
            ]))])
        );
    }

    #[test]
    fn figure_unwrapped_with_caption_paragraph() {
        let img = json!({"t": "Image", "c": [["", [], []], [s("Chart")], ["a.png", ""]]});
        let got = normalized(json!([
            {"t": "Figure", "c": [["fig1", [], []],
                [null, [{"t": "Plain", "c": [s("Chart")]}]],
                [{"t": "Plain", "c": [img.clone()]}]]},
            {"t": "Figure", "c": [["", [], []], [null, []], [{"t": "Plain", "c": [img.clone()]}]]},
        ]));
        assert_eq!(
            got,
            json!([
                para(json!([img.clone()])),
                para(json!([s("Chart")])),
                para(json!([img]))
            ])
        );
    }

    #[test]
    fn span_and_div_unwrapped_recursively() {
        let got = normalized(json!([
            {"t": "Div", "c": [["d", ["section"], []], [
                {"t": "Header", "c": [1, ["h", [], []], [
                    {"t": "Span", "c": [["anchor", [], []], []]}, s("Title")]]},
                {"t": "Div", "c": [["", [], []], [para(json!([
                    {"t": "Span", "c": [["", ["c"], []], [s("in"), {"t": "RawInline", "c": ["html", "<b>"]}]]}
                ]))]]}
            ]]}
        ]));
        assert_eq!(
            got,
            json!([
                {"t": "Header", "c": [1, ["h", [], []], [s("Title")]]},
                para(json!([s("in")]))
            ])
        );
    }

    #[test]
    fn inline_styles_without_html() {
        let got = normalized(json!([para(json!([
            {"t": "Underline", "c": [s("u")]},
            {"t": "SmallCaps", "c": [{"t": "Strong", "c": [s("sc")]}]},
            {"t": "Superscript", "c": [s("2")]},
            {"t": "Subscript", "c": [s("10")]},
            {"t": "Superscript", "c": [s("st")]},
        ]))]));
        assert_eq!(
            got,
            json!([para(json!([
                {"t": "Emph", "c": [s("u")]},
                {"t": "Strong", "c": [s("SC")]},
                s("²"),
                s("₁₀"),
                s("^("), s("st"), s(")"),
            ]))])
        );
    }

    #[test]
    fn tables_and_other_nodes_untouched() {
        let table = json!({"t": "Table", "c": [["t", [], []], [null, []],
            [[{"t": "AlignDefault"}, {"t": "ColWidthDefault"}]], [["", [], []], []], [], [["", [], []], []]]});
        let header = json!({"t": "Header", "c": [2, ["id", ["cls"], []], [s("H")]]});
        let blocks = json!([table, header, {"t": "HorizontalRule"}]);
        assert_eq!(normalized(blocks.clone()), blocks);
    }
}
