use directories::ProjectDirs;
use rust_embed::RustEmbed;

const PROJECT_ROOT: &str = env!("CARGO_MANIFEST_DIR");

pub fn asset_dir() -> std::path::PathBuf {
    let preferred_path = if cfg!(debug_assertions) {
        std::path::PathBuf::from(PROJECT_ROOT).join("../../dev_assets")
    } else if let Ok(path) = std::env::var("VIBEKANBAN_ASSET_DIR") {
        std::path::PathBuf::from(path)
    } else {
        prod_asset_dir_path()
    };

    if std::fs::create_dir_all(&preferred_path).is_ok() {
        return preferred_path;
    }

    // Fallback for containerized environments where the home/data directory
    // may be mounted read-only or owned by a different UID.
    let fallback_path = std::path::PathBuf::from("/tmp/vibe-kanban-assets");
    if std::fs::create_dir_all(&fallback_path).is_ok() {
        return fallback_path;
    }

    panic!(
        "Failed to create asset directory at '{}' and fallback '{}'",
        preferred_path.display(),
        fallback_path.display()
    );

    // ✔ macOS → ~/Library/Application Support/MyApp
    // ✔ Linux → ~/.local/share/myapp   (respects XDG_DATA_HOME)
    // ✔ Windows → %APPDATA%\Example\MyApp
}

pub fn prod_asset_dir_path() -> std::path::PathBuf {
    ProjectDirs::from("ai", "bloop", "vibe-kanban")
        .expect("OS didn't give us a home directory")
        .data_dir()
        .to_path_buf()
}

pub fn config_path() -> std::path::PathBuf {
    asset_dir().join("config.json")
}

pub fn profiles_path() -> std::path::PathBuf {
    asset_dir().join("profiles.json")
}

pub fn credentials_path() -> std::path::PathBuf {
    asset_dir().join("credentials.json")
}

pub fn trusted_keys_path() -> std::path::PathBuf {
    asset_dir().join("trusted_ed25519_public_keys.json")
}

pub fn server_signing_key_path() -> std::path::PathBuf {
    asset_dir().join("server_ed25519_signing_key")
}

#[derive(RustEmbed)]
#[folder = "../../assets/sounds"]
pub struct SoundAssets;

#[derive(RustEmbed)]
#[folder = "../../assets/scripts"]
pub struct ScriptAssets;
