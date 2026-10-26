use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap},
    path::Path,
    rc::Rc,
};

use lol_html::{RewriteStrSettings, doc_comments, element, rewrite_str, text};

use crate::cli::CliError;

pub(crate) fn local_image_sources(html: &str) -> Result<Vec<String>, CliError> {
    let sources = Rc::new(RefCell::new(Vec::new()));
    let style_text = Rc::new(RefCell::new(String::new()));
    let src_sources = Rc::clone(&sources);
    let srcset_sources = Rc::clone(&sources);
    let style_sources = Rc::clone(&sources);
    let style_text_handler = Rc::clone(&style_text);
    rewrite_str(
        html,
        RewriteStrSettings::new()
            .append_element_content_handler(element!("img[src], video[poster]", move |element| {
                for attribute in ["src", "poster"] {
                    if let Some(source) = element.get_attribute(attribute) {
                        src_sources.borrow_mut().push(source);
                    }
                }
                Ok(())
            }))
            .append_element_content_handler(element!(
                "img[srcset], source[srcset]",
                move |element| {
                    if let Some(srcset) = element.get_attribute("srcset") {
                        srcset_sources.borrow_mut().extend(srcset_urls(&srcset));
                    }
                    Ok(())
                }
            ))
            .append_element_content_handler(element!("[style]", move |element| {
                if let Some(style) = element.get_attribute("style") {
                    style_sources.borrow_mut().extend(css_urls(&style));
                }
                Ok(())
            }))
            .append_element_content_handler(text!("style", move |chunk| {
                style_text_handler.borrow_mut().push_str(chunk.as_str());
                Ok(())
            })),
    )
    .map_err(|error| CliError::Html(error.to_string()))?;

    let style_block_sources = css_urls(&style_text.borrow());
    if let Some(source) = style_block_sources.first() {
        classify_source(source)?;
        return Err(CliError::InvalidDraft(
            "local CSS url() references inside <style> are not supported; use an inline style attribute"
                .to_owned(),
        ));
    }

    let mut local = BTreeSet::new();
    for source in sources.borrow().iter() {
        local.insert(classify_source(source)?);
    }
    Ok(local.into_iter().collect())
}

pub(crate) fn rewrite_local_images(
    html: &str,
    replacements: &HashMap<String, String>,
) -> Result<String, CliError> {
    let replacements = Rc::new(replacements.clone());
    let attribute_replacements = Rc::clone(&replacements);
    let srcset_replacements = Rc::clone(&replacements);
    let style_replacements = Rc::clone(&replacements);
    rewrite_str(
        html,
        RewriteStrSettings::new()
            .append_element_content_handler(element!("img[src], video[poster]", move |element| {
                for attribute in ["src", "poster"] {
                    if let Some(source) = element.get_attribute(attribute)
                        && let Some(replacement) = attribute_replacements.get(source.trim())
                    {
                        element
                            .set_attribute(attribute, replacement)
                            .expect("attribute names are valid static HTML attribute names");
                    }
                }
                Ok(())
            }))
            .append_element_content_handler(element!(
                "img[srcset], source[srcset]",
                move |element| {
                    if let Some(srcset) = element.get_attribute("srcset") {
                        element
                            .set_attribute("srcset", &rewrite_srcset(&srcset, &srcset_replacements))
                            .expect("srcset is a valid static HTML attribute name");
                    }
                    Ok(())
                }
            ))
            .append_element_content_handler(element!("[style]", move |element| {
                if let Some(style) = element.get_attribute("style") {
                    element
                        .set_attribute("style", &rewrite_css_urls(&style, &style_replacements))
                        .expect("style is a valid static HTML attribute name");
                }
                Ok(())
            })),
    )
    .map_err(|error| CliError::Html(error.to_string()))
}

