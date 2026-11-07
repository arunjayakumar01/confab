//! Link previews: fetch a page the user consented to and read its Open Graph tags.
//!
//! The app asks before any fetch, because the site sees the user's address. The shell caps the
//! download at 512 KB and six seconds, follows only http(s), and refuses loopback, private and
//! link-local hosts so a message cannot point the shell at something on the user's own network.

use std::net::IpAddr;
use std::time::Duration;

use serde::Serialize;

const CAP_BYTES: usize = 512 * 1024;
const TIMEOUT: Duration = Duration::from_secs(6);
const UA: &str = "Mozilla/5.0 (compatible; Confab/0.1; +https://confab.local)";

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LinkPreview {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
}

/// Why a URL is not fetched at all; shown to the user as plain words.
pub fn refuse(url: &url::Url) -> Option<&'static str> {
    if !matches!(url.scheme(), "http" | "https") {
        return Some("only http and https links have previews");
    }
    match url.host() {
        None => Some("the link has no host"),
        Some(url::Host::Domain(d)) => {
            let d = d.to_ascii_lowercase();
            if d == "localhost"
                || d.ends_with(".localhost")
                || d.ends_with(".local")
                || !d.contains('.')
            {
                Some("links to the local network have no previews")
            } else {
                None
            }
        }
        Some(url::Host::Ipv4(ip)) => {
            private(IpAddr::V4(ip)).then_some("links to the local network have no previews")
        }
        Some(url::Host::Ipv6(ip)) => {
            private(IpAddr::V6(ip)).then_some("links to the local network have no previews")
        }
    }
}

fn private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.octets()[0] == 100 && (64..=127).contains(&v4.octets()[1])
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Fetches up to `CAP_BYTES` of the page and reads its tags. `None` when the page is not HTML or says nothing useful.
pub async fn fetch_preview(
    http: &reqwest::Client,
    raw: &str,
) -> Result<Option<LinkPreview>, String> {
    let url = url::Url::parse(raw).map_err(|e| format!("bad link: {e}"))?;
    if let Some(why) = refuse(&url) {
        return Err(why.to_string());
    }
    let fut = async {
        let resp = http
            .get(url.clone())
            .header(reqwest::header::USER_AGENT, UA)
            .header(
                reqwest::header::ACCEPT,
                "text/html,application/xhtml+xml;q=0.9,*/*;q=0.5",
            )
            .send()
            .await
            .map_err(|e| format!("could not reach the site: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("the site answered {}", resp.status().as_u16()));
        }
        let html_like = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|ct| ct.contains("html") || ct.contains("xml"))
            .unwrap_or(true);
        if !html_like {
            return Ok(None);
        }
        let final_url = resp.url().clone();
        let mut body: Vec<u8> = Vec::new();
        let mut resp = resp;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| format!("read failed: {e}"))?
        {
            body.extend_from_slice(&chunk);
            if body.len() >= CAP_BYTES {
                break;
            }
        }
        let text = String::from_utf8_lossy(&body);
        Ok(parse(&final_url, &text))
    };
    tokio::time::timeout(TIMEOUT, fut)
        .await
        .map_err(|_| "the site took too long".to_string())?
}

/// Open Graph first, then the plain `<title>` and `<meta name=description>`; relative images resolve against the page.
pub fn parse(page: &url::Url, html: &str) -> Option<LinkPreview> {
    let head = match html.to_ascii_lowercase().find("</head>") {
        Some(i) => &html[..i],
        None => html,
    };
    let mut og_title = None;
    let mut og_desc = None;
    let mut og_image = None;
    let mut og_site = None;
    let mut meta_desc = None;
    let mut twitter_image = None;
    for tag in tags(head, "meta") {
        let attrs = attributes(tag);
        let key = attrs
            .iter()
            .find(|(k, _)| k == "property" || k == "name")
            .map(|(_, v)| v.to_ascii_lowercase());
        let content = attrs
            .iter()
            .find(|(k, _)| k == "content")
            .map(|(_, v)| decode(v));
        let (Some(key), Some(content)) = (key, content) else {
            continue;
        };
        let content = clean(&content);
        if content.is_empty() {
            continue;
        }
        match key.as_str() {
            "og:title" => og_title.get_or_insert(content),
            "og:description" => og_desc.get_or_insert(content),
            "og:image" | "og:image:url" | "og:image:secure_url" => og_image.get_or_insert(content),
            "og:site_name" => og_site.get_or_insert(content),
            "description" => meta_desc.get_or_insert(content),
            "twitter:image" | "twitter:image:src" => twitter_image.get_or_insert(content),
            _ => continue,
        };
    }
    let title = og_title.or_else(|| {
        let lower = head.to_ascii_lowercase();
        let start = lower.find("<title")?;
        let open_end = lower[start..].find('>')? + start + 1;
        let close = lower[open_end..].find("</title>")? + open_end;
        let t = clean(&decode(&head[open_end..close]));
        (!t.is_empty()).then_some(t)
    });
    let description = og_desc.or(meta_desc).map(|d| truncate(&d, 300));
    let image = og_image
        .or(twitter_image)
        .and_then(|i| page.join(&i).ok())
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .map(|u| u.to_string());
    let site = og_site.or_else(|| {
        page.host_str()
            .map(|h| h.trim_start_matches("www.").to_string())
    });
    if title.is_none() && description.is_none() && image.is_none() {
        return None;
    }
    Some(LinkPreview {
        url: page.to_string(),
        title: title.map(|t| truncate(&t, 160)),
        description,
        image,
        site,
    })
}

