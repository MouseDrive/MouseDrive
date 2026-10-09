#![deny(unsafe_code)]

use std::io::Read;
#[cfg(target_os = "linux")]
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use mousedrive::log;
#[cfg(target_os = "linux")]
use ureq::rustls::{self, ClientConfig, RootCertStore};

use super::error::UpdateError;

const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
const USER_AGENT: &str = concat!("MouseDrive/", env!("CARGO_PKG_VERSION"));
const PROXY_VARS: [&str; 6] = [
    "https_proxy",
    "HTTPS_PROXY",
    "all_proxy",
    "ALL_PROXY",
    "http_proxy",
    "HTTP_PROXY",
];
const NO_PROXY_VARS: [&str; 2] = ["no_proxy", "NO_PROXY"];
const SOCKS_SCHEME: &str = "socks";

struct Agents {
    direct: ureq::Agent,
    proxied: Option<ureq::Agent>,
    no_proxy: Vec<String>,
}

impl Agents {
    fn for_url(&self, url: &str) -> &ureq::Agent {
        let Some(proxied) = &self.proxied else {
            return &self.direct;
        };
        match self.direct.get(url).request_url() {
            Ok(target) if bypasses(target.host(), &self.no_proxy) => &self.direct,
            _ => proxied,
        }
    }
}

fn agents() -> &'static Agents {
    static AGENTS: OnceLock<Agents> = OnceLock::new();
    AGENTS.get_or_init(|| build_agents(&|name| std::env::var(name).ok()))
}

fn build_agents(env: &dyn Fn(&str) -> Option<String>) -> Agents {
    #[cfg(target_os = "linux")]
    let tls = tls_config();
    let builder = || {
        let builder = ureq::AgentBuilder::new()
            .user_agent(USER_AGENT)
            .try_proxy_from_env(false);
        #[cfg(target_os = "linux")]
        let builder = match &tls {
            Some(config) => builder.tls_config(Arc::clone(config)),
            None => builder,
        };
        builder
    };
    let proxied = choose_proxy(env).map(|(name, proxy)| {
        log::info(&format!(
            "güncelleme bağlantısı ortamdaki proxy ayarını kullanıyor ({name})"
        ));
        builder().proxy(proxy).build()
    });
    let no_proxy = no_proxy_list(env);
    if proxied.is_some() && !no_proxy.is_empty() {
        log::info(&format!(
            "proxy kullanılmayan adresler (NO_PROXY): {}",
            no_proxy.join(",")
        ));
    }
    Agents {
        direct: builder().build(),
        proxied,
        no_proxy,
    }
}

fn choose_proxy(env: &dyn Fn(&str) -> Option<String>) -> Option<(&'static str, ureq::Proxy)> {
    PROXY_VARS.iter().find_map(|&name| {
        let value = env(name)?;
        let value = value.trim();
        if value.is_empty() {
            return None;
        }
        if is_socks(value) {
            log::warn(&format!(
                "{name} bir SOCKS proxy'si; güncelleyici yalnız HTTP proxy'si kullanabildiği için yok sayıldı"
            ));
            return None;
        }
        ureq::Proxy::new(value)
            .map(|proxy| (name, proxy))
            .map_err(|e| log::warn(&format!("{name} proxy ayarı okunamadı, yok sayıldı: {e}")))
            .ok()
    })
}

fn is_socks(value: &str) -> bool {
    value
        .split_once("://")
        .is_some_and(|(scheme, _)| scheme.to_ascii_lowercase().starts_with(SOCKS_SCHEME))
}

fn no_proxy_list(env: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
    let Some(value) = NO_PROXY_VARS
        .iter()
        .filter_map(|name| env(name))
        .find(|value| !value.trim().is_empty())
    else {
        return Vec::new();
    };
    value
        .split(',')
        .map(normalize_host)
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn normalize_host(entry: &str) -> String {
    let entry = entry.trim();
    if entry == "*" {
        return entry.to_string();
    }
    entry
        .trim_start_matches("*.")
        .trim_start_matches('.')
        .trim_matches(['[', ']'])
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn bypasses(host: &str, no_proxy: &[String]) -> bool {
    let host = normalize_host(host);
    no_proxy.iter().any(|entry| {
        entry == "*"
            || *entry == host
            || host
                .strip_suffix(entry.as_str())
                .is_some_and(|rest| rest.ends_with('.'))
    })
}

pub(super) fn get_json(url: &str) -> Result<String, UpdateError> {
    let response = agents()
        .for_url(url)
        .get(url)
        .set("Accept", "application/vnd.github+json")
        .timeout(HTTP_TIMEOUT)
        .call()
        .map_err(|e| network_error(url, e))?;
    response.into_string().map_err(|e| UpdateError::Network {
        url: url.to_string(),
        detail: e.to_string(),
    })
}

pub(super) fn download(url: &str, limit: u64) -> Result<Vec<u8>, UpdateError> {
    let response = agents()
        .for_url(url)
        .get(url)
        .timeout(DOWNLOAD_TIMEOUT)
        .call()
        .map_err(|e| network_error(url, e))?;
    let announced = response
        .header("Content-Length")
        .and_then(|v| v.trim().parse::<u64>().ok());
    if exceeds(announced, limit) {
        return Err(too_large(url, limit));
    }
    let mut buf = Vec::new();
    response
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| UpdateError::Network {
            url: url.to_string(),
            detail: e.to_string(),
        })?;
    if exceeds(Some(buf.len() as u64), limit) {
        return Err(too_large(url, limit));
    }
    Ok(buf)
}

fn exceeds(len: Option<u64>, limit: u64) -> bool {
    len.is_some_and(|len| len > limit)
}

fn too_large(url: &str, limit: u64) -> UpdateError {
    UpdateError::TooLarge {
        url: url.to_string(),
        limit,
    }
}

fn network_error(url: &str, error: ureq::Error) -> UpdateError {
    let detail = match error {
        ureq::Error::Status(code, response) => {
            format!("HTTP {code} {}", response.status_text())
        }
        ureq::Error::Transport(transport) => transport.to_string(),
    };
    UpdateError::Network {
        url: url.to_string(),
        detail,
    }
}

#[cfg(target_os = "linux")]
fn tls_config() -> Option<Arc<ClientConfig>> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
        .map_err(|e| {
            log::warn(&format!(
                "TLS ayarı kurulamadı, varsayılan kullanılıyor: {e}"
            ))
        })
        .ok()?;
    Some(Arc::new(
        builder
            .with_root_certificates(tls_roots())
            .with_no_client_auth(),
    ))
}

#[cfg(target_os = "linux")]
fn tls_roots() -> RootCertStore {
    let mut roots = RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let native = rustls_native_certs::load_native_certs();
    if let Some(first) = native.errors.first() {
        log::warn(&format!(
            "sistem sertifikalarının bir kısmı okunamadı ({} hata), ilki: {first}",
            native.errors.len()
        ));
    }
    let (added, ignored) = roots.add_parsable_certificates(native.certs);
    log::info(&format!(
        "güncelleme bağlantısı için {added} sistem kök sertifikası eklendi ({ignored} geçersiz)"
    ));
    roots
}
