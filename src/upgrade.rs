use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{anyhow, bail, Context, Result};
use directories::ProjectDirs;
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const OWNER: &str = "Cause-of-a-Kind";
const REPO: &str = "skilldeck";
const CACHE_TTL: Duration = Duration::from_secs(24 * 60 * 60);
const FAILURE_BACKOFF: Duration = Duration::from_secs(60 * 60);
const UA: &str = concat!("skilldeck/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
pub struct UpgradeOptions {
    pub yes: bool,
    pub check: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ApiRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ApiAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ApiAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CacheFile {
    checked_at_unix: u64,
    latest_version: String,
    release: ApiRelease,
    #[serde(default)]
    last_attempt_unix: Option<u64>,
    #[serde(default)]
    last_failure_unix: Option<u64>,
}

#[derive(Debug, Clone)]
struct AssetChoice {
    archive_name: String,
    archive_url: String,
    checksum_url: String,
}

pub(crate) trait ReleaseClient {
    fn latest_stable(&self, timeout: Duration) -> Result<Option<ApiRelease>>;
    fn download(&self, url: &str, timeout: Duration) -> Result<Vec<u8>>;
}

pub struct HttpReleaseClient {
    base_url: String,
}

impl HttpReleaseClient {
    pub fn new() -> Self {
        Self {
            base_url: std::env::var("SKILLDECK_UPGRADE_BASE_URL")
                .unwrap_or_else(|_| format!("https://api.github.com/repos/{OWNER}/{REPO}")),
        }
    }
}

impl ReleaseClient for HttpReleaseClient {
    fn latest_stable(&self, timeout: Duration) -> Result<Option<ApiRelease>> {
        let url = format!("{}/releases", self.base_url.trim_end_matches('/'));
        let client = reqwest::blocking::Client::builder()
            .user_agent(UA)
            .timeout(timeout)
            .build()?;
        let releases: Vec<ApiRelease> = client
            .get(url)
            .send()
            .context("querying GitHub releases")?
            .error_for_status()
            .context("GitHub releases request failed")?
            .json()
            .context("parsing GitHub releases response")?;
        Ok(select_latest_stable(releases))
    }

    fn download(&self, url: &str, timeout: Duration) -> Result<Vec<u8>> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(UA)
            .timeout(timeout)
            .build()?;
        let mut res = client
            .get(url)
            .send()
            .with_context(|| format!("downloading {url}"))?
            .error_for_status()
            .with_context(|| format!("download failed for {url}"))?;
        let mut out = Vec::new();
        res.copy_to(&mut out)?;
        Ok(out)
    }
}

pub fn run(options: UpgradeOptions) -> Result<()> {
    run_with_client(&HttpReleaseClient::new(), options)
}

pub(crate) fn run_with_client(client: &dyn ReleaseClient, options: UpgradeOptions) -> Result<()> {
    let current = current_version()?;
    let Some(release) = client.latest_stable(Duration::from_secs(15))? else {
        bail!("no stable Skilldeck releases found");
    };
    let latest = parse_tag(&release.tag_name)?;
    let _ = write_success_cache(&release, &latest);
    if latest <= current {
        println!("Skilldeck is up to date ({current}).");
        return Ok(());
    }
    println!("Update available: Skilldeck {latest} (current {current}).");
    if options.check {
        println!("Run `skilldeck upgrade` to install it.");
        return Ok(());
    }
    if !options.yes && !confirm("Download and install it?")? {
        println!("Upgrade cancelled.");
        return Ok(());
    }
    install_release(client, &release, &latest)?;
    println!("Upgraded Skilldeck from {current} to {latest}.");
    Ok(())
}

pub fn maybe_notify() {
    if std::env::var_os("SKILLDECK_NO_UPDATE_CHECK").is_some() || !std::io::stderr().is_terminal() {
        return;
    }
    if let Ok(Some((current, latest))) = cached_or_quick_latest(&HttpReleaseClient::new()) {
        if latest > current {
            eprintln!(
                "Update available: Skilldeck {latest} (current {current}). Run `skilldeck upgrade` to install it."
            );
        }
    }
}

fn cached_or_quick_latest(client: &dyn ReleaseClient) -> Result<Option<(Version, Version)>> {
    let current = current_version()?;
    if let Some(cache) = read_cache()? {
        let age = now_unix().saturating_sub(cache.checked_at_unix);
        if age < CACHE_TTL.as_secs() {
            let latest = Version::parse(&cache.latest_version)?;
            return Ok(Some((current, latest)));
        }
        if let Some(last_failure) = cache.last_failure_unix {
            let failure_age = now_unix().saturating_sub(last_failure);
            if failure_age < FAILURE_BACKOFF.as_secs() {
                return Ok(None);
            }
        }
    }
    let release = match client.latest_stable(Duration::from_millis(750)) {
        Ok(Some(release)) => release,
        Ok(None) => return Ok(None),
        Err(err) => {
            let _ = write_failure_cache();
            return Err(err);
        }
    };
    let latest = parse_tag(&release.tag_name)?;
    let _ = write_success_cache(&release, &latest);
    Ok(Some((current, latest)))
}

fn install_release(
    client: &dyn ReleaseClient,
    release: &ApiRelease,
    latest: &Version,
) -> Result<()> {
    let choice = choose_asset(release)?;
    let archive = client.download(&choice.archive_url, Duration::from_secs(60))?;
    let checksum = client.download(&choice.checksum_url, Duration::from_secs(15))?;
    verify_checksum(&archive, &choice.archive_name, &checksum)?;
    let tmp = TempDir::new()?;
    let extracted = extract_binary(&archive, &choice.archive_name, tmp.path())?;
    let target = target_exe()?;
    ensure_replace_allowed(&target)?;
    replace_exe(&target, &extracted).with_context(|| {
        format!(
            "replacing {}; if Skilldeck was installed by a package manager, use that package manager to upgrade",
            target.display()
        )
    })?;
    let _ = write_success_cache(release, latest);
    Ok(())
}

fn target_exe() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("SKILLDECK_UPGRADE_EXE") {
        return Ok(PathBuf::from(path));
    }
    std::env::current_exe().context("locating current executable")
}