/// Every `<name ...>` tag's inside text, case-insensitively, in document order.
fn tags<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let lower = html.to_ascii_lowercase();
    let needle = format!("<{name}");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = lower[from..].find(&needle) {
        let start = from + i + needle.len();
        let Some(j) = lower[start..].find('>') else {
            break;
        };
        out.push(&html[start..start + j]);
        from = start + j + 1;
    }
    out
}

/// `key="value"`, `key='value'` and bare `key=value` pairs; keys lower-cased, values raw.
fn attributes(inside: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let chars: Vec<char> = inside.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == '/') {
            i += 1;
        }
        let ks = i;
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' && chars[i] != '/' {
            i += 1;
        }
        if ks == i {
            break;
        }
        let key: String = chars[ks..i].iter().collect::<String>().to_ascii_lowercase();
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let value: String = if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let q = chars[i];
                i += 1;
                let vs = i;
                while i < chars.len() && chars[i] != q {
                    i += 1;
                }
                let v = chars[vs..i].iter().collect();
                i += 1;
                v
            } else {
                let vs = i;
                while i < chars.len() && !chars[i].is_whitespace() {
                    i += 1;
                }
                chars[vs..i].iter().collect()
            };
            out.push((key, value));
        } else {
            out.push((key, String::new()));
        }
    }
    out
}

fn decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
}

fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_open_graph_then_falls_back_to_title_and_description() {
        let page = url::Url::parse("https://example.test/a/b").unwrap();
        let html = r#"<html><head><title>Plain &amp; simple</title>
            <meta name="description" content="  meta   description ">
            <META property='og:title' content='OG title'>
            <meta property="og:image" content="/img/card.png"/>
            <meta property="og:site_name" content="Example">
            </head><body><meta property="og:description" content="ignored, in the body"></body></html>"#;
        let p = parse(&page, html).unwrap();
        assert_eq!(p.title.as_deref(), Some("OG title"));
        assert_eq!(p.description.as_deref(), Some("meta description"));
        assert_eq!(
            p.image.as_deref(),
            Some("https://example.test/img/card.png")
        );
        assert_eq!(p.site.as_deref(), Some("Example"));
        let bare = parse(
            &page,
            "<html><head><title>Only a title</title></head></html>",
        )
        .unwrap();
        assert_eq!(bare.title.as_deref(), Some("Only a title"));
        assert_eq!(bare.site.as_deref(), Some("example.test"));
        assert!(parse(&page, "<html><head></head><body>nothing</body></html>").is_none());
    }

    #[test]
    fn refuses_local_and_odd_links() {
        let bad = [
            "http://localhost:1420/",
            "http://127.0.0.1/",
            "http://10.0.0.5/x",
            "http://192.168.1.1/",
            "http://169.254.169.254/",
            "ftp://example.test/",
            "http://intranet/",
        ];
        for b in bad {
            assert!(refuse(&url::Url::parse(b).unwrap()).is_some(), "{b}");
        }
        assert!(refuse(&url::Url::parse("https://github.com/example-org").unwrap()).is_none());
    }

    #[test]
    fn attributes_handle_quotes_and_bare_values() {
        let a = attributes(
            r#" name=viewport content="width=device-width, initial-scale=1" data-x='y' hidden"#,
        );
        assert_eq!(a[0], ("name".into(), "viewport".into()));
        assert_eq!(a[1].1, "width=device-width, initial-scale=1");
        assert_eq!(a[2], ("data-x".into(), "y".into()));
        assert_eq!(a[3], ("hidden".into(), String::new()));
    }

    /// Network: `cargo test -p prism-desktop unfurl_live -- --ignored` fetches a public page once.
    #[tokio::test]
    #[ignore]
    async fn unfurl_live() {
        let http = reqwest::Client::new();
        let p = fetch_preview(&http, "https://example.com/")
            .await
            .unwrap()
            .unwrap();
        assert!(p.title.as_deref().unwrap_or("").contains("Example"));
    }
}
