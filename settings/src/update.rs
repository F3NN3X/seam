//! The GitHub self-updater. All the network code lives here, so the tray never
//! does any. A check finds the newest release, verifies its signed manifest and
//! downloads both exes ahead of time, so installing never waits on the network.
//! Installing swaps the files and restarts. The Store copy skips all of it.

use minisign_verify::{PublicKey, Signature};
use seam::api::{CallError, Client};
use seam::updates::{self, UpdateFile};
use seam::{install, log, package, process};
use semver::Version;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const REPO: &str = "https://github.com/F3NN3X/seam";
const RELEASES: &str = "https://api.github.com/repos/F3NN3X/seam/releases?per_page=10";
/// Where every asset of Seam's releases is served from, before GitHub's redirect.
const RELEASE_DOWNLOADS: &str = "https://github.com/F3NN3X/seam/releases/download/";
/// The public half of Seam's minisign key (ID DD3DF0F7CB3ACA88): the second
/// line of `seam.pub`. See packaging/README.md.
pub const SEAM_PUBLIC_KEY: &str = "RWSIyjrL9/A93aX1yKj6IwVEqWPJfCHdaObPGMyLiD3ri5I/VvS9FnGQ";
const EXES: [&str; 2] = ["seam.exe", "seam-settings.exe"];
const MANIFEST: &str = "latest.json";
const SIGNATURE: &str = "latest.json.minisig";
/// How long a tray told to quit gets before the old one is given up on.
const SLOW_QUIT_WAIT_MS: u32 = 15_000;
const API_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const QUIT_WAIT_MS: u32 = 5000;
/// How long a freshly started tray gets to answer on the pipe.
const NEW_TRAY_WAIT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct Release {
    pub tag_name: String,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}

#[derive(Clone, Debug)]
pub struct Asset {
    pub name: String,
    pub browser_download_url: String,
}

impl Release {
    fn from_json(release: &Value) -> Option<Release> {
        let assets = release["assets"].as_array().into_iter().flatten().filter_map(|asset| {
            let name = asset["name"].as_str()?.to_string();
            Some(Asset { name, browser_download_url: asset["browser_download_url"].as_str()?.to_string() })
        });
        Some(Release {
            tag_name: release["tag_name"].as_str()?.to_string(),
            draft: release["draft"].as_bool().unwrap_or(false),
            prerelease: release["prerelease"].as_bool().unwrap_or(false),
            assets: assets.collect(),
        })
    }

    fn asset_url(&self, name: &str) -> Result<&str, String> {
        let found = self.assets.iter().find(|a| a.name == name);
        let url = found.map(|a| a.browser_download_url.as_str()).ok_or_else(|| format!("The release has no {name}"))?;
        // The signature is the real check. This only keeps a download on Seam's own releases.
        if !url.starts_with(RELEASE_DOWNLOADS) {
            return Err("The release points somewhere unexpected".into());
        }
        Ok(url)
    }
}

/// Newest non-draft release above `current`. Prereleases only when opted in.
/// A tag that isn't a version doesn't count.
pub fn pick(releases: &[Release], current: &Version, prerelease: bool) -> Option<(Version, Release)> {
    releases
        .iter()
        .filter(|r| !r.draft && (prerelease || !r.prerelease))
        .filter_map(|r| Version::parse(r.tag_name.trim_start_matches('v')).ok().map(|v| (v, r.clone())))
        .filter(|(v, _)| v > current && (prerelease || v.pre.is_empty()))
        .max_by(|a, b| a.0.cmp(&b.0))
}

struct File {
    name: String,
    size: u64,
    sha256: String,
}

struct Manifest {
    version: String,
    date: String,
    notes: String,
    files: Vec<File>,
}

fn parse_manifest(bytes: &[u8]) -> Option<Manifest> {
    let manifest: Value = serde_json::from_slice(bytes).ok()?;
    let files: Vec<File> = manifest["files"]
        .as_array()?
        .iter()
        .filter_map(|f| {
            Some(File {
                name: f["name"].as_str()?.to_string(),
                size: f["size"].as_u64()?,
                sha256: f["sha256"].as_str()?.to_lowercase(),
            })
        })
        .collect();
    // Exactly the two exes: nothing missing, and no other name to join onto a path.
    let only_exes = files.iter().all(|f| EXES.contains(&f.name.as_str()));
    if !only_exes || !EXES.iter().all(|exe| files.iter().any(|f| f.name == *exe)) {
        return None;
    }
    Some(Manifest {
        version: manifest["version"].as_str()?.to_string(),
        date: manifest["date"].as_str().unwrap_or_default().to_string(),
        notes: manifest["notes"].as_str().unwrap_or_default().to_string(),
        files,
    })
}