fn ensure_replace_allowed(target: &Path) -> Result<()> {
    let meta = fs::metadata(target).with_context(|| format!("reading {}", target.display()))?;
    if meta.permissions().readonly() {
        bail!(
            "{} is not writable; if Skilldeck was installed by a package manager, use that package manager to upgrade",
            target.display()
        );
    }
    let parent = target
        .parent()
        .ok_or_else(|| anyhow!("executable has no parent directory"))?;
    let probe = parent.join(format!(".skilldeck-write-test-{}", std::process::id()));
    match fs::OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(_) => { let _ = fs::remove_file(&probe); Ok(()) }
        Err(err) => bail!(
            "{} is not writable ({err}); if Skilldeck was installed by a package manager, use that package manager to upgrade",
            parent.display()
        ),
    }
}

fn replace_exe(target: &Path, new_exe: &Path) -> Result<()> {
    let current = std::env::current_exe().ok();
    if current.as_deref() == Some(target) && std::env::var_os("SKILLDECK_UPGRADE_EXE").is_none() {
        self_replace::self_replace(new_exe)?;
        return Ok(());
    }
    let backup = target.with_extension(format!("skilldeck-backup-{}", std::process::id()));
    fs::copy(target, &backup).with_context(|| format!("backing up {}", target.display()))?;
    if let Err(err) = fs::copy(new_exe, target) {
        let _ = fs::copy(&backup, target);
        let _ = fs::remove_file(&backup);
        return Err(err).with_context(|| format!("copying replacement to {}", target.display()));
    }
    let _ = fs::remove_file(&backup);
    Ok(())
}

fn choose_asset(release: &ApiRelease) -> Result<AssetChoice> {
    let archive_name = asset_name_for_target()?;
    let archive = release
        .assets
        .iter()
        .find(|a| a.name == archive_name)
        .ok_or_else(|| {
            anyhow!(
                "release {} does not contain supported asset {archive_name}",
                release.tag_name
            )
        })?;
    let sidecar_name = format!("{archive_name}.sha256");
    let checksum = release
        .assets
        .iter()
        .find(|a| a.name == sidecar_name)
        .or_else(|| release.assets.iter().find(|a| a.name == "sha256.sum"))
        .ok_or_else(|| {
            anyhow!(
                "release {} is missing SHA-256 checksum asset",
                release.tag_name
            )
        })?;
    Ok(AssetChoice {
        archive_name,
        archive_url: archive.browser_download_url.clone(),
        checksum_url: checksum.browser_download_url.clone(),
    })
}

