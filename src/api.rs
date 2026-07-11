/// HTTP client for MetaTube SDK backend API.
/// Communicates via REST with Bearer token auth (mirrors `route/auth.go`).
use crate::error::Error;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use tracing::{debug, error, instrument};

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    data: Option<T>,
    error: Option<ApiErrorBody>,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    code: u16,
    message: String,
}

/// Mirrors `model.MovieSearchResult`.
#[derive(Debug, Clone, Deserialize)]
pub struct MovieSearchResult {
    pub id: String,
    pub number: String,
    pub title: String,
    pub provider: String,
    pub homepage: String,
    #[serde(default)]
    pub thumb_url: Option<String>,
    #[serde(default)]
    pub cover_url: Option<String>,
    #[serde(default)]
    pub score: Option<f64>,
    #[serde(default)]
    pub actors: Vec<String>,
    #[serde(default)]
    pub release_date: Option<String>,
}

/// Mirrors `model.MovieInfo`.
#[derive(Debug, Clone, Deserialize)]
pub struct MovieInfo {
    pub id: String,
    pub number: String,
    pub title: String,
    pub provider: String,
    pub homepage: String,
    #[serde(default)]
    pub actors: Vec<String>,
    #[serde(default)]
    pub genres: Vec<String>,
    #[serde(default)]
    pub maker: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub series: Option<String>,
}

/// Mirrors `model.ActorInfo`.
#[derive(Debug, Clone, Deserialize)]
pub struct ActorInfo {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub homepage: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub images: Vec<String>,
}

pub struct Client {
    inner: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl Client {
    pub fn new(base_url: String, token: Option<String>, proxy: Option<&str>) -> Self {
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120));
        if let Some(proxy_url) = proxy {
            let p = reqwest::Proxy::all(proxy_url).expect("invalid proxy url");
            builder = builder.proxy(p);
        } else {
            builder = builder.no_proxy();
        }
        let inner = builder.build().expect("failed to build reqwest client");
        Client {
            inner,
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
        }
    }

    fn request(&self, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self.inner.get(&url).header(CONTENT_TYPE, "application/json");
        if let Some(ref token) = self.token {
            req = req.header(AUTHORIZATION, format!("Bearer {}", token));
        }
        req
    }

    async fn get_data<T: for<'de> Deserialize<'de>>(&self, path: &str) -> Result<T, Error> {
        let response = match self.request(path).send().await {
            Ok(r) => r,
            Err(e) => {
                error!("⚠ 网络请求失败，请检查网络连接");
                return Err(e.into());
            }
        };
        let status = response.status();
        if !status.is_success() {
            if let Ok(body) = response.json::<ApiResponse<T>>().await {
                if let Some(e) = body.error {
                    return Err(Error::Api { code: e.code, message: e.message });
                }
            }
            return Err(Error::Api { code: status.as_u16(), message: status.to_string() });
        }
        let body: ApiResponse<T> = response.json().await?;
        if let Some(e) = body.error {
            return Err(Error::Api { code: e.code, message: e.message });
        }
        body.data.ok_or_else(|| Error::Api { code: 500, message: "empty data".into() })
    }

    /// Search for a movie by keyword. Returns the first result.
    #[instrument(skip(self), fields(keyword = %keyword))]
    pub async fn search_movie(&self, keyword: &str) -> Result<MovieSearchResult, Error> {
        let path = format!("/v1/movies/search?q={}&fallback=true", urlencode(keyword));
        debug!("→ 搜索影片: {}", keyword);
        let results: Vec<MovieSearchResult> = self.get_data(&path).await?;
        results.into_iter().next().ok_or_else(|| Error::NoResults(keyword.to_string()))
    }

    /// Get full movie info by provider and ID.
    #[instrument(skip(self), fields(provider = %provider, id = %id))]
    pub async fn get_movie_info(&self, provider: &str, id: &str) -> Result<MovieInfo, Error> {
        let path = format!("/v1/movies/{}/{}?lazy=false", urlencode(provider), urlencode(id));
        // trace only - too verbose for debug
        self.get_data(&path).await
    }

    /// Get actor info from Gfriends for name normalization.
    #[instrument(skip(self), fields(name = %name))]
    pub async fn get_gfriends_actor(&self, name: &str) -> Result<ActorInfo, Error> {
        let path = format!("/v1/actors/gfriends/{}", urlencode(name));
        // trace only - too verbose for debug
        self.get_data(&path).await
    }
}