pub(crate) fn validate_plain_text(content: &str) -> Result<(), CliError> {
    let contains_markup = Rc::new(Cell::new(false));
    let element_flag = Rc::clone(&contains_markup);
    let comment_flag = Rc::clone(&contains_markup);
    rewrite_str(
        content,
        RewriteStrSettings::new()
            .append_element_content_handler(element!("*", move |_| {
                element_flag.set(true);
                Ok(())
            }))
            .append_document_content_handler(doc_comments!(move |_| {
                comment_flag.set(true);
                Ok(())
            })),
    )
    .map_err(|error| CliError::Html(error.to_string()))?;

    if contains_markup.get() {
        return Err(CliError::InvalidDraft(
            "newspic content must be plain text; HTML tags are not supported by the high-level workflow"
                .to_owned(),
        ));
    }
    Ok(())
}

fn srcset_urls(srcset: &str) -> Vec<String> {
    srcset
        .split(',')
        .filter_map(|candidate| candidate.split_whitespace().next())
        .filter(|source| !source.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn rewrite_srcset(srcset: &str, replacements: &HashMap<String, String>) -> String {
    srcset
        .split(',')
        .map(|candidate| {
            let trimmed = candidate.trim();
            let split = trimmed.find(char::is_whitespace).unwrap_or(trimmed.len());
            let (source, descriptor) = trimmed.split_at(split);
            format!(
                "{}{}",
                replacements
                    .get(source)
                    .map(String::as_str)
                    .unwrap_or(source),
                descriptor
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn css_urls(css: &str) -> Vec<String> {
    css_url_tokens(css)
        .into_iter()
        .map(|token| token.source)
        .collect()
}

struct CssUrlToken {
    source: String,
    start: usize,
    end: usize,
}

fn css_url_tokens(css: &str) -> Vec<CssUrlToken> {
    let mut urls = Vec::new();
    let lower = css.to_ascii_lowercase();
    let mut offset = 0;
    while let Some(relative_start) = lower[offset..].find("url(") {
        let value_start = offset + relative_start + 4;
        let Some(relative_end) = css[value_start..].find(')') else {
            break;
        };
        let value_end = value_start + relative_end;
        let value = css[value_start..value_end]
            .trim()
            .trim_matches(['\'', '"'])
            .trim();
        if !value.is_empty() {
            urls.push(CssUrlToken {
                source: value.to_owned(),
                start: offset + relative_start,
                end: value_end + 1,
            });
        }
        offset = value_end + 1;
    }
    urls
}

fn rewrite_css_urls(css: &str, replacements: &HashMap<String, String>) -> String {
    let mut rewritten = String::with_capacity(css.len());
    let mut cursor = 0;
    for token in css_url_tokens(css) {
        rewritten.push_str(&css[cursor..token.start]);
        if let Some(replacement) = replacements.get(&token.source) {
            rewritten.push_str("url('");
            rewritten.push_str(replacement);
            rewritten.push_str("')");
        } else {
            rewritten.push_str(&css[token.start..token.end]);
        }
        cursor = token.end;
    }
    rewritten.push_str(&css[cursor..]);
    rewritten
}

fn classify_source(source: &str) -> Result<String, CliError> {
    let source = source.trim();
    if source.is_empty() {
        return Err(CliError::InvalidDraft(
            "an <img> element has an empty src attribute".to_owned(),
        ));
    }
    let lower = source.to_ascii_lowercase();
    if lower.starts_with("https://") || lower.starts_with("http://") || source.starts_with("//") {
        return Err(CliError::InvalidDraft(format!(
            "remote image URL `{source}` is not supported; save the image inside the draft directory so it can be uploaded to WeChat"
        )));
    }
    if lower.starts_with("data:") {
        return Err(CliError::InvalidDraft(
            "data: image URLs are not supported; place the image in the draft directory".to_owned(),
        ));
    }
    if Path::new(source).is_absolute() {
        return Err(CliError::InvalidDraft(format!(
            "local image path `{source}` must be relative to the draft directory"
        )));
    }
    if source.contains('?') || source.contains('#') {
        return Err(CliError::InvalidDraft(format!(
            "local image path `{source}` cannot contain a query string or fragment"
        )));
    }
    if source
        .split('/')
        .next()
        .is_some_and(|first| first.contains(':'))
    {
        return Err(CliError::InvalidDraft(format!(
            "unsupported image URL scheme in `{source}`"
        )));
    }
    Ok(source.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_local_images_and_deduplicates_them() {
        let html = r#"
            <p><img src="assets/a.png"></p>
            <img src="assets/a.png">
            <img src="./assets/b.jpg">
        "#;

        assert_eq!(
            local_image_sources(html).unwrap(),
            vec!["./assets/b.jpg", "assets/a.png"]
        );
    }

    #[test]
    fn rewrites_only_known_local_sources() {
        let replacements = HashMap::from([(
            "assets/a.png".to_owned(),
            "https://mmbiz.example/a.png".to_owned(),
        )]);
        let result = rewrite_local_images(
            r#"<img src="assets/a.png"><img src="https://example.com/b.png">"#,
            &replacements,
        )
        .unwrap();

        assert!(result.contains(r#"src="https://mmbiz.example/a.png""#));
        assert!(result.contains(r#"src="https://example.com/b.png""#));
    }

    #[test]
    fn extracts_and_rewrites_srcset_poster_and_inline_css() {
        let html = r#"<picture><source srcset="assets/a.png 1x, assets/b.png 2x"></picture><video poster="assets/c.jpg"></video><p style="background:url('assets/d.png')"></p>"#;
        assert_eq!(
            local_image_sources(html).unwrap(),
            vec![
                "assets/a.png",
                "assets/b.png",
                "assets/c.jpg",
                "assets/d.png"
            ]
        );
        let replacements = HashMap::from([
            ("assets/a.png".to_owned(), "https://cdn/a.png".to_owned()),
            ("assets/b.png".to_owned(), "https://cdn/b.png".to_owned()),
            ("assets/c.jpg".to_owned(), "https://cdn/c.jpg".to_owned()),
            ("assets/d.png".to_owned(), "https://cdn/d.png".to_owned()),
        ]);
        let rewritten = rewrite_local_images(html, &replacements).unwrap();
        assert!(rewritten.contains("https://cdn/a.png 1x"));
        assert!(rewritten.contains("https://cdn/b.png 2x"));
        assert!(rewritten.contains("https://cdn/c.jpg"));
        assert!(rewritten.contains("https://cdn/d.png"));
    }

    #[test]
    fn rejects_local_urls_in_style_blocks_instead_of_publishing_broken_html() {
        let error = local_image_sources("<style>.hero { background: url(assets/a.png) }</style>")
            .unwrap_err();
        assert!(matches!(error, CliError::InvalidDraft(_)));
    }

    #[test]
    fn rewrites_css_urls_with_spaces_and_mixed_case() {
        let replacements =
            HashMap::from([("assets/a.png".to_owned(), "https://cdn/a.png".to_owned())]);
        let rewritten = rewrite_local_images(
            r#"<p style="background: URL( 'assets/a.png' )"></p>"#,
            &replacements,
        )
        .unwrap();
        assert!(rewritten.contains("background: url('https://cdn/a.png')"));
    }

    #[test]
    fn rejects_remote_images_in_every_supported_location() {
        for html in [
            r#"<img src="https://example.com/a.png">"#,
            r#"<source srcset="//example.com/a.png 1x">"#,
            r#"<p style="background:url('http://example.com/a.png')"></p>"#,
            r#"<style>.hero { background:url(https://example.com/a.png) }</style>"#,
        ] {
            let error = local_image_sources(html).unwrap_err();
            assert!(error.to_string().contains("remote image URL"));
        }
    }

    #[test]
    fn plain_text_validation_distinguishes_comparisons_from_markup() {
        for text in ["价格 < 3 元，今天 > 昨天", "x < y > z", "1 < 2"] {
            assert!(validate_plain_text(text).is_ok(), "{text}");
        }
        for markup in [
            "<p>caption</p>",
            "text <strong>bold</strong>",
            "<!-- note -->",
        ] {
            assert!(validate_plain_text(markup).is_err(), "{markup}");
        }
    }
}