pub fn asset_name_for_target() -> Result<String> {
    if let Ok(name) = std::env::var("SKILLDECK_UPGRADE_ASSET") {
        return Ok(name);
    }
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("linux", "x86_64") => Ok("skilldeck-x86_64-unknown-linux-gnu.tar.xz".into()),
        ("linux", "aarch64") => Ok("skilldeck-aarch64-unknown-linux-gnu.tar.xz".into()),
        ("macos", "x86_64") => Ok("skilldeck-x86_64-apple-darwin.tar.xz".into()),
        ("macos", "aarch64") => Ok("skilldeck-aarch64-apple-darwin.tar.xz".into()),
        ("windows", "x86_64") => Ok("skilldeck-x86_64-pc-windows-msvc.zip".into()),
        _ => bail!("unsupported upgrade target {os}/{arch}; install Skilldeck with a package manager or from a release archive"),
    }
}

fn extract_binary(archive: &[u8], archive_name: &str, dest: &Path) -> Result<PathBuf> {
    let exe_name = if cfg!(windows) {
        "skilldeck.exe"
    } else {
        "skilldeck"
    };
    if archive_name.ends_with(".zip") {
        let reader = io::Cursor::new(archive);
        let mut zip = zip::ZipArchive::new(reader).context("reading zip archive")?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            if file.name().rsplit('/').next() == Some(exe_name) {
                if !file.is_file() {
                    bail!("archive entry {} is not a regular file", file.name());
                }
                let out = dest.join(exe_name);
                let mut w = fs::File::create(&out)?;
                io::copy(&mut file, &mut w)?;
                return Ok(out);
            }
        }
    } else if archive_name.ends_with(".tar.xz") {
        let dec = xz2::read::XzDecoder::new(io::Cursor::new(archive));
        let mut tar = tar::Archive::new(dec);
        for entry in tar.entries().context("reading tar archive")? {
            let mut entry = entry?;
            let path = entry.path()?;
            if path.file_name().and_then(|s| s.to_str()) == Some(exe_name) {
                if !entry.header().entry_type().is_file() {
                    bail!("archive entry {} is not a regular file", path.display());
                }
                let out = dest.join(exe_name);
                entry.unpack(&out)?;
                return Ok(out);
            }
        }
    }
    bail!("archive {archive_name} does not contain expected binary {exe_name}")
}

fn verify_checksum(bytes: &[u8], archive_name: &str, checksum_bytes: &[u8]) -> Result<()> {
    let actual = format!("{:x}", Sha256::digest(bytes));
    let text = String::from_utf8_lossy(checksum_bytes);
    let mut expected = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.len() == 1
            || parts
                .iter()
                .any(|p| p.trim_start_matches('*') == archive_name)
        {
            expected = Some(parts[0].to_ascii_lowercase());
            break;
        }
    }
    let expected =
        expected.ok_or_else(|| anyhow!("checksum file does not contain {archive_name}"))?;
    if expected != actual {
        bail!("checksum mismatch for {archive_name}");
    }
    Ok(())
}

fn select_latest_stable(releases: Vec<ApiRelease>) -> Option<ApiRelease> {
    releases
        .into_iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter_map(|r| parse_tag(&r.tag_name).ok().map(|v| (v, r)))
        .max_by(|(a, _), (b, _)| a.cmp(b))
        .map(|(_, r)| r)
}

fn parse_tag(tag: &str) -> Result<Version> {
    Version::parse(tag.trim_start_matches('v'))
        .with_context(|| format!("parsing release tag {tag}"))
}

fn current_version() -> Result<Version> {
    Version::parse(env!("CARGO_PKG_VERSION")).context("parsing current Skilldeck version")
}

fn confirm(label: &str) -> Result<bool> {
    eprint!("{label} [y/N] ");
    io::stderr().flush().ok();
    let mut s = String::new();
    io::stdin().read_line(&mut s)?;
    Ok(matches!(s.trim(), "y" | "Y" | "yes" | "YES" | "Yes"))
}

