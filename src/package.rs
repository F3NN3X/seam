//! Whether this is the Microsoft Store copy (an MSIX package), and the few
//! things that differ if so. The same exes run in both channels.

use crate::autostart;
use crate::config::{Config, write_atomic};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use windows::ApplicationModel::Activation::ActivationKind;
use windows::ApplicationModel::{AppInstance, StartupTask, StartupTaskState};
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};
use windows::Win32::Storage::Packaging::Appx::{GetCurrentPackageFamilyName, GetCurrentPackageFullName};
use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use windows::core::{HSTRING, PWSTR};

/// `desktop:StartupTask` in the manifest.
const STARTUP_TASK: &str = "SeamStartup";
const FAMILY_NAME_MAX: usize = 65;

/// True when `GetCurrentPackageFullName` finds a package identity.
pub fn is_packaged() -> bool {
    static PACKAGED: OnceLock<bool> = OnceLock::new();
    *PACKAGED.get_or_init(|| {
        let mut len = 0u32;
        let found = unsafe { GetCurrentPackageFullName(&mut len, None) };
        found == ERROR_INSUFFICIENT_BUFFER || found == ERROR_SUCCESS
    })
}

fn family_name() -> Option<String> {
    let mut buf = [0u16; FAMILY_NAME_MAX + 1];
    let mut len = buf.len() as u32;
    let found = unsafe { GetCurrentPackageFamilyName(&mut len, Some(PWSTR(buf.as_mut_ptr()))) };
    (found == ERROR_SUCCESS).then(|| String::from_utf16_lossy(&buf[..(len as usize).saturating_sub(1)]))
}

/// Packaged apps write %APPDATA% into a private folder. This is where the
/// files really are, for the "Open" button and for reading them without the redirect.
pub fn settings_dir() -> PathBuf {
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    match (is_packaged().then(family_name).flatten(), local) {
        (Some(family), Some(local)) => {
            local.join("Packages").join(family).join("LocalCache").join("Roaming").join("Seam")
        }
        _ => Config::path().parent().map(Path::to_path_buf).unwrap_or_default(),
    }
}

/// Run on a thread of its own: WinRT needs an apartment, and a blocking
/// blocking wait must never sit on the tray's main thread.
fn on_fresh_thread<T: Send + 'static>(work: impl FnOnce() -> Option<T> + Send + 'static) -> Option<T> {
    std::thread::spawn(move || {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        work()
    })
    .join()
    .ok()
    .flatten()
}

/// What Windows says about the "Start with Windows" task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Startup {
    Off,
    On,
    /// Turned off in Windows Settings, Startup apps, so only the user can turn it on.
    DisabledByUser,
    DisabledByPolicy,
}

impl Startup {
    fn of(state: StartupTaskState) -> Startup {
        match state {
            StartupTaskState::Enabled | StartupTaskState::EnabledByPolicy => Startup::On,
            StartupTaskState::DisabledByUser => Startup::DisabledByUser,
            StartupTaskState::DisabledByPolicy => Startup::DisabledByPolicy,
            _ => Startup::Off,
        }
    }

    pub fn is_on(self) -> bool {
        self == Startup::On
    }

    /// The switch can't change it.
    pub fn locked(self) -> bool {
        matches!(self, Startup::DisabledByUser | Startup::DisabledByPolicy)
    }
}

fn startup_task() -> Option<StartupTask> {
    StartupTask::GetAsync(&HSTRING::from(STARTUP_TASK)).ok()?.join().ok()
}

/// Blocks. Not for the tray's main thread or the panel's UI thread.
pub fn startup_state() -> Option<Startup> {
    on_fresh_thread(|| Some(Startup::of(startup_task()?.State().ok()?)))
}

/// Blocks, like `startup_state`. Returns the state Windows ended up in.
pub fn set_startup(on: bool) -> Option<Startup> {
    on_fresh_thread(move || {
        let task = startup_task()?;
        let state = if on {
            task.RequestEnableAsync().ok()?.join().ok()?
        } else {
            task.Disable().ok()?;
            task.State().ok()?
        };
        Some(Startup::of(state))
    })
}

/// Whether Windows started this at sign-in, so only the tray starts and not
/// the window. A launch the user made opens the window as well. When Windows
/// won't say, stay quiet rather than open a window at every sign-in.
pub fn login_launch() -> bool {
    if std::env::args().any(|a| a == autostart::FLAG) {
        return true;
    }
    is_packaged() && !from_start_menu()
}

/// Only true when Windows says this wasn't the startup task.
fn from_start_menu() -> bool {
    let kind = || Some(AppInstance::GetActivatedEventArgs().ok()?.Kind().ok()? != ActivationKind::StartupTask);
    on_fresh_thread(kind).unwrap_or(false)
}

fn marker() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_default();
    base.join("Seam").join("installed-unpackaged")
}

/// The GitHub copy leaves its path where the Store copy can find it.
pub fn note_unpackaged_install() {
    let Ok(exe) = std::env::current_exe() else { return };
    if let Some(dir) = marker().parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(marker(), exe.display().to_string());
}

/// For the Store copy: is a GitHub copy still on this PC? Its Run key or its
/// marker names an exe, and the exe has to be there, so a deleted copy stops counting.
pub fn unpackaged_copy_installed() -> bool {
    let marked = std::fs::read_to_string(marker()).ok();
    [marked, autostart::exe()].into_iter().flatten().any(|exe| Path::new(exe.trim()).is_file())
}

fn earlier_config_path() -> PathBuf {
    Config::path().with_file_name("earlier-config.json")
}

fn offered_marker() -> PathBuf {
    Config::path().with_file_name("import-offered")
}

/// The first run of a Store copy has no config of its own yet.
pub fn first_run() -> bool {
    is_packaged() && !settings_dir().join("config.json").exists()
}

/// Keep a copy of the earlier GitHub install's settings, if Windows lets this
/// package read them, so the panel can offer them. Run before the first save.
pub fn stash_earlier_config() {
    if let Ok(text) = std::fs::read_to_string(Config::path()) {
        let _ = write_atomic(&earlier_config_path(), &text);
    }
}

/// The earlier settings, once, for the panel to offer.
pub fn take_import_offer() -> Option<Config> {
    if !is_packaged() || offered_marker().exists() {
        return None;
    }
    let bytes = std::fs::read(earlier_config_path()).ok()?;
    let _ = std::fs::write(offered_marker(), "");
    Config::from_import(&bytes).ok()
}