fn urlencode(s: &str) -> String {
    s.replace(' ', "%20")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_urlencode() {
        assert_eq!(urlencode("hello world"), "hello%20world");
        assert_eq!(urlencode("no_spaces"), "no_spaces");
        assert_eq!(urlencode(""), "");
    }

    #[test]
    fn test_deserialize_movie_search_result_minimal() {
        let json = r#"{
            "id": "ssis00123",
            "number": "SSIS-123",
            "title": "Test Title",
            "provider": "fanza",
            "homepage": "https://example.com"
        }"#;
        let result: MovieSearchResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.id, "ssis00123");
        assert_eq!(result.number, "SSIS-123");
        assert_eq!(result.title, "Test Title");
        assert_eq!(result.provider, "fanza");
        assert_eq!(result.homepage, "https://example.com");
        assert!(result.thumb_url.is_none());
        assert!(result.cover_url.is_none());
        assert!(result.score.is_none());
        assert!(result.actors.is_empty());
        assert!(result.release_date.is_none());
    }

    #[test]
    fn test_deserialize_movie_search_result_full() {
        let json = r#"{
            "id": "midv005",
            "number": "MIDV-005",
            "title": "Another Title",
            "provider": "fanza",
            "homepage": "https://example.com/2",
            "thumb_url": "https://img.example.com/thumb.jpg",
            "cover_url": "https://img.example.com/cover.jpg",
            "score": 4.5,
            "actors": ["actress1", "actress2"],
            "release_date": "2023-01-15"
        }"#;
        let result: MovieSearchResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.thumb_url.unwrap(), "https://img.example.com/thumb.jpg");
        assert_eq!(result.cover_url.unwrap(), "https://img.example.com/cover.jpg");
        assert_eq!(result.score.unwrap(), 4.5);
        assert_eq!(result.actors.len(), 2);
        assert_eq!(result.release_date.unwrap(), "2023-01-15");
    }

    #[test]
    fn test_deserialize_movie_info_full() {
        let json = r#"{
            "id": "ssis00123",
            "number": "SSIS-123",
            "title": "Movie Info Title",
            "provider": "fanza",
            "homepage": "https://example.com",
            "actors": ["actress_a", "actress_b"],
            "genres": ["genre1", "genre2"],
            "maker": "S1",
            "label": "S1 NO.1 STYLE",
            "series": "Series Name"
        }"#;
        let info: MovieInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.actors.len(), 2);
        assert_eq!(info.genres.len(), 2);
        assert_eq!(info.maker.unwrap(), "S1");
        assert_eq!(info.label.unwrap(), "S1 NO.1 STYLE");
        assert_eq!(info.series.unwrap(), "Series Name");
    }

    #[test]
    fn test_deserialize_movie_info_minimal() {
        let json = r#"{
            "id": "test001",
            "number": "TEST-001",
            "title": "Minimal",
            "provider": "test",
            "homepage": "https://example.com"
        }"#;
        let info: MovieInfo = serde_json::from_str(json).unwrap();
        assert!(info.actors.is_empty());
        assert!(info.genres.is_empty());
        assert!(info.maker.is_none());
        assert!(info.label.is_none());
        assert!(info.series.is_none());
    }

    #[test]
    fn test_deserialize_actor_info_full() {
        let json = r#"{
            "id": "actor_001",
            "name": "Actress Name",
            "provider": "gfriends",
            "homepage": "https://example.com/actor",
            "aliases": ["alias1", "alias2"],
            "images": ["https://img.example.com/1.jpg"]
        }"#;
        let actor: ActorInfo = serde_json::from_str(json).unwrap();
        assert_eq!(actor.id, "actor_001");
        assert_eq!(actor.name, "Actress Name");
        assert_eq!(actor.provider, "gfriends");
        assert_eq!(actor.homepage, "https://example.com/actor");
        assert_eq!(actor.aliases.len(), 2);
        assert_eq!(actor.images.len(), 1);
    }

    #[test]
    fn test_deserialize_actor_info_minimal() {
        let json = r#"{
            "id": "actor_001",
            "name": "Actress Name",
            "provider": "gfriends",
            "homepage": "https://example.com"
        }"#;
        let actor: ActorInfo = serde_json::from_str(json).unwrap();
        assert!(actor.aliases.is_empty());
        assert!(actor.images.is_empty());
    }

    #[test]
    fn test_client_new_base_url_trim() {
        let client = Client::new("http://localhost:8080/".to_string(), None, None);
        assert_eq!(client.base_url, "http://localhost:8080");

        let client = Client::new("http://localhost:8080".to_string(), None, None);
        assert_eq!(client.base_url, "http://localhost:8080");
    }

    #[test]
    fn test_client_request_auth_header() {
        let client = Client::new("http://localhost".to_string(), Some("mytoken".into()), None);
        let req = client.request("/test");
        let headers = req
            .build()
            .unwrap()
            .headers()
            .get("authorization")
            .cloned();
        assert!(headers.is_some());

        let client = Client::new("http://localhost".to_string(), None, None);
        let req = client.request("/test");
        let headers = req
            .build()
            .unwrap()
            .headers()
            .get("authorization")
            .cloned();
        assert!(headers.is_none());
    }
}
