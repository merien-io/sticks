use serde_json::Value;
use url::Url;

pub fn http_get(url: &str, headers: &[(&str, &str)]) -> Result<Value, String> {
    let parsed_url = Url::parse(url).map_err(|e| format!("Invalid URL: {e}"))?;

    let agent = ureq::builder()
        .timeout(std::time::Duration::from_secs(5))
        .redirects(0)
        .build();

    let mut req = agent.get(url);

    let is_github = parsed_url.host_str() == Some("api.github.com");
    if is_github {
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            req = req.set("Authorization", &format!("Bearer {}", token.trim()));
        }
    }

    let mut has_ua = false;
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("user-agent") {
            has_ua = true;
        }
        req = req.set(k, v);
    }
    if !has_ua {
        req = req.set("User-Agent", "sticks-runtime");
    }

    match req.call() {
        Ok(resp) => resp
            .into_json::<Value>()
            .map_err(|e| format!("JSON decode error: {e}")),
        Err(e) => Err(format!("HTTP request failed: {e}")),
    }
}