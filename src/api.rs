/// HTTP client for MetaTube SDK backend API.
/// Communicates via REST with Bearer token auth (mirrors `route/auth.go`).
use crate::error::Error;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use std::error::Error as StdError;
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
                error!("request failed: {:#}", e);
                if let Some(src) = e.source() {
                    error!("caused by: {}", src);
                }
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
        debug!("searching: {}", path);
        let results: Vec<MovieSearchResult> = self.get_data(&path).await?;
        results.into_iter().next().ok_or_else(|| Error::NoResults(keyword.to_string()))
    }

    /// Get full movie info by provider and ID.
    #[instrument(skip(self), fields(provider = %provider, id = %id))]
    pub async fn get_movie_info(&self, provider: &str, id: &str) -> Result<MovieInfo, Error> {
        let path = format!("/v1/movies/{}/{}?lazy=false", urlencode(provider), urlencode(id));
        debug!("fetching: {}", path);
        self.get_data(&path).await
    }

    /// Get actor info from Gfriends for name normalization.
    #[instrument(skip(self), fields(name = %name))]
    pub async fn get_gfriends_actor(&self, name: &str) -> Result<ActorInfo, Error> {
        let path = format!("/v1/actors/gfriends/{}", urlencode(name));
        debug!("fetching: {}", path);
        self.get_data(&path).await
    }
}

fn urlencode(s: &str) -> String {
    s.replace(' ', "%20")
}
