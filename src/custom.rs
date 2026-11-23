// SPDX-License-Identifier: AGPL-3.0-or-later
//! Owner customisation without ever running owner-supplied code (FR-50).
//!
//! * CSS is parsed into rules and rebuilt from an allowlist of properties and value characters. Anything else
//!   (at-rules, `url()`, `expression`, escapes, comments, unknown properties) is dropped. Every selector is forced under
//!   `.cust` so it can only affect the customer page, never the staff screens.
//! * HTML goes through `ammonia` with a tiny tag allowlist and no attributes except `href` on https links.

const MAX_CSS: usize = 8 * 1024;
const MAX_HTML: usize = 2 * 1024;

const PROPS: &[&str] = &[
    "color",
    "background-color",
    "background",
    "font-size",
    "font-weight",
    "font-style",
    "font-family",
    "letter-spacing",
    "line-height",
    "text-align",
    "text-transform",
    "border",
    "border-color",
    "border-width",
    "border-style",
    "border-radius",
    "padding",
    "margin",
    "padding-top",
    "padding-bottom",
    "padding-left",
    "padding-right",
    "margin-top",
    "margin-bottom",
    "max-width",
    "opacity",
];

fn value_ok(v: &str) -> bool {
    let low = v.to_ascii_lowercase();
    !v.is_empty()
        && v.len() <= 80
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || " #%.,()-_\"'".contains(c))
        && ![
            "url",
            "expression",
            "javascript",
            "image-set",
            "attr(",
            "env(",
            "behavior",
            "binding",
            "!important",
        ]
        .iter()
        .any(|bad| low.contains(bad))
}

/// A selector is a space/`>`/`,` separated list of `.class`, `tag`, `tag.class`, `:hover`, `:first-child`. Nothing else.
fn selector_ok(sel: &str) -> bool {
    let sel = sel.trim();
    !sel.is_empty()
        && sel.len() <= 80
        && sel.chars().all(|c| c.is_ascii_alphanumeric() || " .,>-_:#".contains(c))
        && !sel.contains("::")
        && sel.split([' ', '>', ',']).filter(|p| !p.is_empty()).all(|p| {
            let base = p.split(':').next().unwrap_or("");
            let pseudo_ok = p
                .split(':')
                .skip(1)
                .all(|ps| matches!(ps, "hover" | "focus" | "first-child" | "last-child"));
            pseudo_ok && !base.starts_with('#') && !base.is_empty() || (base.is_empty() && pseudo_ok)
        })
}

/// Returns clean CSS text and the number of rules/declarations that were dropped.
pub fn sanitize_css(input: &str) -> (String, usize) {
    let input: String = input.chars().take(MAX_CSS).collect();
    let mut out = String::new();
    let mut dropped = 0;
    // Strip comments first so they cannot hide anything, then split on `}` into rules.
    let mut cleaned = String::new();
    let mut rest = input.as_str();
    while let Some(i) = rest.find("/*") {
        cleaned.push_str(&rest[..i]);
        rest = rest[i + 2..].split_once("*/").map(|x| x.1).unwrap_or("");
    }
    cleaned.push_str(rest);
    for rule in cleaned.split('}') {
        let Some((sel, body)) = rule.split_once('{') else {
            if !rule.trim().is_empty() {
                dropped += 1;
            }
            continue;
        };
        if sel.contains('@') || body.contains('{') || sel.contains('\\') || body.contains('\\') || !selector_ok(sel) {
            dropped += 1;
            continue;
        }
        let mut decls = Vec::new();
        for d in body.split(';') {
            let d = d.trim();
            if d.is_empty() {
                continue;
            }
            match d.split_once(':') {
                Some((p, v)) if PROPS.contains(&p.trim().to_ascii_lowercase().as_str()) && value_ok(v.trim()) => {
                    decls.push(format!("{}:{}", p.trim().to_ascii_lowercase(), v.trim()))
                }
                _ => dropped += 1,
            }
        }
        if decls.is_empty() {
            continue;
        }
        let scoped: Vec<String> = sel.split(',').map(|s| format!(".cust {}", s.trim())).collect();
        out.push_str(&format!("{}{{{}}}\n", scoped.join(","), decls.join(";")));
    }
    (out, dropped)
}

pub fn sanitize_html(input: &str) -> String {
    let input: String = input.chars().take(MAX_HTML).collect();
    ammonia::Builder::new()
        .tags(
            ["p", "strong", "em", "br", "ul", "ol", "li", "a", "h2", "h3", "span"]
                .into_iter()
                .collect(),
        )
        .tag_attributes(std::collections::HashMap::from([("a", ["href"].into_iter().collect())]))
        .url_schemes(["https"].into_iter().collect())
        .link_rel(Some("noopener noreferrer nofollow"))
        .clean(&input)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_safe_css_and_scopes_it() {
        let (css, dropped) = sanitize_css(
            "h1 { color: #a8481f; font-size: 28px } .price, button:hover { background-color: rgb(10, 20, 30); }",
        );
        assert_eq!(dropped, 0);
        assert!(css.contains(".cust h1{color:#a8481f;font-size:28px}"), "{css}");
        assert!(
            css.contains(".cust .price,.cust button:hover{background-color:rgb(10, 20, 30)}"),
            "{css}"
        );
    }

    #[test]
    fn drops_every_dangerous_construct() {
        for evil in [
            "@import url(https://evil.example/x.css);",
            "body{background:url(https://evil.example/pixel)}",
            "h1{background:url(javascript:alert(1))}",
            "h1{width:expression(alert(1))}",
            "h1{color:red} </style><script>alert(1)</script>",
            "h1{color:\\72 ed}",
            "h1{behavior:url(x.htc)}",
            "#login input{color:red}",
            "h1::before{content:'x'}",
            "h1{color:red !important}",
            "a{-moz-binding:url(x)}",
            "@media print{h1{color:red}}",
            "h1{position:fixed;top:0;left:0;width:100%;height:100%}",
        ] {
            let (css, _) = sanitize_css(evil);
            let low = css.to_lowercase();
            assert!(
                !low.contains("url")
                    && !low.contains("script")
                    && !low.contains("expression")
                    && !low.contains('@')
                    && !low.contains("position")
                    && !low.contains("::")
                    && !low.contains("!important"),
                "{evil} -> {css}"
            );
        }
    }

    #[test]
    fn html_allows_text_and_https_links_only() {
        let out = sanitize_html(
            "<p onclick=\"x()\">Halo <strong>kafe</strong></p><script>alert(1)</script><img src=x onerror=alert(1)><a href=\"javascript:alert(1)\">a</a><a href=\"https://ok.example\">ok</a><iframe src=\"https://e\"></iframe>",
        );
        assert!(out.contains("<strong>kafe</strong>") && out.contains("https://ok.example"));
        let low = out.to_lowercase();
        assert!(
            !low.contains("script")
                && !low.contains("onclick")
                && !low.contains("onerror")
                && !low.contains("javascript")
                && !low.contains("<img")
                && !low.contains("iframe"),
            "{out}"
        );
    }
}