/// Where a check is, for a window that shows it.
pub enum Progress {
    Checking,
    Downloading { version: String, done: u64, total: u64 },
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(timeout)).build().new_agent()
}

fn get(agent: &ureq::Agent, url: &str) -> Result<ureq::http::Response<ureq::Body>, String> {
    let user_agent = format!("seam/{}", env!("CARGO_PKG_VERSION"));
    agent.get(url).header("User-Agent", &user_agent).call().map_err(|e| format!("Couldn't reach GitHub: {e}"))
}

fn fetch(agent: &ureq::Agent, url: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = get(agent, url)?;
    response.body_mut().with_config().limit(limit).read_to_vec().map_err(|e| format!("Couldn't read the download: {e}"))
}

/// The manifest is only trusted if Seam's key signed exactly these bytes.
fn verify(manifest: &[u8], signature: &str) -> Result<(), String> {
    let key = PublicKey::from_base64(SEAM_PUBLIC_KEY)
        .map_err(|_| "This build has no update key yet, so no update can be trusted".to_string())?;
    let signature = Signature::decode(signature).map_err(|_| "The update's signature is unreadable".to_string())?;
    key.verify(manifest, &signature, false).map_err(|_| "The update's signature doesn't match Seam's key".to_string())
}

fn sha256_hex(digest: impl AsRef<[u8]>) -> String {
    digest.as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// Both exes are in `dir` with the sizes and checksums the manifest promises.
fn staged_ok(dir: &Path, manifest: &Manifest) -> bool {
    manifest.files.iter().all(|f| match std::fs::read(dir.join(&f.name)) {
        Ok(bytes) => bytes.len() as u64 == f.size && sha256_hex(Sha256::digest(&bytes)) == f.sha256,
        Err(_) => false,
    })
}

/// Download one file, checking its size and checksum as it arrives.
fn download(
    agent: &ureq::Agent,
    file: &File,
    url: &str,
    dir: &Path,
    arrived: &mut dyn FnMut(u64),
) -> Result<(), String> {
    let mut response = get(agent, url)?;
    let mut reader = response.body_mut().as_reader();
    let mut out = std::fs::File::create(dir.join(&file.name)).map_err(|e| e.to_string())?;
    let (mut hash, mut written, mut buf) = (Sha256::new(), 0u64, [0u8; 64 * 1024]);
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("Couldn't read the download: {e}"))?;
        if n == 0 {
            break;
        }
        written += n as u64;
        if written > file.size {
            break;
        }
        hash.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        arrived(n as u64);
    }
    if written != file.size || sha256_hex(hash.finalize()) != file.sha256 {
        return Err(format!("{} didn't match its checksum, so it was thrown away", file.name));
    }
    Ok(())
}

/// The manifest and the signature that vouches for it. Install checks them again,
/// because the files in the folder are only as good as that signature.
struct Signed {
    manifest: Vec<u8>,
    signature: String,
}

fn keep_signed(dir: &Path, signed: &Signed) -> Result<(), String> {
    std::fs::write(dir.join(MANIFEST), &signed.manifest).map_err(|e| e.to_string())?;
    std::fs::write(dir.join(SIGNATURE), &signed.signature).map_err(|e| e.to_string())
}