fn cache_path() -> Result<PathBuf> {
    if let Ok(dir) = std::env::var("SKILLDECK_CACHE_DIR") {
        return Ok(PathBuf::from(dir).join("update-check.toml"));
    }
    let dirs = ProjectDirs::from("org", "CauseOfAKind", "skilldeck")
        .ok_or_else(|| anyhow!("could not determine user cache directory"))?;
    Ok(dirs.cache_dir().join("update-check.toml"))
}

fn read_cache() -> Result<Option<CacheFile>> {
    let path = cache_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return Ok(None),
    };
    Ok(toml::from_str(&text).ok())
}

fn write_success_cache(release: &ApiRelease, latest: &Version) -> Result<()> {
    write_cache_file(&CacheFile {
        checked_at_unix: now_unix(),
        latest_version: latest.to_string(),
        release: release.clone(),
        last_attempt_unix: Some(now_unix()),
        last_failure_unix: None,
    })
}

fn write_failure_cache() -> Result<()> {
    let now = now_unix();
    let cache = read_cache()?.map(|mut cache| {
        cache.last_attempt_unix = Some(now);
        cache.last_failure_unix = Some(now);
        cache
    });
    let cache = cache.unwrap_or_else(|| CacheFile {
        checked_at_unix: 0,
        latest_version: env!("CARGO_PKG_VERSION").to_string(),
        release: ApiRelease {
            tag_name: format!("v{}", env!("CARGO_PKG_VERSION")),
            draft: false,
            prerelease: false,
            assets: Vec::new(),
        },
        last_attempt_unix: Some(now),
        last_failure_unix: Some(now),
    });
    write_cache_file(&cache)
}

fn write_cache_file(cache: &CacheFile) -> Result<()> {
    let path = cache_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, toml::to_string_pretty(cache)?)?;
    Ok(())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

use std::io::IsTerminal;

#[cfg(test)]
mod tests {
    use super::*;
    fn release(tag: &str, draft: bool, prerelease: bool) -> ApiRelease {
        ApiRelease {
            tag_name: tag.into(),
            draft,
            prerelease,
            assets: vec![],
        }
    }

    #[test]
    fn semantic_latest_ignores_drafts_prereleases_and_older() {
        let picked = select_latest_stable(vec![
            release("v9.0.0", true, false),
            release("v8.0.0", false, true),
            release("v0.1.10", false, false),
            release("v0.1.2", false, false),
        ])
        .unwrap();
        assert_eq!(picked.tag_name, "v0.1.10");
    }

    #[test]
    fn target_asset_mapping_is_known_for_current_platform() {
        let name = asset_name_for_target().unwrap();
        assert!(name.starts_with("skilldeck-"));
        assert!(name.ends_with(".tar.xz") || name.ends_with(".zip"));
    }

    #[test]
    fn checksum_success_and_mismatch() {
        let bytes = b"archive";
        let hash = format!(
            "{:x}  skilldeck-x86_64-unknown-linux-gnu.tar.xz\n",
            Sha256::digest(bytes)
        );
        verify_checksum(
            bytes,
            "skilldeck-x86_64-unknown-linux-gnu.tar.xz",
            hash.as_bytes(),
        )
        .unwrap();
        assert!(verify_checksum(
            b"bad",
            "skilldeck-x86_64-unknown-linux-gnu.tar.xz",
            hash.as_bytes()
        )
        .is_err());
    }

    use std::{cell::Cell, sync::Mutex};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct FakeClient {
        releases: Vec<ApiRelease>,
        fail: bool,
        calls: Cell<usize>,
    }
    impl FakeClient {
        fn ok(releases: Vec<ApiRelease>) -> Self {
            Self {
                releases,
                fail: false,
                calls: Cell::new(0),
            }
        }
        fn failing() -> Self {
            Self {
                releases: Vec::new(),
                fail: true,
                calls: Cell::new(0),
            }
        }
    }
    impl ReleaseClient for FakeClient {
        fn latest_stable(&self, _: Duration) -> Result<Option<ApiRelease>> {
            self.calls.set(self.calls.get() + 1);
            if self.fail {
                bail!("offline");
            }
            Ok(select_latest_stable(self.releases.clone()))
        }
        fn download(&self, _: &str, _: Duration) -> Result<Vec<u8>> {
            unreachable!()
        }
    }

