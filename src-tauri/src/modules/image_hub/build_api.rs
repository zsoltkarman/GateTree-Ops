use super::config::ImageHubConfig;
use serde_json::{Value, json};

pub trait BuildApi {
    fn call(&mut self, operation: &str, arguments: Value) -> Result<Value, String>;
}
/// Direct, read-only OCI-signed REST and Object Storage requests. No MCP server.
pub struct Client {
    base: url::Url,
    profile: String,
    auth: String,
    region: String,
}
fn string<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("Missing build request field: {name}"))
}
fn code(envelope: &Value) -> &str {
    envelope
        .pointer("/data/code")
        .and_then(Value::as_str)
        .filter(|value| {
            value.len() <= 80
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        })
        .unwrap_or("no service code")
}
fn response(envelope: Value, profile: &str) -> Result<(Value, Value), String> {
    let status = envelope
        .get("status")
        .and_then(Value::as_str)
        .and_then(|value| value.split_whitespace().next())
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or("Build API returned no HTTP status.")?;
    if !(200..300).contains(&status) {
        return Err(format!(
            "{}Build API returned HTTP {status} ({}; profile: {profile}).",
            if status == 401 {
                "OCI_AUTH_REQUIRED: "
            } else {
                ""
            },
            code(&envelope)
        ));
    }
    Ok((
        envelope
            .get("data")
            .cloned()
            .ok_or("Build API returned no response data.")?,
        envelope.get("headers").cloned().unwrap_or(json!({})),
    ))
}
fn number(value: Option<&Value>) -> Option<u64> {
    value.and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|v| v.parse().ok()))
    })
}
fn list_page(body: Value, headers: Value, url: &url::Url) -> Result<Value, String> {
    let items = body
        .as_array()
        .or_else(|| body.get("data").and_then(Value::as_array))
        .or_else(|| body.get("items").and_then(Value::as_array))
        .ok_or("Build API returned no item list.")?
        .clone();
    let mut next = body
        .get("next_page")
        .filter(|value| !value.is_null())
        .cloned();
    if next.is_none() {
        if let Some(link) = body
            .pointer("/links/next")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            let link = url
                .join(link)
                .map_err(|_| "Invalid Build API continuation URL.")?;
            if link.origin() != url.origin() || link.path() != url.path() {
                return Err("Build API continuation changed its endpoint.".into());
            }
            next = link
                .query_pairs()
                .find(|(key, _)| key == "page")
                .map(|(_, value)| json!(value));
            if next.is_none() {
                return Err("Build API continuation has no page cursor.".into());
            }
        }
    }
    if next.is_none() {
        next = headers
            .as_object()
            .into_iter()
            .flatten()
            .find(|(key, _)| key.eq_ignore_ascii_case("opc-next-page"))
            .map(|(_, value)| value.clone());
    }
    Ok(json!({"items":items,"next_page":next,"complete":next.is_none()}))
}
impl Client {
    pub fn new(endpoint: &str, config: &ImageHubConfig) -> Result<Self, String> {
        let base = url::Url::parse(endpoint)
            .map_err(|_| "Set build_api_url to the HTTPS BuildService REST base URL.")?;
        if base.scheme() != "https"
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
        {
            return Err(
                "build_api_url must be an HTTPS base URL without credentials, query or fragment."
                    .into(),
            );
        }
        let config = config.resolved()?;
        if config.scm_profile.is_empty() || config.scm_auth.is_empty() {
            return Err("Set scm_profile and scm_auth for BuildService API authentication.".into());
        }
        Ok(Self {
            base,
            profile: config.scm_profile,
            auth: config.scm_auth,
            region: config
                .extra
                .get("build_region")
                .filter(|value| !value.is_empty())
                .cloned()
                .unwrap_or(config.region),
        })
    }
    fn url(&self, path: &[&str], args: &Value, query: &[&str]) -> Result<url::Url, String> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| "Invalid Build API base path.")?
            .pop_if_empty()
            .extend(path);
        for key in query {
            if let Some(value) = args.get(*key).filter(|value| !value.is_null()) {
                let value = value
                    .as_str()
                    .map(str::to_owned)
                    .unwrap_or_else(|| value.to_string());
                url.query_pairs_mut().append_pair(key, &value);
            }
        }
        Ok(url)
    }
    fn get(&self, url: &url::Url) -> Result<(Value, Value), String> {
        let args = [
            "raw-request",
            "--http-method",
            "GET",
            "--target-uri",
            url.as_str(),
            "--profile",
            &self.profile,
            "--auth",
            &self.auth,
            "--output",
            "json",
        ];
        let text = super::project_sync::run("oci", &args, None)?;
        let envelope =
            serde_json::from_str(&text).map_err(|_| "Build API returned invalid JSON.")?;
        response(envelope, &self.profile)
    }
    fn log(&self, args: &Value) -> Result<Value, String> {
        let namespace = string(args, "namespace")?;
        let bucket = string(args, "bucket_name")?;
        let object = string(args, "object_name")?;
        let region = args
            .get("region")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .unwrap_or(&self.region);
        let mut common = vec![
            "--namespace-name",
            namespace,
            "--bucket-name",
            bucket,
            "--name",
            object,
            "--profile",
            &self.profile,
            "--auth",
            &self.auth,
        ];
        if !region.is_empty() {
            common.extend(["--region", region]);
        }
        let mut head = vec!["os", "object", "head", "--output", "json"];
        head.extend(common.iter().copied());
        let raw = super::project_sync::run("oci", &head, None).map_err(|error| {
            format!(
                "Build log metadata failed (region: {}): {error}",
                if region.is_empty() {
                    "OCI profile default"
                } else {
                    region
                }
            )
        })?;
        let value: Value = serde_json::from_str(&raw)
            .map_err(|_| "Object Storage returned invalid log metadata.")?;
        let metadata = value.get("data").unwrap_or(&value);
        let size = number(
            metadata
                .get("content-length")
                .or_else(|| metadata.get("contentLength")),
        )
        .ok_or("Object Storage log size is missing.")?;
        let start = number(args.get("start_byte")).unwrap_or(0);
        if start > size {
            return Err(
                "Saved log offset exceeds the current object size; the log may have been replaced."
                    .into(),
            );
        }
        if start == size {
            return Ok(json!({"content":"","truncated":false,"next_start_byte":start}));
        }
        let limit = number(args.get("max_bytes"))
            .unwrap_or(262144)
            .clamp(4, 1048576);
        let next = start.saturating_add(limit).min(size);
        let range = format!("bytes={start}-{}", next - 1);
        let mut get = vec!["os", "object", "get", "--file", "-", "--range", &range];
        get.extend(common.iter().copied());
        let etag = metadata.get("etag").and_then(Value::as_str);
        if let Some(etag) = etag {
            get.extend(["--if-match", etag]);
        }
        let bytes = super::project_sync::run_bytes("oci", &get, None)?;
        if bytes.len() as u64 != next - start {
            return Err("Downloaded log byte count does not match the requested range.".into());
        }
        decode_log_page(start, size, bytes)
    }
}
fn decode_log_page(start: u64, size: u64, bytes: Vec<u8>) -> Result<Value, String> {
    let mut next = start
        .checked_add(bytes.len() as u64)
        .filter(|value| *value <= size)
        .ok_or("Log page exceeds the object size.")?;
    let content = match String::from_utf8(bytes) {
        Ok(content) => content,
        Err(error) => {
            let valid = error.utf8_error().valid_up_to();
            if error.utf8_error().error_len().is_some() || valid == 0 {
                return Err("Build log contains invalid UTF-8 text.".into());
            }
            next = start + valid as u64;
            String::from_utf8(error.into_bytes()[..valid].to_vec())
                .map_err(|_| "Invalid log text boundary.")?
        }
    };
    Ok(json!({"content":content,"truncated":next<size,"next_start_byte":next}))
}
impl BuildApi for Client {
    fn call(&mut self, operation: &str, args: Value) -> Result<Value, String> {
        match operation {
            "list_builds" => {
                let url = self.url(
                    &["v2", "builds"],
                    &args,
                    &[
                        "team_name",
                        "project_name",
                        "time_started",
                        "time_ended",
                        "limit",
                        "page",
                    ],
                )?;
                let (body, headers) = self.get(&url)?;
                list_page(body, headers, &url)
            }
            "get_build" => {
                let url = self.url(
                    &["v2", "builds", string(&args, "build_guid")?],
                    &json!({}),
                    &[],
                )?;
                self.get(&url).map(|(body, _)| body)
            }
            "get_build_steps" => {
                let mut url = self.url(
                    &["v2", "builds", string(&args, "build_guid")?, "steps"],
                    &args,
                    &["limit", "page"],
                )?;
                url.query_pairs_mut()
                    .append_pair("generatePARLink", "false");
                let (body, headers) = self.get(&url)?;
                list_page(body, headers, &url)
            }
            "get_step_artifacts" => {
                let mut url = self.url(
                    &[
                        "v2",
                        "builds",
                        "step",
                        string(&args, "step_guid")?,
                        "artifacts",
                    ],
                    &json!({}),
                    &[],
                )?;
                url.query_pairs_mut()
                    .append_pair("generatePARLink", "false");
                self.get(&url).map(|(body, _)| body)
            }
            "download_logs" => self.log(&args),
            _ => Err("Unsupported build archive operation.".into()),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn utf8_characters_crossing_log_ranges_are_saved_without_replacement_or_loss() {
        let raw = "ab€cd".as_bytes();
        let first = decode_log_page(0, raw.len() as u64, raw[..3].to_vec()).unwrap();
        assert_eq!(first["content"], "ab");
        assert_eq!(first["next_start_byte"], 2);
        assert_eq!(first["truncated"], true);
        let second = decode_log_page(2, raw.len() as u64, raw[2..].to_vec()).unwrap();
        assert_eq!(second["content"], "€cd");
        assert_eq!(second["truncated"], false);
        assert_eq!(
            format!(
                "{}{}",
                first["content"].as_str().unwrap(),
                second["content"].as_str().unwrap()
            ),
            "ab€cd"
        );
        assert!(decode_log_page(0, 1, vec![255]).is_err());
    }

    fn config() -> ImageHubConfig {
        serde_json::from_value(json!({"scm_profile":"TEST_SESSION","scm_auth":"security_token"}))
            .unwrap()
    }
    #[test]
    fn direct_api_pagination_and_safe_routes_preserve_all_history() {
        let client = Client::new("https://build.example.test", &config()).unwrap();
        let args = json!({"team_name":"team","project_name":"sample","time_started":"1970-01-01","limit":100});
        let url = client
            .url(
                &["v2", "builds"],
                &args,
                &["team_name", "project_name", "time_started", "limit"],
            )
            .unwrap();
        let page=list_page(json!({"data":[{"id":"build1","attributes":{"status":"DONE"}}],"links":{"next":"/v2/builds?page=2&limit=100"}}),json!({}),&url).unwrap();
        assert_eq!(page["next_page"], "2");
        assert_eq!(page["complete"], false);
        assert_eq!(page["items"][0]["id"], "build1");
        assert!(
            list_page(
                json!({"data":[],"links":{"next":"https://another.example.test/v2/builds?page=2"}}),
                json!({}),
                &url
            )
            .is_err()
        );
        assert_eq!(
            list_page(json!([]), json!({}), &url).unwrap()["complete"],
            true
        );
        assert!(Client::new("http://build.example.test", &config()).is_err());
    }
    #[test]
    fn only_http_errors_trigger_authentication_failures() {
        assert!(
            response(
                json!({"status":"200 OK","data":{"details":"authentication failed"}}),
                "TEST_SESSION"
            )
            .is_ok()
        );
        assert!(
            response(
                json!({"status":"401 Unauthorized","data":{"code":"NotAuthenticated"}}),
                "TEST_SESSION"
            )
            .unwrap_err()
            .contains("OCI_AUTH_REQUIRED")
        );
        let mut client = Client::new("https://build.example.test", &config()).unwrap();
        assert!(client.call("trigger_build", json!({})).is_err());
    }
    #[test]
    #[ignore = "Requires an external API config and an existing OCI session"]
    fn configured_direct_build_api_live() {
        let config: ImageHubConfig = serde_json::from_slice(
            &std::fs::read(std::env::var("GATETREE_BUILD_TEST_CONFIG").unwrap()).unwrap(),
        )
        .unwrap();
        let endpoint = config.extra.get("build_api_url").unwrap();
        let mut client = Client::new(endpoint, &config).unwrap();
        let team = std::env::var("GATETREE_BUILD_TEST_TEAM").unwrap();
        let project = std::env::var("GATETREE_BUILD_TEST_PROJECT").unwrap();
        let list=client.call("list_builds",json!({"team_name":team,"project_name":project,"time_started":"1970-01-01","limit":1})).unwrap();
        let guid = list["items"][0]["id"].as_str().unwrap();
        let build = client
            .call("get_build", json!({"build_guid":guid}))
            .unwrap();
        assert!(build.pointer("/data/attributes").is_some());
        let steps = client
            .call("get_build_steps", json!({"build_guid":guid}))
            .unwrap();
        let step = steps["items"][0]["step_guid"].as_str().unwrap();
        let artifacts = client
            .call("get_step_artifacts", json!({"step_guid":step}))
            .unwrap();
        assert!(artifacts.get("log_coordinates").is_some());
        assert!(!list["next_page"].is_null());
        println!(
            "Direct OCI API: build history, metadata, steps and log coordinates verified without MCP."
        );
    }
}