/// Download both exes into `updates\<version>\`, unless they're already there.
fn stage(
    release: &Release,
    signed: &Signed,
    manifest: &Manifest,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), String> {
    let dir = updates::version_dir(&manifest.version);
    if staged_ok(&dir, manifest) {
        return keep_signed(&dir, signed);
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let total = manifest.files.iter().map(|f| f.size).sum();
    let (agent, mut done) = (agent(DOWNLOAD_TIMEOUT), 0);
    let mut arrived = |n: u64| {
        done += n;
        progress(Progress::Downloading { version: manifest.version.clone(), done, total });
    };
    let staged = manifest
        .files
        .iter()
        .try_for_each(|file| download(&agent, file, release.asset_url(&file.name)?, &dir, &mut arrived));
    if let Err(e) = staged {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(e);
    }
    keep_signed(&dir, signed)?;
    // Older downloads are no use now.
    for entry in std::fs::read_dir(updates::dir()).into_iter().flatten().flatten() {
        if entry.path().is_dir() && entry.path() != dir {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}

/// Find the newest release, and download it if it's newer than this one.
fn look(prerelease: bool, progress: &mut dyn FnMut(Progress)) -> Result<Option<UpdateFile>, String> {
    progress(Progress::Checking);
    let current = Version::parse(env!("CARGO_PKG_VERSION")).map_err(|e| e.to_string())?;
    let api = agent(API_TIMEOUT);
    let listing = fetch(&api, RELEASES, 4 << 20)?;
    let listing: Value =
        serde_json::from_slice(&listing).map_err(|_| "GitHub's answer wasn't a release list".to_string())?;
    let releases: Vec<Release> = listing.as_array().into_iter().flatten().filter_map(Release::from_json).collect();
    let Some((version, release)) = pick(&releases, &current, prerelease) else { return Ok(None) };

    let manifest_bytes = fetch(&api, release.asset_url(MANIFEST)?, 1 << 20)?;
    let signature = fetch(&api, release.asset_url(SIGNATURE)?, 16 << 10)?;
    let signed = Signed { manifest: manifest_bytes, signature: String::from_utf8_lossy(&signature).into_owned() };
    verify(&signed.manifest, &signed.signature)?;
    let manifest = parse_manifest(&signed.manifest).ok_or("The update's manifest is unreadable")?;
    if manifest.version != version.to_string() {
        return Err("The update's manifest is for another version".into());
    }
    stage(&release, &signed, &manifest, progress)?;
    Ok(Some(UpdateFile {
        last_checked: now(),
        available: Some(manifest.version),
        notes: manifest.notes,
        date: manifest.date,
        size: manifest.files.iter().map(|f| f.size).sum(),
        ready: true,
        error: None,
    }))
}

/// Check for an update and record what happened in the state file, then tell
/// the tray. Returns the error too, for a window that shows it.
pub fn check(prerelease: bool, progress: &mut dyn FnMut(Progress)) -> Result<(), String> {
    let found = look(prerelease, progress);
    let file = match &found {
        Ok(Some(file)) => file.clone(),
        Ok(None) => UpdateFile { last_checked: now(), ..UpdateFile::default() },
        Err(e) => UpdateFile { last_checked: now(), error: Some(e.clone()), ..updates::read() },
    };
    let _ = updates::write(&file);
    if matches!(found, Ok(Some(_))) {
        tell_tray();
    }
    found.map(|_| ())
}

fn tell_tray() {
    if let Ok(mut tray) = Client::connect("seam-settings", None) {
        let _ = tray.call("update.ready", Value::Null);
    }
}

pub enum InstallError {
    /// The folder needs admin rights, so the user downloads the release instead.
    NotWritable,
    Failed(String),
}

fn failed(message: impl Into<String>) -> InstallError {
    InstallError::Failed(message.into())
}

fn own_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)).unwrap_or_default()
}

/// The running tray's process, if there is one. It has to be the `seam.exe`
/// next to this exe: files are only ever replaced in our own folder, whatever
/// the other end of the pipe says.
fn running_tray(dir: &Path) -> Result<Option<u32>, InstallError> {
    let mut tray = match Client::connect("seam-settings", None) {
        Ok(tray) => tray,
        Err(CallError::Io(e)) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            return Err(failed("Something else is using Seam's pipe, so nothing was changed"));
        }
        Err(_) => return Ok(None),
    };
    let info = tray.call("app.info", Value::Null).ok();
    let pid = info.and_then(|i| i["pid"].as_u64()).and_then(|pid| u32::try_from(pid).ok());
    let image = pid.and_then(process::image_path).map(|exe| exe.to_string_lossy().to_lowercase());
    let ours = dir.join("seam.exe").to_string_lossy().to_lowercase();
    match (pid, image) {
        (Some(pid), Some(image)) if image == ours => Ok(Some(pid)),
        _ => Err(failed("Seam is running from another folder, so nothing was changed")),
    }
}