    #[test]
    fn cache_freshness_corruption_failure_backoff_and_recovery() {
        let _lock = ENV_LOCK.lock().unwrap();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("SKILLDECK_CACHE_DIR", tmp.path());
        fs::create_dir_all(tmp.path()).unwrap();
        fs::write(tmp.path().join("update-check.toml"), "not toml").unwrap();
        assert!(read_cache().unwrap().is_none());

        let rel = release("v0.1.3", false, false);
        write_success_cache(&rel, &Version::parse("0.1.3").unwrap()).unwrap();
        let cached_client = FakeClient::ok(vec![release("v9.9.9", false, false)]);
        let cached = cached_or_quick_latest(&cached_client).unwrap().unwrap();
        assert_eq!(cached.1, Version::parse("0.1.3").unwrap());

        let mut cache = read_cache().unwrap().unwrap();
        cache.checked_at_unix = 0;
        fs::write(
            tmp.path().join("update-check.toml"),
            toml::to_string_pretty(&cache).unwrap(),
        )
        .unwrap();
        let refresh_client = FakeClient::ok(vec![rel.clone()]);
        let refreshed = cached_or_quick_latest(&refresh_client).unwrap().unwrap();
        assert_eq!(refreshed.1, Version::parse("0.1.3").unwrap());

        let mut cache = read_cache().unwrap().unwrap();
        cache.checked_at_unix = 0;
        write_cache_file(&cache).unwrap();
        let failing = FakeClient::failing();
        let _ = cached_or_quick_latest(&failing);
        assert_eq!(failing.calls.get(), 1);
        let backed_off = FakeClient::ok(vec![release("v9.9.9", false, false)]);
        assert!(cached_or_quick_latest(&backed_off).unwrap().is_none());
        assert_eq!(backed_off.calls.get(), 0);

        let mut cache = read_cache().unwrap().unwrap();
        cache.last_failure_unix = Some(0);
        cache.checked_at_unix = 0;
        write_cache_file(&cache).unwrap();
        let recovered = FakeClient::ok(vec![release("v0.1.4", false, false)]);
        let latest = cached_or_quick_latest(&recovered).unwrap().unwrap().1;
        assert_eq!(latest, Version::parse("0.1.4").unwrap());
        assert_eq!(recovered.calls.get(), 1);
        std::env::remove_var("SKILLDECK_CACHE_DIR");
    }

    #[test]
    fn check_mode_caches_successful_release_check() {
        let _lock = ENV_LOCK.lock().unwrap();
        let tmp = TempDir::new().unwrap();
        std::env::set_var("SKILLDECK_CACHE_DIR", tmp.path());
        let client = FakeClient::ok(vec![release("v0.1.3", false, false)]);
        run_with_client(
            &client,
            UpgradeOptions {
                yes: false,
                check: true,
            },
        )
        .unwrap();
        assert_eq!(client.calls.get(), 1);
        let cache = read_cache().unwrap().unwrap();
        assert_eq!(cache.latest_version, "0.1.3");
        let later = FakeClient::ok(vec![release("v9.9.9", false, false)]);
        let latest = cached_or_quick_latest(&later).unwrap().unwrap().1;
        assert_eq!(latest, Version::parse("0.1.3").unwrap());
        assert_eq!(later.calls.get(), 0);
        std::env::remove_var("SKILLDECK_CACHE_DIR");
    }

    #[test]
    fn rejects_non_regular_or_missing_archive_entries() {
        let tmp = TempDir::new().unwrap();
        let exe_name = if cfg!(windows) {
            "skilldeck.exe"
        } else {
            "skilldeck"
        };

        let mut tar_bytes = Vec::new();
        {
            let enc = xz2::write::XzEncoder::new(&mut tar_bytes, 6);
            let mut tar = tar::Builder::new(enc);
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            header.set_cksum();
            tar.append_data(&mut header, format!("pkg/{exe_name}"), io::empty())
                .unwrap();
            let enc = tar.into_inner().unwrap();
            enc.finish().unwrap();
        }
        let err = extract_binary(&tar_bytes, "skilldeck-test.tar.xz", tmp.path())
            .unwrap_err()
            .to_string();
        assert!(err.contains("not a regular file"), "{err}");

        let missing = extract_binary(&[], "skilldeck-test.zip", tmp.path())
            .unwrap_err()
            .to_string();
        assert!(missing.contains("reading zip archive") || missing.contains("does not contain"));
    }
}
