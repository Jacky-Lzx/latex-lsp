use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub build: BuildConfig,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct BuildConfig {
    pub executable: String,
    pub arguments: Option<Vec<String>>,
    pub output_directory: String,
    pub root_file: Option<String>,
    pub on_save: bool,
    pub timeout_seconds: u64,
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            executable: "latexmk".into(),
            arguments: None,
            output_directory: "build/latex-lsp".into(),
            root_file: None,
            on_save: false,
            timeout_seconds: 120,
        }
    }
}

impl Config {
    pub fn parse(value: serde_json::Value) -> Result<Self, String> {
        let value = value.get("latex").unwrap_or(&value);
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("Invalid LaTeX configuration: {error}"))?;
        if config.build.executable.trim().is_empty()
            || config.build.output_directory.trim().is_empty()
        {
            return Err("Build executable and outputDirectory must not be empty".into());
        }
        if !(1..=3600).contains(&config.build.timeout_seconds) {
            return Err("Build timeoutSeconds must be between 1 and 3600".into());
        }
        Ok(config)
    }
}

impl BuildConfig {
    pub fn arguments(&self) -> Vec<String> {
        if let Some(arguments) = &self.arguments {
            return arguments.clone();
        }
        let engine = std::path::Path::new(&self.executable)
            .file_stem()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let arguments: &[&str] = match engine {
            "tectonic" => &[
                "-X",
                "compile",
                "--synctex",
                "--keep-logs",
                "--outdir",
                "%o",
                "%f",
            ],
            "pdflatex" | "xelatex" | "lualatex" => &[
                "-recorder",
                "-interaction=nonstopmode",
                "-file-line-error",
                "-synctex=1",
                "-no-shell-escape",
                "-output-directory=%o",
                "%f",
            ],
            _ => &[
                "-pdf",
                "-nobibfudge",
                "-interaction=nonstopmode",
                "-file-line-error",
                "-synctex=1",
                "-no-shell-escape",
                "-outdir=%o",
                "%f",
            ],
        };
        let mut arguments: Vec<String> = arguments
            .iter()
            .map(|argument| (*argument).into())
            .collect();
        // BibTeX's default file policy rejects hidden or parent-relative output
        // paths. Keep latexmk's directory workaround for explicitly configured
        // paths that need it; ordinary in-project paths can preserve source cwd.
        let output = std::path::Path::new(&self.output_directory);
        let needs_fudge = output.is_absolute() || output.components().any(|component| {
            matches!(component, std::path::Component::ParentDir)
                || matches!(component, std::path::Component::Normal(name) if name.to_str().is_some_and(|name| name.starts_with('.')))
        });
        if needs_fudge {
            for argument in &mut arguments {
                if argument == "-nobibfudge" {
                    *argument = "-bibfudge".into();
                }
            }
        }
        arguments
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_configuration_and_engine_defaults() {
        let config =
            Config::parse(json!({"latex":{"build":{"executable":"pdflatex","onSave":true}}}))
                .unwrap();
        assert!(config.build.on_save);
        assert!(config
            .build
            .arguments()
            .contains(&"-output-directory=%o".into()));
        assert!(!config.build.arguments().contains(&"-pdf".into()));
        assert!(Config::parse(json!({"build":{"timeoutSeconds":0}})).is_err());
        assert!(Config::parse(json!({"build":{"timeoutSecond":30}})).is_err());
        assert!(Config::parse(json!({"build":{"executable":""}})).is_err());
        assert!(BuildConfig::default()
            .arguments()
            .contains(&"-nobibfudge".into()));
        let config = BuildConfig {
            output_directory: ".latex-lsp".into(),
            ..Default::default()
        };
        assert!(config.arguments().contains(&"-bibfudge".into()));
        let config = BuildConfig {
            output_directory: "../output".into(),
            ..Default::default()
        };
        assert!(config.arguments().contains(&"-bibfudge".into()));
    }
}