fn writable(dir: &Path) -> bool {
    let probe = dir.join(".seam-write-test");
    let made = std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(&probe);
    made
}

/// Copy a staged file next to its target and check the copy against the
/// manifest, so a swap of the staged file after it was verified is caught.
fn copy_checked(from: &Path, to: &Path, file: &File) -> std::io::Result<()> {
    let new = to.with_file_name(format!("{}.new", file.name));
    let copied = std::fs::copy(from, &new).and_then(|_| std::fs::read(&new)).and_then(|bytes| {
        let same = bytes.len() as u64 == file.size && sha256_hex(Sha256::digest(&bytes)) == file.sha256;
        same.then_some(()).ok_or_else(|| std::io::Error::other(format!("{} changed after it was checked", file.name)))
    });
    match copied {
        Ok(()) => std::fs::rename(&new, to),
        Err(e) => {
            let _ = std::fs::remove_file(&new);
            Err(e)
        }
    }
}

/// Renaming a running exe works on NTFS, so each exe can replace itself.
fn swap(dir: &Path, staged: &Path, manifest: &Manifest, renamed: &mut Vec<&'static str>) -> std::io::Result<()> {
    for name in EXES {
        let old = dir.join(format!("{name}.old"));
        let _ = std::fs::remove_file(&old);
        std::fs::rename(dir.join(name), old)?;
        renamed.push(name);
    }
    for file in &manifest.files {
        copy_checked(&staged.join(&file.name), &dir.join(&file.name), file)?;
    }
    Ok(())
}

/// Put back the exes `swap` renamed, and drop any new one that arrived. A
/// `.old` that swap didn't make is from an earlier update and stays where it is.
fn roll_back(dir: &Path, renamed: &[&str]) {
    for name in renamed {
        let _ = std::fs::remove_file(dir.join(name));
        let _ = std::fs::rename(dir.join(format!("{name}.old")), dir.join(name));
    }
}

fn start_tray(dir: &Path, args: &[&str]) -> std::io::Result<std::process::Child> {
    std::process::Command::new(dir.join("seam.exe")).args(args).spawn()
}

