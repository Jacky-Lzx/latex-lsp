use std::process::Command;

#[test]
#[ignore = "requires Neovim 0.12+ and pdflatex"]
fn documented_neovim_configuration_refreshes_compiler_pull_diagnostics_and_applies_fixes() {
    let directory = tempfile::tempdir().unwrap();
    let script = directory.path().join("audit.lua");
    std::fs::write(&script, include_str!("fixtures/neovim_pull.lua")).unwrap();
    let config = directory.path().join("config.lua");
    let readme = include_str!("../README.md");
    let section = readme.split("### Neovim 0.11+").nth(1).unwrap();
    let lua = section
        .split("```lua\n")
        .nth(1)
        .unwrap()
        .split("```")
        .next()
        .unwrap();
    std::fs::write(&config, lua).unwrap();
    let output = Command::new("nvim")
        .args(["--headless", "-u", "NONE", "-i", "NONE", "-n", "-l"])
        .arg(script)
        .env("TEXQUILL_AUDIT_BIN", env!("CARGO_BIN_EXE_texquill"))
        .env("TEXQUILL_AUDIT_CONFIG", config)
        .env("XDG_CACHE_HOME", directory.path().join("cache"))
        .env("XDG_STATE_HOME", directory.path().join("state"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "Neovim diagnostic audit failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
