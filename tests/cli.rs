//! End-to-end checks that do not require the network.

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_harbor")
}

fn temp_env() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(bin())
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_STATE_HOME", home.join("state"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env_remove("HARBOR_CONFIG")
        .env_remove("HARBOR_BIN_DIR")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run harbor")
}

#[test]
fn help_and_version_are_available() {
    let temp = temp_env();
    let help = run(temp.path(), &["--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("Harbor discovers release binaries"));
    let version = run(temp.path(), &["--version"]);
    assert!(version.status.success());
    assert!(String::from_utf8_lossy(&version.stdout).contains("harbor 0.1.0"));
}

#[test]
fn interactive_ui_requires_a_terminal() {
    let temp = temp_env();
    let output = run(temp.path(), &["tui"]);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("terminal"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn install_and_update_require_yes_without_a_terminal() {
    let temp = temp_env();
    let install = run(temp.path(), &["install", "summertools/dulens"]);
    assert!(!install.status.success());
    assert!(
        String::from_utf8_lossy(&install.stderr).contains("--yes"),
        "stderr: {}",
        String::from_utf8_lossy(&install.stderr)
    );
    let update = run(temp.path(), &["update"]);
    assert!(!update.status.success());
    assert!(
        String::from_utf8_lossy(&update.stderr).contains("--yes"),
        "stderr: {}",
        String::from_utf8_lossy(&update.stderr)
    );
}

#[test]
fn add_list_and_remove_sources() {
    let temp = temp_env();
    let config = temp.path().join("harbor.toml");
    let config_arg = config.to_str().unwrap();

    let added = run(
        temp.path(),
        &[
            "source",
            "add",
            "repo",
            "BurntSushi/ripgrep",
            "--config",
            config_arg,
        ],
    );
    assert!(
        added.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&added.stderr)
    );
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.contains("BurntSushi/ripgrep"));
    assert!(text.contains("summertools"));

    let listed = run(
        temp.path(),
        &["source", "list", "--config", config_arg, "--json"],
    );
    assert!(listed.status.success());
    let json: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 2);
    assert!(json.to_string().contains("BurntSushi/ripgrep"));

    let duplicate = run(
        temp.path(),
        &[
            "source",
            "add",
            "org",
            "summertools",
            "--config",
            config_arg,
        ],
    );
    assert!(duplicate.status.success());
    assert!(String::from_utf8_lossy(&duplicate.stdout).contains("already configured"));

    let removed = run(
        temp.path(),
        &[
            "source",
            "remove",
            "BurntSushi/ripgrep",
            "--config",
            config_arg,
        ],
    );
    assert!(removed.status.success());
    let text = fs::read_to_string(&config).unwrap();
    assert!(!text.contains("BurntSushi/ripgrep"));
}

#[test]
fn invalid_sources_are_rejected_without_writing_config() {
    let temp = temp_env();
    let config = temp.path().join("harbor.toml");
    let config_arg = config.to_str().unwrap();
    let output = run(
        temp.path(),
        &[
            "source",
            "add",
            "repo",
            "owner/repo;rm",
            "--config",
            config_arg,
        ],
    );
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("invalid repository name")
            || String::from_utf8_lossy(&output.stderr).contains("invalid source")
    );
    assert!(!config.exists());
}

#[test]
fn config_show_and_path_use_the_selected_file() {
    let temp = temp_env();
    let config = temp.path().join("custom.toml");
    fs::write(
        &config,
        "[[sources]]\ntype = \"repo\"\nname = \"owner/tool\"\n",
    )
    .unwrap();
    let shown = run(
        temp.path(),
        &["config", "show", "--config", config.to_str().unwrap()],
    );
    assert!(shown.status.success());
    let stdout = String::from_utf8_lossy(&shown.stdout);
    assert!(stdout.contains("owner/tool"));
    let path = run(
        temp.path(),
        &["config", "path", "--config", config.to_str().unwrap()],
    );
    assert!(path.status.success());
    assert_eq!(
        String::from_utf8_lossy(&path.stdout).trim(),
        config.display().to_string()
    );
}

#[test]
fn relative_config_paths_are_supported() {
    let temp = temp_env();
    let output = Command::new(bin())
        .current_dir(temp.path())
        .args([
            "source",
            "add",
            "repo",
            "owner/tool",
            "--config",
            "harbor.toml",
        ])
        .env("HOME", temp.path())
        .env("XDG_CONFIG_HOME", temp.path().join("config"))
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("run harbor");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(temp.path().join("harbor.toml").exists());
}

#[test]
fn source_add_accepts_bare_and_colon_forms() {
    let temp = temp_env();
    let config = temp.path().join("harbor.toml");
    let config_arg = config.to_str().unwrap();
    let bare = run(
        temp.path(),
        &["source", "add", "owner/tool", "--config", config_arg],
    );
    assert!(bare.status.success());
    let colon = run(
        temp.path(),
        &["source", "add", "repo:other/tool2", "--config", config_arg],
    );
    assert!(colon.status.success());
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.contains("owner/tool"));
    assert!(text.contains("other/tool2"));
}