/// The version of the tray that answers on the pipe, once one does.
fn tray_version(wait: Duration) -> Option<String> {
    let started = Instant::now();
    loop {
        if let Ok(mut tray) = Client::connect("seam-settings", None) {
            let info = tray.call("app.info", Value::Null).ok();
            if let Some(version) = info.and_then(|i| i["seam_version"].as_str().map(str::to_string)) {
                return Some(version);
            }
        }
        if started.elapsed() >= wait {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

impl InstallError {
    fn message(&self) -> String {
        match self {
            InstallError::NotWritable => "Seam is in a folder that needs admin rights".to_string(),
            InstallError::Failed(message) => message.clone(),
        }
    }
}

/// The staged files are what the signed manifest says, checked now and not
/// only when they were downloaded. `version` has to be plain semver, because
/// it becomes part of a path, and newer than this build, so an old signed
/// release can't be put back.
fn staged_and_signed(version: &str) -> Result<(PathBuf, Manifest), InstallError> {
    let incomplete = || failed("The download isn't complete. Check for updates again");
    let Some(parsed) = Version::parse(version).ok().filter(|v| v.to_string() == version) else {
        return Err(failed("That isn't a version Seam can install"));
    };
    if Version::parse(env!("CARGO_PKG_VERSION")).map_or(true, |current| parsed <= current) {
        return Err(failed("That isn't newer than this version of Seam"));
    }
    let staged = updates::version_dir(version);
    let manifest = std::fs::read(staged.join(MANIFEST)).map_err(|_| incomplete())?;
    let signature = std::fs::read_to_string(staged.join(SIGNATURE)).map_err(|_| incomplete())?;
    verify(&manifest, &signature).map_err(failed)?;
    let manifest = parse_manifest(&manifest).ok_or_else(incomplete)?;
    if manifest.version != version || !staged_ok(&staged, &manifest) {
        return Err(incomplete());
    }
    Ok((staged, manifest))
}

/// Swap in the downloaded version and start it. On any failure the old files
/// come back and the old tray starts again. A failure is also written to the
/// state file, so nothing retries it blindly.
pub fn install(version: &str) -> Result<(), InstallError> {
    let result = install_signed(version);
    if let Err(e) = &result {
        let _ = updates::write(&UpdateFile { error: Some(e.message()), ..updates::read() });
    }
    result
}

fn install_signed(version: &str) -> Result<(), InstallError> {
    let (staged, manifest) = staged_and_signed(version)?;
    let dir = own_dir();
    let tray = running_tray(&dir)?;
    if !writable(&dir) {
        return Err(InstallError::NotWritable);
    }
    if let Some(pid) = tray {
        let quit = Client::connect("seam-settings", None).and_then(|mut tray| tray.call("app.quit", Value::Null));
        if quit.is_err() {
            return Err(failed("Seam didn't close, so nothing was changed"));
        }
        if !process::wait_for_exit(pid, QUIT_WAIT_MS) {
            // It was told to close, so it will. Never leave without a tray.
            if process::wait_for_exit(pid, SLOW_QUIT_WAIT_MS) {
                let _ = start_tray(&dir, &["--tray-only"]);
            }
            return Err(failed("Seam was slow to close, so nothing was changed"));
        }
    }
    let mut renamed = Vec::new();
    if let Err(e) = swap(&dir, &staged, &manifest, &mut renamed) {
        roll_back(&dir, &renamed);
        let _ = start_tray(&dir, &["--tray-only"]);
        return Err(failed(format!("Seam is back on its old version. {e}")));
    }
    // A swap that works is no use if the new tray doesn't come up.
    let started = start_tray(&dir, &["--updated-from", env!("CARGO_PKG_VERSION")]);
    if started.is_err() || tray_version(NEW_TRAY_WAIT).as_deref() != Some(version) {
        if let Ok(mut new) = started {
            let _ = new.kill();
            let _ = new.wait();
        }
        roll_back(&dir, &renamed);
        let _ = start_tray(&dir, &["--tray-only"]);
        return Err(failed("Seam is back on its old version. The new one didn't start"));
    }
    if let Err(e) = install::note_version(version) {
        log::info!("{e}");
    }
    Ok(())
}

pub fn release_page(version: &str) -> String {
    format!("{REPO}/releases/tag/v{version}")
}

/// `seam-settings.exe --check-updates`. Headless, so the exit code is all it says.
pub fn check_main() -> i32 {
    if package::is_packaged() {
        return 0;
    }
    let prerelease = seam::config::Config::load().updates.prerelease;
    i32::from(check(prerelease, &mut |_| {}).is_err())
}

/// `seam-settings.exe --install <version>`, as the tray runs it for automatic installs.
pub fn install_main(version: &str) -> i32 {
    if package::is_packaged() {
        return 0;
    }
    i32::from(install(version).is_err())
}

/// `seam-settings.exe --quit-tray`, for the installer. Exits 0 when Seam quit or
/// wasn't running, 1 when it didn't close in time, 2 when it couldn't be asked
/// while it is still running.
pub fn quit_main() -> i32 {
    let Ok(mut tray) = Client::connect("seam-settings", None) else {
        return if process::tray_running() { 2 } else { 0 };
    };
    let pid = tray.call("app.info", Value::Null).ok().and_then(|i| i["pid"].as_u64());
    let Some(pid) = pid.and_then(|pid| u32::try_from(pid).ok()) else { return 2 };
    if tray.call("app.quit", Value::Null).is_err() {
        return 2;
    }
    i32::from(!(process::wait_for_exit(pid, QUIT_WAIT_MS) || process::wait_for_exit(pid, SLOW_QUIT_WAIT_MS)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(tag: &str, draft: bool, pre: bool) -> Release {
        Release { tag_name: tag.into(), draft, prerelease: pre, assets: vec![] }
    }

    #[test]
    fn picks_the_right_release() {
        let cur = Version::parse("0.2.0").unwrap();
        let list = [
            rel("v0.3.0", true, false),        // draft: never
            rel("v0.2.2-beta.1", false, true), // prerelease
            rel("v0.2.1", false, false),
            rel("v0.1.9", false, false),
            rel("garbage", false, false),
        ];
        assert_eq!(pick(&list, &cur, false).unwrap().0.to_string(), "0.2.1");
        assert_eq!(pick(&list, &cur, true).unwrap().0.to_string(), "0.2.2-beta.1");
        assert!(pick(&[rel("v0.2.0", false, false)], &cur, false).is_none());
    }
}
