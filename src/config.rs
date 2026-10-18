use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    PdfLatex,
    XeLatex,
    LuaLatex,
}

impl Engine {
    pub fn from_program(program: &str) -> Option<Self> {
        Some(match program.to_ascii_lowercase().as_str() {
            "pdflatex" => Self::PdfLatex,
            "xelatex" => Self::XeLatex,
            "lualatex" => Self::LuaLatex,
            _ => return None,
        })
    }

    fn latexmk_argument(self) -> &'static str {
        match self {
            Self::PdfLatex => "-pdf",
            Self::XeLatex => "-xelatex",
            Self::LuaLatex => "-lualatex",
        }
    }
}

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
    pub engine: Option<Engine>,
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
            engine: None,
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
    pub fn effective_engine(&self) -> Option<Engine> {
        if self.arguments.is_some() {
            return None;
        }
        let executable = std::path::Path::new(&self.executable)
            .file_stem()?
            .to_str()?;
        if executable == "latexmk" {
            Some(self.engine.unwrap_or(Engine::PdfLatex))
        } else {
            Engine::from_program(executable)
        }
    }

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
        if let Some(engine) = self.engine {
            if let Some(argument) = arguments.iter_mut().find(|argument| *argument == "-pdf") {
                *argument = engine.latexmk_argument().into();
            }
        }
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

    #[test]
    fn latexmk_engine_configuration_preserves_explicit_executables_and_arguments() {
        for (engine, flag) in [
            ("pdflatex", "-pdf"),
            ("xelatex", "-xelatex"),
            ("lualatex", "-lualatex"),
        ] {
            let config = Config::parse(json!({"build":{"engine":engine}})).unwrap();
            assert!(config.build.arguments().contains(&flag.into()));
            assert_eq!(
                serde_json::to_value(config.build.effective_engine()).unwrap(),
                json!(engine)
            );
        }
        let config =
            Config::parse(json!({"build":{"executable":"pdflatex","engine":"xelatex"}})).unwrap();
        assert_eq!(config.build.effective_engine(), Some(Engine::PdfLatex));
        assert!(!config.build.arguments().contains(&"-xelatex".into()));
        let config =
            Config::parse(json!({"build":{"engine":"xelatex","arguments":["custom", "%f"]}}))
                .unwrap();
        assert_eq!(config.build.arguments(), vec!["custom", "%f"]);
        assert_eq!(config.build.effective_engine(), None);
        assert!(Config::parse(json!({"build":{"engine":"bash"}})).is_err());
    }
}
