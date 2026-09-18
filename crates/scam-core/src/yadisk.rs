//! Клиент REST API Яндекс Диска.
//!
//! [`PublicDisk`] читает публичную папку без токена (так работает лаунчер),
//! [`Disk`] пишет на диск владельца по OAuth-токену (так работает `scam-pack`).

use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH};
use reqwest::{Client, Method, RequestBuilder, Response, StatusCode};
use serde::Deserialize;
use std::future::Future;
use std::path::Path;
use std::time::Duration;

const API: &str = "https://cloud-api.yandex.net/v1/disk";
const ATTEMPTS: u32 = 5;
const PAGE: u64 = 1000;

#[derive(Debug, thiserror::Error)]
pub enum YaError {
    #[error("не найдено на Яндекс Диске: {0}")]
    NotFound(String),
    #[error("токен Яндекса недействителен или истёк — выполни `scam-pack login`")]
    Unauthorized,
    #[error("Яндекс Диск ответил {status} {error}: {description}")]
    Api {
        status: u16,
        error: String,
        description: String,
    },
    #[error("ошибка сети: {0}")]
    Http(#[from] reqwest::Error),
    #[error("ошибка файла: {0}")]
    Io(#[from] std::io::Error),
}

impl YaError {
    pub fn is_not_found(&self) -> bool {
        matches!(self, YaError::NotFound(_))
    }

    fn api_code(&self) -> Option<&str> {
        match self {
            YaError::Api { error, .. } => Some(error),
            _ => None,
        }
    }

    fn is_transient(&self) -> bool {
        match self {
            YaError::Http(e) => e.is_timeout() || e.is_connect() || e.is_request() || e.is_body(),
            YaError::Api { status, .. } => *status == 429 || *status >= 500,
            YaError::Io(_) => true,
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, YaError>;

#[derive(Deserialize)]
struct ApiErrorBody {
    #[serde(default)]
    error: String,
    #[serde(default)]
    description: String,
}

#[derive(Deserialize)]
struct Link {
    href: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Resource {
    pub path: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(default)]
    pub public_key: Option<String>,
    #[serde(default, rename = "_embedded")]
    pub embedded: Option<ResourceList>,
}

impl Resource {
    pub fn is_dir(&self) -> bool {
        self.kind == "dir"
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResourceList {
    #[serde(default)]
    pub items: Vec<Resource>,
    #[serde(default)]
    pub total: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct PublicResourceList {
    #[serde(default)]
    items: Vec<Resource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiskInfo {
    pub total_space: u64,
    pub used_space: u64,
    #[serde(default)]
    pub user: Option<DiskUser>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiskUser {
    #[serde(default)]
    pub login: String,
    #[serde(default)]
    pub display_name: String,
}

pub fn http_client(user_agent: &str) -> Client {
    Client::builder()
        .user_agent(user_agent)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(60))
        .build()
        .expect("не удалось создать HTTP-клиент")
}

/// Повторяет запрос при сетевых сбоях, 429 и 5xx.
async fn send(make: impl Fn() -> RequestBuilder) -> Result<Response> {
    let mut delay = Duration::from_millis(500);
    for attempt in 1.. {
        let last = attempt == ATTEMPTS;
        match make().send().await {
            Ok(resp) => {
                let s = resp.status();
                if last || !(s == StatusCode::TOO_MANY_REQUESTS || s.is_server_error()) {
                    return Ok(resp);
                }
            }
            Err(e) => {
                let err = YaError::Http(e);
                if last || !err.is_transient() {
                    return Err(err);
                }
            }
        }
        tokio::time::sleep(delay).await;
        delay *= 2;
    }
    unreachable!()
}

/// Повторяет целую операцию (например, «получить ссылку + залить файл»).
async fn retry_op<T, F, Fut>(op: F) -> Result<T>
where
    F: Fn() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let mut delay = Duration::from_millis(500);
    let mut attempt = 1;
    loop {
        match op().await {
            Err(e) if e.is_transient() && attempt < ATTEMPTS => {
                tokio::time::sleep(delay).await;
                delay *= 2;
                attempt += 1;
            }
            other => return other,
        }
    }
}

/// Превращает неуспешный ответ в [`YaError`].
async fn check(resp: Response, what: &str) -> Result<Response> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    if status == StatusCode::NOT_FOUND {
        return Err(YaError::NotFound(what.to_owned()));
    }
    if status == StatusCode::UNAUTHORIZED {
        return Err(YaError::Unauthorized);
    }
    let body: ApiErrorBody = resp.json().await.unwrap_or(ApiErrorBody {
        error: String::new(),
        description: String::new(),
    });
    Err(YaError::Api {
        status: status.as_u16(),
        error: body.error,
        description: if body.description.is_empty() {
            what.to_owned()
        } else {
            body.description
        },
    })
}

async fn fetch_href(http: &Client, href: &str, what: &str) -> Result<Vec<u8>> {
    let resp = send(|| http.get(href)).await?;
    Ok(check(resp, what).await?.bytes().await?.to_vec())
}

/// Чтение публичной папки по ссылке, без токена.
#[derive(Clone)]
pub struct PublicDisk {
    http: Client,
    public_key: String,
}

impl PublicDisk {
    pub fn new(http: Client, public_url: impl Into<String>) -> Self {
        Self {
            http,
            public_key: public_url.into(),
        }
    }

    pub async fn download_url(&self, rel: &str) -> Result<String> {
        let path = format!("/{rel}");
        let resp = send(|| {
            self.http
                .get(format!("{API}/public/resources/download"))
                .query(&[("public_key", self.public_key.as_str()), ("path", &path)])
        })
        .await?;
        Ok(check(resp, rel).await?.json::<Link>().await?.href)
    }

    pub async fn get_bytes(&self, rel: &str) -> Result<Vec<u8>> {
        let href = self.download_url(rel).await?;
        fetch_href(&self.http, &href, rel).await
    }
}

/// Диск владельца по OAuth-токену. Пути — вида `disk:/Папка/файл`.
#[derive(Clone)]
pub struct Disk {
    http: Client,
    token: String,
}

impl Disk {
    pub fn new(http: Client, token: impl Into<String>) -> Self {
        Self {
            http,
            token: token.into(),
        }
    }

    fn req(&self, method: Method, endpoint: &str) -> RequestBuilder {
        self.http
            .request(method, format!("{API}{endpoint}"))
            .header(AUTHORIZATION, format!("OAuth {}", self.token))
    }

    pub async fn info(&self) -> Result<DiskInfo> {
        let resp = send(|| self.req(Method::GET, "/")).await?;
        Ok(check(resp, "информация о диске").await?.json().await?)
    }

    /// Метаданные файла или папки; `None`, если не существует.
    pub async fn resource(&self, path: &str) -> Result<Option<Resource>> {
        let resp = send(|| {
            self.req(Method::GET, "/resources")
                .query(&[("path", path), ("limit", "0")])
        })
        .await?;
        match check(resp, path).await {
            Ok(resp) => Ok(Some(resp.json().await?)),
            Err(e) if e.is_not_found() => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Содержимое папки целиком (постранично).
    pub async fn list_dir(&self, path: &str) -> Result<Vec<Resource>> {
        let mut items = Vec::new();
        let mut offset = 0u64;
        loop {
            let (limit, off) = (PAGE.to_string(), offset.to_string());
            let resp = send(|| {
                self.req(Method::GET, "/resources").query(&[
                    ("path", path),
                    ("limit", limit.as_str()),
                    ("offset", off.as_str()),
                    ("sort", "name"),
                ])
            })
            .await?;
            let res: Resource = check(resp, path).await?.json().await?;
            let page = res.embedded.map(|e| e.items).unwrap_or_default();
            let got = page.len() as u64;
            items.extend(page);
            if got < PAGE {
                return Ok(items);
            }
            offset += got;
        }
    }

    /// Создаёт папку; уже существующая — не ошибка.
    pub async fn mkdir(&self, path: &str) -> Result<()> {
        let resp = send(|| self.req(Method::PUT, "/resources").query(&[("path", path)])).await?;
        match check(resp, path).await {
            Ok(_) => Ok(()),
            Err(e) if e.api_code() == Some("DiskPathPointsToExistentDirectoryError") => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn upload_link(&self, path: &str, overwrite: bool) -> Result<String> {
        let ow = if overwrite { "true" } else { "false" };
        let resp = send(|| {
            self.req(Method::GET, "/resources/upload")
                .query(&[("path", path), ("overwrite", ow)])
        })
        .await?;
        Ok(check(resp, path).await?.json::<Link>().await?.href)
    }

    async fn check_upload(resp: Response, path: &str) -> Result<()> {
        match resp.status() {
            StatusCode::CREATED | StatusCode::ACCEPTED | StatusCode::OK => Ok(()),
            _ => check(resp, path).await.map(|_| ()),
        }
    }

    pub async fn upload_bytes(&self, path: &str, data: &[u8], overwrite: bool) -> Result<()> {
        retry_op(|| async {
            let href = self.upload_link(path, overwrite).await?;
            let resp = self.http.put(&href).body(data.to_vec()).send().await?;
            Self::check_upload(resp, path).await
        })
        .await
    }

    pub async fn upload_file(&self, path: &str, file: &Path, overwrite: bool) -> Result<()> {
        retry_op(|| async {
            let href = self.upload_link(path, overwrite).await?;
            let f = tokio::fs::File::open(file).await?;
            let len = f.metadata().await?.len();
            let body = reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(f));
            let resp = self
                .http
                .put(&href)
                .header(CONTENT_LENGTH, len)
                .body(body)
                .send()
                .await?;
            Self::check_upload(resp, path).await
        })
        .await
    }

    pub async fn get_bytes(&self, path: &str) -> Result<Vec<u8>> {
        let resp = send(|| {
            self.req(Method::GET, "/resources/download")
                .query(&[("path", path)])
        })
        .await?;
        let href = check(resp, path).await?.json::<Link>().await?.href;
        fetch_href(&self.http, &href, path).await
    }

    /// Удаляет файл или папку (в корзину, если не `permanently`). Отсутствие — не ошибка.
    pub async fn delete(&self, path: &str, permanently: bool) -> Result<()> {
        let perm = if permanently { "true" } else { "false" };
        let resp = send(|| {
            self.req(Method::DELETE, "/resources")
                .query(&[("path", path), ("permanently", perm)])
        })
        .await?;
        match check(resp, path).await {
            Ok(_) => Ok(()),
            Err(e) if e.is_not_found() => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Опубликованные папки владельца.
    pub async fn public_dirs(&self) -> Result<Vec<Resource>> {
        let mut items = Vec::new();
        let mut offset = 0u64;
        loop {
            let (limit, off) = (PAGE.to_string(), offset.to_string());
            let resp = send(|| {
                self.req(Method::GET, "/resources/public").query(&[
                    ("type", "dir"),
                    ("limit", limit.as_str()),
                    ("offset", off.as_str()),
                ])
            })
            .await?;
            let page: PublicResourceList =
                check(resp, "список публичных папок").await?.json().await?;
            let got = page.items.len() as u64;
            items.extend(page.items);
            if got < PAGE {
                return Ok(items);
            }
            offset += got;
        }
    }
}

/// Идентификатор публичной ссылки: `https://disk.yandex.ru/d/AbC` и `https://yadi.sk/d/AbC` → `AbC`.
pub fn public_link_id(url: &str) -> Option<&str> {
    let (_, tail) = url.split_once("/d/")?;
    let id = tail.split(['/', '?', '#']).next()?;
    (!id.is_empty()).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::public_link_id;

    #[test]
    fn link_ids() {
        assert_eq!(
            public_link_id("https://disk.yandex.ru/d/MiMS9XJjpxt52w"),
            Some("MiMS9XJjpxt52w")
        );
        assert_eq!(
            public_link_id("https://yadi.sk/d/MiMS9XJjpxt52w"),
            Some("MiMS9XJjpxt52w")
        );
        assert_eq!(public_link_id("https://yadi.sk/d/Ab/sub?x=1"), Some("Ab"));
        assert_eq!(public_link_id("https://disk.yandex.ru/i/xyz"), None);
    }
}
